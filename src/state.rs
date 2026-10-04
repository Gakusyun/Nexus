//! Application state: the task list, the filter, the engine connection, and the polling
//! loop that keeps all of it in sync with aria2.
//!
//! This entity is the root view. Every mutation goes through a method here so that
//! persistence and `cx.notify()` stay in one place.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Context, Entity, Font, FontFallbacks, PathPromptOptions, Subscription, Window,
};
use serde::{Deserialize, Serialize};

use crate::aria2::{Aria2, DownloadRequest, EngineOptions, Snapshot};
use crate::i18n::{self, Strings};
use crate::model::{Status, Task, fmt_size, total_speed};
use crate::settings::{Language, Settings, ThemeMode, font_stack};
use crate::store::{Event, Store};
// The stored `ThemeMode` is this app's — it has a serde shape for the legacy importer. The
// library's has the same three names and no persistence, which is the right split: how a
// preference is spelled in a database file is not a design-language decision.
use nexus_look::ThemeMode as LookMode;
use nexus_look::{Look, TextInput};

/// How often we ask aria2 for fresh numbers.
const POLL: Duration = Duration::from_millis(500);
/// A hiccup must repeat this many times before the UI admits the engine is gone.
const FAILURE_TOLERANCE: u32 = 5;
/// How often progress *numbers* are written to the database. Status changes are written the
/// moment they happen; the byte counters move twice a second while downloading, and there is
/// no reason to make the disk watch that.
const PROGRESS_FLUSH: Duration = Duration::from_secs(3);
/// The SQLite file, next to the app's other data.
const DATABASE: &str = "nexus.db";

/// How hard [`discard_download`] tries before it gives up. The budget has to outlast two things:
/// Windows closing the handle after aria2 is told to forget a live download, and the antivirus
/// scan that follows a freshly written large file — Defender holding a 1.5 GB image is the usual
/// reason a delete looks like it did nothing. A file that is really locked stays locked, so the
/// wait is bounded and the failure is reported.
const DELETE_TRIES: u32 = 6;
const DELETE_RETRY: Duration = Duration::from_millis(250);

/// The editable boxes the add-download dialog owns.
///
/// `AddField` is an index rather than a name for a buffer: the dialog owns one library input per
/// variant (see [`AddDialog`]), so adding a box means adding a variant and an entry here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddField {
    Uri,
    Dir,
    Name,
    UserAgent,
    Proxy,
    Referer,
}

impl AddField {
    pub const ALL: [AddField; 6] = [
        AddField::Uri,
        AddField::Dir,
        AddField::Name,
        AddField::UserAgent,
        AddField::Proxy,
        AddField::Referer,
    ];
}

/// The open add-download dialog.
///
/// It owns the per-download choices for one task. The boxes are the library's `TextInput`s,
/// indexed by [`AddField`], so a field is added by extending the enum and the array — the widget
/// brings its own buffer, focus handle, caret and IME bridge, and the dialog only has to say where
/// each box lives. The whole dialog is dropped when it closes, boxes and all.
pub struct AddDialog {
    boxes: [Entity<TextInput>; 6],
    pub connections: u32,
    /// This download's `max-download-limit`; `0` is unlimited.
    pub speed_limit: u64,
}

impl AddDialog {
    fn new(strings: &Strings, cx: &mut Context<NexusApp>) -> Self {
        let boxes = std::array::from_fn(|index| {
            let which = AddField::ALL[index];
            let placeholder = placeholder_for(which, strings);
            // Enter queues from the URL line and nowhere else: the other boxes are steps on the
            // way to it, and a half-filled form must not start a download because the caret was
            // sitting in the wrong place.
            let submit = (which == AddField::Uri).then(|| {
                cx.listener(|this, _text: &str, window, cx| this.start_add_download(window, cx))
            });
            let dismiss =
                NexusApp::dismissal(cx, |this, window, cx| this.close_add_dialog(window, cx));
            cx.new(move |cx| {
                let field = TextInput::new(cx, placeholder);
                let field = if let Some(submit) = submit {
                    field.on_submit(submit)
                } else {
                    field
                };
                field.on_dismiss(dismiss)
            })
        });
        Self {
            boxes,
            connections: 0,
            speed_limit: 0,
        }
    }

    /// The box itself: a view paints it as a child, exactly as it paints one in the settings sheet.
    pub fn input(&self, which: AddField) -> &Entity<TextInput> {
        &self.boxes[which as usize]
    }

    /// What is typed in a box, for the code that turns the dialog into a request.
    pub fn text(&self, which: AddField, cx: &App) -> String {
        self.boxes[which as usize].read(cx).text().to_string()
    }

    /// Fill a box — opening the dialog, or the folder picker answering with a path.
    fn set(&mut self, which: AddField, text: impl Into<String>, cx: &mut Context<NexusApp>) {
        let text = text.into();
        self.boxes[which as usize].update(cx, |field, cx| field.set_text(text, cx));
    }
}

/// The placeholder a dialog box starts with. A localized app can switch language with the dialog
/// closed, so this is a function of the language rather than something stored on the box.
fn placeholder_for(which: AddField, strings: &Strings) -> &'static str {
    match which {
        AddField::Uri => strings.placeholder,
        AddField::Dir => strings.dir_placeholder,
        AddField::Name => strings.file_name_placeholder,
        AddField::UserAgent => strings.user_agent_placeholder,
        AddField::Proxy => strings.proxy_placeholder,
        AddField::Referer => strings.referer_placeholder,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    All,
    Downloading,
    Completed,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::All, Filter::Downloading, Filter::Completed];

    pub fn label(self, strings: &Strings) -> &'static str {
        match self {
            Filter::All => strings.filter_all,
            Filter::Downloading => strings.filter_active,
            Filter::Completed => strings.filter_finished,
        }
    }

    fn matches(self, status: Status) -> bool {
        match self {
            Filter::All => true,
            // "Active" means work actually in flight. A paused download is not active — it
            // would otherwise be counted here while its own badge reads "Paused".
            // Paused and failed rows stay reachable under "All".
            Filter::Downloading => matches!(status, Status::Active | Status::Waiting),
            // Terminal states, so this count matches exactly what "Clear finished" removes.
            Filter::Completed => status.is_terminal(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Engine {
    Connecting,
    Online,
    Failed(String),
}

/// A destructive action parked until the user says what should happen to the bytes on disk.
///
/// The dialog only exists when there is actually something to delete, so "remove a task whose
/// file was never created" stays a single click.
pub struct Confirm {
    pub heading: String,
    /// What will be removed, on a line of its own. Keeping it separate is what stops a long name
    /// from shoving the path into an arbitrary break — the reason this used to be one joined line.
    pub subject: String,
    /// How many bytes are at stake, spelled out. Sits across from `subject` rather than in the
    /// list below: it is the other half of the delete/keep decision, and as one more left-aligned
    /// line it read as just another detail.
    pub size: String,
    /// The facts under it — location, file count — one line each so each gets the whole card.
    pub facts: Vec<String>,
    /// Rows to drop once the user decides.
    seqs: Vec<u64>,
}

pub struct NexusApp {
    pub tasks: Vec<Task>,
    pub engine: Engine,
    pub filter: Filter,
    pub download_dir: PathBuf,
    /// The command bar's URL field. An entity of its own — the library's input owns its buffer,
    /// its focus handle, its caret and its IME bridge, so there is nothing left for the app to
    /// hold on its behalf.
    pub input: Entity<TextInput>,
    pub notice: Option<String>,
    /// Mirrors the platform window state, refreshed by `observe_window_bounds`, so the
    /// title bar can swap maximise for restore.
    pub maximized: bool,
    /// Whether the settings page is showing.
    pub settings_open: bool,
    /// Set while a removal is waiting to be confirmed.
    pub confirm: Option<Confirm>,
    /// Set while the add-download dialog is open.
    pub add_dialog: Option<AddDialog>,
    pub settings: Settings,
    /// What the settings sheet is editing. `Some` only while it is open: the widgets read and
    /// write this copy, so nothing reaches disk until Save and Cancel just drops it. See
    /// [`NexusApp::active_settings`].
    draft: Option<Settings>,
    /// What is typed in the font box, exactly as typed. Kept apart from `settings.font` so
    /// the trimming that goes into the database cannot move the caret while the user types.
    pub font_input: Entity<TextInput>,
    /// Whether the font picker's panel is showing.
    pub font_menu_open: bool,
    /// The picker's filter box.
    pub font_search: Entity<TextInput>,
    /// What is typed in the engine's user-agent box, kept apart from `settings.user_agent` for
    /// the same reason as `font_input`: trimming must not move the caret while typing.
    pub ua_input: Entity<TextInput>,
    /// Likewise for the proxy box.
    pub proxy_input: Entity<TextInput>,
    /// The font list, read from the platform once because the settings page needs it on
    /// every frame while it is open.
    pub fonts: Vec<String>,
    /// The history database. Always present; it may be disabled if it could not be opened.
    pub store: Store,
    /// Last state written to the database, per task, so a flush only touches rows that
    /// actually moved and the event log only records real transitions.
    seen: HashMap<u64, (Status, u64)>,
    last_flush: Instant,
    aria2: Option<Arc<Aria2>>,
    /// Set when the persisted state no longer matches memory.
    dirty: bool,
    seq: u64,
    failures: u32,
    /// Kept alive so the bounds callback stays registered for the view's lifetime.
    _bounds: Option<Subscription>,
    /// Likewise for the light/dark callback.
    _appearance: Option<Subscription>,
}

impl NexusApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = Store::open(data_dir().join(DATABASE));

        // Preferences are one row per key. A database written before that change keeps them
        // in a single JSON blob; fold it in once.
        Settings::absorb_legacy(&store);

        // On the very first run after the switch to SQLite the old `state.json` is imported
        // once, then renamed so it cannot be imported twice. It only wins while the database
        // holds no preferences of its own.
        let legacy = (!store.has_settings()).then(Persisted::load);
        let settings = match &legacy {
            Some(saved) => saved.settings.clone(),
            None => Settings::load(&store),
        };
        let download_dir = settings
            .download_dir
            .as_ref()
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                legacy
                    .as_ref()
                    .and_then(|saved| saved.download_dir.clone())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(default_download_dir);
        // Keep the settings row authoritative: the folder used to live in a sibling field of
        // the old JSON file, and a migrated row would otherwise read back as "not set" and
        // silently fall back to the default on the next launch.
        let mut settings = settings;
        settings.download_dir = Some(download_dir.to_string_lossy().into_owned());

        // Anything still "in flight" when the app last closed is really just a leftover
        // record: aria2 restarts with the app, so those gids are gone. Showing the rows
        // as paused — rather than pretending they are running — is honest, and pressing
        // resume re-attaches them through aria2's own resume support.
        let stored = match &legacy {
            Some(saved) => saved.tasks.clone(),
            None => store.load_downloads(),
        };
        let tasks: Vec<Task> = stored
            .into_iter()
            .map(|mut task| {
                task.status = if task.status.is_terminal() {
                    task.status
                } else {
                    Status::Paused
                };
                task.speed = 0;
                task
            })
            .collect();

        // Hidden rows still own their number, so the list cannot be the only source for this.
        let seq = tasks
            .iter()
            .map(|task| task.seq)
            .max()
            .unwrap_or(0)
            .max(store.highest_seq())
            + 1;
        let maximized = window.is_maximized();
        // Sorted once: the picker lists them in a stable order and nothing here cares about the
        // platform's.
        let mut fonts = cx.text_system().all_font_names();
        fonts.sort_by_key(|family| family.to_lowercase());
        // Every box in the app is one of the library's inputs: it owns its buffer, its focus
        // handle, its caret and its IME bridge, so the app keeps only the entity and the rule for
        // what Enter and Escape mean here.
        let strings = i18n::Strings::get(settings.language);
        let submit_input = cx.listener(|this, _text: &str, _, cx| this.submit(cx));
        let dismiss_input = Self::dismissal(cx, |this, _, cx| {
            // Escape at the front door clears the notice, and nothing else.
            this.notice = None;
            cx.notify();
        });
        let input_placeholder = strings.placeholder;
        let input = cx.new(move |cx| {
            TextInput::new(cx, input_placeholder)
                .large()
                .on_submit(submit_input)
                .on_dismiss(dismiss_input)
        });
        let font_input = Self::settings_field(cx, strings.font_placeholder, |this, text, _, cx| {
            this.font_text_changed(text, cx)
        });
        // The list of matches is drawn by the sheet, so filtering it needs the sheet to redraw;
        // the box itself only repaints its own caret.
        let font_search =
            Self::settings_field(cx, strings.font_search_placeholder, |_, _, _, cx| {
                cx.notify()
            });
        let ua_input =
            Self::settings_field(cx, strings.user_agent_placeholder, |this, text, _, cx| {
                this.ua_text_changed(text, cx)
            });
        let proxy_input =
            Self::settings_field(cx, strings.proxy_placeholder, |this, text, _, cx| {
                this.proxy_text_changed(text, cx)
            });
        let font = settings.font.clone().unwrap_or_default();
        let user_agent = settings.user_agent.clone().unwrap_or_default();
        let proxy = settings.proxy.clone().unwrap_or_default();
        font_input.update(cx, |field, cx| field.set_text(font, cx));
        ua_input.update(cx, |field, cx| field.set_text(user_agent, cx));
        proxy_input.update(cx, |field, cx| field.set_text(proxy, cx));
        // Apply the stored palette before the first frame paints.
        Look::update(cx, |look| look.mode = look_mode(settings.theme));

        // Remember what the database already knows, so the first flush neither rewrites
        // unchanged rows nor logs events for downloads that were recorded last session.
        let seen = tasks
            .iter()
            .map(|task| (task.seq, (task.status, task.completed)))
            .collect();

        let mut app = Self {
            tasks,
            engine: Engine::Connecting,
            filter: Filter::All,
            download_dir,
            input,
            notice: None,
            maximized,
            settings_open: false,
            confirm: None,
            add_dialog: None,
            settings,
            draft: None,
            font_input,
            font_menu_open: false,
            font_search,
            ua_input,
            proxy_input,
            fonts,
            store,
            seen,
            last_flush: Instant::now(),
            aria2: None,
            dirty: false,
            seq,
            failures: 0,
            _bounds: None,
            _appearance: None,
        };

        if legacy.is_some() {
            app.import_legacy();
        } else {
            // Keep the stored rows in step with what we just resolved, so a value derived
            // rather than read (the download folder, once) is written down instead of being
            // re-derived every launch.
            app.save_settings();
        }

        // Maximising changes the window bounds and nothing else we care about, so without
        // this the restore glyph would only appear on the next unrelated repaint.
        app._bounds = Some(cx.observe_window_bounds(window, |this, window, cx| {
            let maximized = window.is_maximized();
            if this.maximized != maximized {
                this.maximized = maximized;
                cx.notify();
            }
        }));

        // Only matters while the theme is set to follow the system, but the check is cheap
        // and the subscription has to exist before the user switches to it.
        app._appearance = Some(cx.observe_window_appearance(window, |this, _window, cx| {
            if this.active_settings().theme == ThemeMode::System {
                Look::update(cx, |look| look.mode = LookMode::System);
                cx.notify();
            }
        }));

        // Pasting a link is the whole point of the app, so start with the cursor already
        // in the command bar. Deferred to the next frame because the field does not
        // exist as a focusable element until it has been rendered once.
        cx.on_next_frame(window, |this, window, cx| {
            let handle = this.input.read(cx).focus_handle().clone();
            window.focus(&handle, cx);
        });

        app.boot(cx);
        app
    }

    /// Start the engine, then poll it forever. Both loops end with the view.
    fn boot(&mut self, cx: &mut Context<Self>) {
        let dir = self.download_dir.clone();
        let options = self.engine_options();
        cx.spawn(async move |this, cx| {
            let started = cx
                .background_spawn(async move { Aria2::start(&dir, &options) })
                .await;
            this.update(cx, |this, cx| {
                match started {
                    Ok(engine) => {
                        this.aria2 = Some(Arc::new(engine));
                        this.engine = Engine::Online;
                        this.store.log(Event::EngineReady, None, None, None);
                    }
                    Err(err) => {
                        this.engine = Engine::Failed(format!("{err:#}"));
                        this.store
                            .log(Event::EngineFailed, None, None, Some(&format!("{err:#}")));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;

                let engine = this.update(cx, |this, _| this.aria2.clone()).ok().flatten();
                let snapshot = match engine {
                    Some(engine) => {
                        Some(cx.background_spawn(async move { engine.snapshot() }).await)
                    }
                    None => None,
                };
                let alive = this
                    .update(cx, |this, cx| {
                        match snapshot {
                            Some(Ok(snapshot)) => {
                                this.failures = 0;
                                this.engine = Engine::Online;
                                this.apply(snapshot);
                            }
                            Some(Err(err)) => {
                                this.failures += 1;
                                if this.failures >= FAILURE_TOLERANCE {
                                    this.engine = Engine::Failed(format!("{err:#}"));
                                }
                            }
                            None => {}
                        }

                        if this.dirty {
                            this.save_settings();
                            this.dirty = false;
                        }
                        this.flush();
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    // ---------------------------------------------------------------- derived data

    /// Indices into `tasks`, in display order for the active filter: running work first
    /// (downloading, then queued, then paused), newest finished work after that.
    pub fn visible(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.tasks.len())
            .filter(|&i| self.filter.matches(self.tasks[i].status))
            .collect();
        indices.sort_by_key(|&i| {
            let task = &self.tasks[i];
            match task.status.is_terminal() {
                true => (1u8, 0u8, u64::MAX - task.seq),
                false => (0u8, task.status.rank(), task.seq),
            }
        });
        indices
    }

    /// Bytes per second across the rows that are moving, summed on demand.
    ///
    /// Never stored: the aggregate used to be a field that only a successful poll refreshed, so
    /// pausing left its last speed on screen until the next tick came back. See
    /// [`total_speed`](crate::model::total_speed) for why the engine's own total is not an
    /// option either.
    pub fn speed(&self) -> u64 {
        total_speed(&self.tasks)
    }

    pub fn count(&self, filter: Filter) -> usize {
        self.tasks
            .iter()
            .filter(|task| filter.matches(task.status))
            .count()
    }

    pub fn index_of(&self, seq: u64) -> Option<usize> {
        self.tasks.iter().position(|task| task.seq == seq)
    }

    fn next_seq(&mut self) -> u64 {
        let seq = self.seq;
        self.seq += 1;
        seq
    }

    // ---------------------------------------------------------------- user actions

    pub fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        if self.filter != filter {
            self.filter = filter;
            cx.notify();
        }
    }

    /// Queue whatever is in the command bar: the quick path, with no per-task overrides.
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        let uri = self.input.read(cx).text().trim().to_string();
        if uri.is_empty() {
            return;
        }
        if !looks_like_uri(&uri) {
            self.warn(self.strings().bad_uri, cx);
            return;
        }
        if !self.engine_ready(cx) {
            return;
        }
        let request = self.quick_request(uri, self.download_dir.clone());
        self.input.update(cx, |input, cx| input.clear(cx));
        self.enqueue(request, None, cx);
    }

    // ---------------------------------------------------------------- add-download dialog

    /// Open the advanced dialog, pre-filled from the quick bar and the current engine settings.
    pub fn open_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.add_dialog.is_some() {
            return;
        }
        let strings = self.strings();
        let mut dialog = AddDialog::new(strings, cx);
        dialog.connections = self.settings.connections.clamp(1, 16);
        // Read, then write: the box's own entity cannot be read and updated in one step, so each
        // pre-fill gets its own statement.
        let uri = self.input.read(cx).text().to_string();
        let dir = self.download_dir.to_string_lossy().into_owned();
        let user_agent = self.settings.user_agent.clone().unwrap_or_default();
        let proxy = self.settings.proxy.clone().unwrap_or_default();
        dialog.set(AddField::Uri, uri, cx);
        dialog.set(AddField::Dir, dir, cx);
        dialog.set(AddField::UserAgent, user_agent, cx);
        dialog.set(AddField::Proxy, proxy, cx);
        let focus = dialog.input(AddField::Uri).read(cx).focus_handle().clone();
        self.add_dialog = Some(dialog);
        window.focus(&focus, cx);
        cx.notify();
    }

    pub fn close_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.add_dialog.is_some() {
            // Move focus out *before* the dialog (and its focus handles) is dropped, so the window
            // is never left pointing at a handle that no longer exists.
            let handle = self.input.read(cx).focus_handle().clone();
            window.focus(&handle, cx);
            self.add_dialog = None;
            cx.notify();
        }
    }

    /// Queue the dialog's download and close it.
    pub fn start_add_download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Take the text out first so the immutable borrow of the dialog ends before the mutable
        // calls below.
        let (uri, typed_dir) = {
            let Some(dialog) = self.add_dialog.as_ref() else {
                return;
            };
            (
                dialog.text(AddField::Uri, cx).trim().to_string(),
                dialog.text(AddField::Dir, cx).trim().to_string(),
            )
        };
        if uri.is_empty() {
            return;
        }
        if !looks_like_uri(&uri) {
            self.warn(self.strings().bad_uri, cx);
            return;
        }
        if !self.engine_ready(cx) {
            return;
        }
        let dir = if typed_dir.is_empty() {
            self.download_dir.clone()
        } else {
            PathBuf::from(typed_dir)
        };
        let request = self.dialog_request(cx, &uri, dir);
        let name = request.file_name.clone();
        // Back to the quick bar, ready for the next paste.
        let handle = self.input.read(cx).focus_handle().clone();
        window.focus(&handle, cx);
        self.add_dialog = None;
        self.enqueue(request, name, cx);
    }

    /// Pick a folder for the open dialog's "save to" box.
    pub fn choose_add_dir(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(self.strings().choose_folder.into()),
        });

        cx.spawn(async move |this, cx| {
            let picked = match receiver.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) | Err(_) => None,
                Ok(Err(err)) => {
                    this.update(cx, |this, cx| {
                        let message = format!("{}: {err}", this.strings().picker_failed);
                        this.warn(message, cx);
                    })
                    .ok();
                    return;
                }
            };
            if let Some(dir) = picked {
                this.update(cx, |this, cx| {
                    if let Some(dialog) = this.add_dialog.as_mut() {
                        dialog.set(AddField::Dir, dir.to_string_lossy().into_owned(), cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    pub fn set_add_connections(&mut self, value: u32, cx: &mut Context<Self>) {
        if let Some(dialog) = self.add_dialog.as_mut() {
            dialog.connections = value;
            cx.notify();
        }
    }

    pub fn set_add_speed_limit(&mut self, value: u64, cx: &mut Context<Self>) {
        if let Some(dialog) = self.add_dialog.as_mut() {
            dialog.speed_limit = value;
            cx.notify();
        }
    }

    /// Whether a download can be queued at all; warns and returns `false` otherwise.
    fn engine_ready(&mut self, cx: &mut Context<Self>) -> bool {
        if self.aria2.is_some() {
            return true;
        }
        let strings = self.strings();
        let message = match &self.engine {
            Engine::Failed(reason) => format!("{}: {reason}", strings.engine_unavailable),
            _ => strings.engine_starting.to_string(),
        };
        self.warn(message, cx);
        false
    }

    /// A download from the quick bar: the global engine settings, no per-task overrides.
    fn quick_request(&self, uri: String, dir: PathBuf) -> DownloadRequest {
        let engine = self.engine_options();
        DownloadRequest {
            uri,
            dir: dir.to_string_lossy().into_owned(),
            file_name: None,
            user_agent: engine.user_agent,
            connections: engine.connections,
            proxy: engine.proxy,
            max_tries: engine.max_tries,
            timeout: engine.timeout,
            speed_limit: 0,
            referer: None,
        }
    }

    /// A download from the dialog: the per-task overrides the user chose.
    fn dialog_request(&self, cx: &App, uri: &str, dir: PathBuf) -> DownloadRequest {
        let Some(dialog) = self.add_dialog.as_ref() else {
            return self.quick_request(uri.to_string(), dir);
        };
        // Retries and timeout stay global; the dialog only overrides what is genuinely per-task.
        let engine = self.engine_options();
        let name = dialog.text(AddField::Name, cx);
        let user_agent = dialog.text(AddField::UserAgent, cx);
        let proxy = dialog.text(AddField::Proxy, cx);
        let referer = dialog.text(AddField::Referer, cx);
        DownloadRequest {
            uri: uri.to_string(),
            dir: dir.to_string_lossy().into_owned(),
            file_name: non_empty(&name),
            user_agent: non_empty(&user_agent),
            connections: dialog.connections.clamp(1, 16),
            proxy: non_empty(&proxy),
            max_tries: engine.max_tries,
            timeout: engine.timeout,
            speed_limit: dialog.speed_limit,
            referer: non_empty(&referer),
        }
    }

    /// Create the row, record it, and hand the request to the engine. Shared by the quick bar and
    /// the add-download dialog so the bookkeeping only exists once.
    fn enqueue(&mut self, request: DownloadRequest, name: Option<String>, cx: &mut Context<Self>) {
        let seq = self.next_seq();
        let uri = request.uri.clone();
        let mut task = Task::placeholder(seq, uri.clone(), request.dir.clone());
        if let Some(name) = name {
            task.name = name;
        }
        // Record it before the engine even acknowledges the request: if aria2 rejects the URI the
        // row is removed again, and the attempt is still in the log.
        self.store.upsert_download(&task, task.created_at);
        self.store
            .log(Event::Added, Some(seq), Some(&task.name), Some(&uri));
        self.seen.insert(seq, (task.status, task.completed));
        self.tasks.push(task);
        self.dirty = true;
        cx.notify();

        self.dispatch(
            cx,
            move |aria2| aria2.add_uri(&request),
            move |this, result, cx| {
                match result {
                    Ok(gid) => {
                        if let Some(task) = this.tasks.iter_mut().find(|task| task.seq == seq) {
                            task.gid = Some(gid);
                        }
                        this.dirty = true;
                    }
                    Err(err) => {
                        this.tasks.retain(|task| task.seq != seq);
                        this.seen.remove(&seq);
                        // The row was written before the engine answered so the attempt would
                        // show up in the log; hide it now that it is known to have gone nowhere.
                        this.store.mark_deleted(seq);
                        this.store
                            .log(Event::Failed, Some(seq), None, Some(&format!("{err:#}")));
                        let message = format!("{}: {err:#}", this.strings().add_failed);
                        this.warn(message, cx);
                    }
                }
                cx.notify();
            },
        );
    }

    /// Pause a running download, or resume a paused one.
    pub fn toggle(&mut self, seq: u64, cx: &mut Context<Self>) {
        match self.index_of(seq).map(|index| self.tasks[index].status) {
            Some(Status::Active | Status::Waiting) => self.pause(seq, cx),
            Some(Status::Paused) => self.resume(seq, cx),
            Some(Status::Error) => self.retry(seq, cx),
            _ => {}
        }
    }

    fn pause(&mut self, seq: u64, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(seq) else {
            return;
        };
        let Some(gid) = self.tasks[index].gid.clone() else {
            return;
        };
        // Reflect the intent immediately; the next poll confirms it.
        self.tasks[index].status = Status::Paused;
        self.tasks[index].speed = 0;
        self.dirty = true;
        cx.notify();

        self.dispatch(
            cx,
            move |aria2| aria2.pause(&gid),
            move |this, result, cx| {
                if let Err(err) = result {
                    this.warn(format!("{}: {err:#}", this.strings().pause_failed), cx);
                }
                cx.notify();
            },
        );
    }

    fn resume(&mut self, seq: u64, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(seq) else {
            return;
        };
        // With no gid aria2 has never seen this task — it is fresh, or it was restored
        // after a restart. Re-queueing the URI lets `--continue` pick up the partial
        // file where it left off.
        let Some(gid) = self.tasks[index].gid.clone() else {
            return self.requeue(seq, cx);
        };

        self.tasks[index].status = Status::Waiting;
        self.dirty = true;
        cx.notify();

        self.dispatch(
            cx,
            move |aria2| aria2.unpause(&gid),
            move |this, result, cx| {
                if result.is_err() {
                    // The gid went stale; fall back to re-queueing.
                    this.requeue(seq, cx);
                }
                cx.notify();
            },
        );
    }

    fn retry(&mut self, seq: u64, cx: &mut Context<Self>) {
        if let Some(index) = self.index_of(seq) {
            self.tasks[index].gid = None;
        }
        self.requeue(seq, cx);
    }

    /// Hand a task's URI back to aria2 and adopt the new gid.
    fn requeue(&mut self, seq: u64, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(seq) else {
            return;
        };
        let Some(uri) = self.tasks[index].primary_uri().map(str::to_string) else {
            self.warn(self.strings().bad_uri, cx);
            return;
        };
        let dir = self.download_dir.clone();
        let request = self.quick_request(uri, dir);

        self.tasks[index].gid = None;
        self.tasks[index].status = Status::Waiting;
        self.tasks[index].error = None;
        self.dirty = true;
        cx.notify();

        self.dispatch(
            cx,
            move |aria2| aria2.add_uri(&request),
            move |this, result, cx| {
                let Some(index) = this.index_of(seq) else {
                    return;
                };
                match result {
                    Ok(gid) => this.tasks[index].gid = Some(gid),
                    Err(err) => {
                        this.tasks[index].status = Status::Error;
                        this.tasks[index].error = Some(format!("{err:#}"));
                    }
                }
                this.dirty = true;
                cx.notify();
            },
        );
    }

    /// Ask before dropping a row, but only when there is a file to ask about.
    pub fn request_remove(&mut self, seq: u64, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(seq) else {
            return;
        };
        let path = self.tasks[index].path();
        if !path.exists() {
            // Nothing was ever written (metadata never arrived, or the file was moved
            // away by hand), so there is no question worth asking.
            self.remove_now(vec![seq], false, cx);
            return;
        }

        let task = &self.tasks[index];
        let size = on_disk_size(&path).map(fmt_size).unwrap_or_default();
        self.confirm = Some(Confirm {
            heading: self.strings().remove_one.to_string(),
            subject: task.name.clone(),
            size,
            facts: vec![task.dir.clone()],
            seqs: vec![seq],
        });
        cx.notify();
    }

    /// The same question, for the whole finished list at once.
    pub fn request_clear_finished(&mut self, cx: &mut Context<Self>) {
        let seqs: Vec<u64> = self
            .tasks
            .iter()
            .filter(|task| task.status.is_terminal())
            .map(|task| task.seq)
            .collect();
        if seqs.is_empty() {
            return;
        }

        let on_disk: Vec<(String, u64)> = self
            .tasks
            .iter()
            .filter(|task| task.status.is_terminal())
            .filter_map(|task| {
                let path = task.path();
                has_download_data(&path)
                    .then(|| (task.name.clone(), on_disk_size(&path).unwrap_or(0)))
            })
            .collect();

        if on_disk.is_empty() {
            self.remove_now(seqs, false, cx);
            return;
        }

        let bytes: u64 = on_disk.iter().map(|(_, bytes)| bytes).sum();
        let size = if bytes > 0 {
            fmt_size(bytes)
        } else {
            String::new()
        };
        let detail = self.strings().files_on_disk(on_disk.len(), &size);
        self.confirm = Some(Confirm {
            heading: self.strings().remove_many.to_string(),
            subject: String::new(),
            size: String::new(),
            facts: vec![detail],
            seqs,
        });
        cx.notify();
    }

    /// Resolve the open dialog. `delete_files` decides whether the bytes go too.
    pub fn resolve_confirm(&mut self, delete_files: bool, cx: &mut Context<Self>) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        self.remove_now(confirm.seqs, delete_files, cx);
    }

    pub fn dismiss_confirm(&mut self, cx: &mut Context<Self>) {
        if self.confirm.take().is_some() {
            cx.notify();
        }
    }

    /// Drop rows from the list, tell aria2 to forget them, and optionally erase the files.
    ///
    /// The engine call has to land before the delete — aria2 holds the file open, and on
    /// Windows an open handle can make the removal fail. That ordering is why both happen
    /// inside one background task rather than in two detached ones.
    fn remove_now(&mut self, seqs: Vec<u64>, delete_files: bool, cx: &mut Context<Self>) {
        let engine = self.aria2.clone();
        let mut gids: Vec<(String, bool)> = Vec::new();
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut doomed: Vec<(u64, String)> = Vec::new();

        for seq in seqs {
            let Some(index) = self.index_of(seq) else {
                continue;
            };
            let task = self.tasks.remove(index);
            let finished = task.status.is_terminal();
            let path = delete_files.then(|| task.path());
            if let Some(gid) = task.gid {
                gids.push((gid, finished));
            }
            if let Some(path) = path {
                paths.push(path);
            }
            doomed.push((task.seq, task.name));
        }
        if doomed.is_empty() {
            return;
        }

        cx.notify();

        // The log records the intent, not just the aftermath, so a removal that also wipes
        // files is distinguishable from one that only forgot the task.
        for (seq, name) in &doomed {
            self.store.log(Event::Removed, Some(*seq), Some(name), None);
        }
        if delete_files {
            for path in &paths {
                let shown = path.to_string_lossy().into_owned();
                self.store
                    .log(Event::FilesDeleted, None, None, Some(&shown));
            }
        }
        for (seq, _) in &doomed {
            self.seen.remove(seq);
            // Hidden, not erased: the row is the record of what happened, and `seq` has to stay
            // taken so a later download cannot inherit this one's history.
            self.store.mark_deleted(*seq);
        }

        cx.spawn(async move |this, cx| {
            let leftovers = cx
                .background_spawn(async move {
                    if let Some(engine) = engine {
                        for (gid, finished) in gids {
                            if finished {
                                let _ = engine.forget(&gid);
                            } else {
                                // Removing a live download also leaves a "removed" record
                                // behind, so sweep that up too. A failure there is purely
                                // cosmetic.
                                let _ = engine.remove(&gid);
                                let _ = engine.forget(&gid);
                            }
                        }
                    }
                    paths
                        .into_iter()
                        .filter_map(|path| discard_download(&path).map(|err| (path, err)))
                        .collect::<Vec<_>>()
                })
                .await;

            // The row is already gone by now, so a file that refused to go would otherwise
            // reappear in Explorer with nothing to explain it.
            if leftovers.is_empty() {
                return;
            }
            this.update(cx, |this, cx| {
                for (path, err) in leftovers {
                    let message =
                        format!("{} {}: {err}", this.strings().delete_failed, path.display());
                    this.warn(message, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------------------ settings

    /// What the settings sheet shows and edits: the draft while it is open, the saved settings
    /// otherwise. Reading the whole UI through this is what makes theme, language, font and path
    /// preview live while still keeping every one of them out of the database until Save.
    pub fn active_settings(&self) -> &Settings {
        self.draft.as_ref().unwrap_or(&self.settings)
    }

    /// Open the preferences sheet on a snapshot of the saved settings. Nothing it edits touches
    /// disk until [`Self::commit_settings`]; [`Self::cancel_settings`] throws the snapshot away.
    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        if self.settings_open {
            return;
        }
        self.draft = Some(self.settings.clone());
        self.settings_open = true;
        self.load_setting_drafts(cx);
        cx.notify();
    }

    /// Save what the sheet has been editing, then close it.
    pub fn commit_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        self.settings = draft;
        if let Some(dir) = self
            .settings
            .download_dir
            .clone()
            .filter(|dir| !dir.is_empty())
        {
            let dir = PathBuf::from(dir);
            // The picker only returns folders that exist, but a path restored from a state file
            // may since have been removed; recreate it now that the user has confirmed.
            let _ = std::fs::create_dir_all(&dir);
            self.download_dir = dir;
        }
        // Write now rather than on the next poll tick. "Save" has to mean the bytes are on disk
        // even if the process dies a moment later, and the write is one transaction anyway.
        self.save_settings();
        self.apply_global_settings(cx);
        // A committed language change has to reach the command bar, which is older than the
        // setting it is now quoting.
        self.sync_placeholders(cx);
        self.close_settings(window, cx);
    }

    /// Throw the sheet's edits away and restore what was last saved.
    pub fn cancel_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.take().is_none() {
            return;
        }
        // The theme was installed live as a preview; put the stored one back, and put the command
        // bar's placeholder back with it (the language previews the same way the theme does).
        Look::update(cx, |look| look.mode = look_mode(self.settings.theme));
        self.sync_placeholders(cx);
        self.close_settings(window, cx);
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.font_menu_open = false;
        // The text boxes are the draft's other half; reload them so a reopened sheet never shows
        // what was typed and then abandoned.
        self.load_setting_drafts(cx);
        // The command bar is the app's default keyboard target. Clicking the gear blurs it (a
        // click landed outside the box), so hand focus back when the sheet closes — otherwise the
        // user would have to click the bar before pasting the next link.
        let handle = self.input.read(cx).focus_handle().clone();
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Keep every placeholder in step with the active language.
    ///
    /// The boxes outlive every settings change — the command bar is the app's front door and the
    /// sheet's own boxes are rebuilt around the sheet — so a placeholder cannot be a constructor
    /// argument that only ever gets read once.
    fn sync_placeholders(&self, cx: &mut Context<Self>) {
        let strings = self.strings();
        self.input.update(cx, |input, cx| {
            input.set_placeholder(strings.placeholder, cx)
        });
        self.font_input.update(cx, |input, cx| {
            input.set_placeholder(strings.font_placeholder, cx)
        });
        self.font_search.update(cx, |input, cx| {
            input.set_placeholder(strings.font_search_placeholder, cx)
        });
        self.ua_input.update(cx, |input, cx| {
            input.set_placeholder(strings.user_agent_placeholder, cx)
        });
        self.proxy_input.update(cx, |input, cx| {
            input.set_placeholder(strings.proxy_placeholder, cx)
        });
        if let Some(dialog) = self.add_dialog.as_ref() {
            for which in AddField::ALL {
                let placeholder = placeholder_for(which, strings);
                dialog
                    .input(which)
                    .update(cx, |input, cx| input.set_placeholder(placeholder, cx));
            }
        }
    }

    /// Point the three boxes at the saved settings. Called on open and on close, so a reopened
    /// sheet never shows what was typed and then abandoned.
    fn load_setting_drafts(&mut self, cx: &mut Context<Self>) {
        let font = self.settings.font.clone().unwrap_or_default();
        let user_agent = self.settings.user_agent.clone().unwrap_or_default();
        let proxy = self.settings.proxy.clone().unwrap_or_default();
        self.font_input
            .update(cx, |input, cx| input.set_text(font, cx));
        self.ua_input
            .update(cx, |input, cx| input.set_text(user_agent, cx));
        self.proxy_input
            .update(cx, |input, cx| input.set_text(proxy, cx));
    }

    /// The active translation table. `'static`, so callers can hold on to a string while
    /// still mutating the app.
    pub fn strings(&self) -> &'static Strings {
        i18n::Strings::get(self.active_settings().language)
    }

    pub fn set_language(&mut self, language: Language, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        if draft.language != language {
            draft.language = language;
            // The placeholder previews along with the rest of the sheet.
            self.sync_placeholders(cx);
            cx.notify();
        }
    }

    pub fn set_theme_mode(&mut self, mode: ThemeMode, _window: &Window, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        if draft.theme == mode {
            return;
        }
        draft.theme = mode;
        // Installed live so the user sees the palette before committing; Cancel puts the saved
        // one back.
        Look::update(cx, |look| look.mode = look_mode(mode));
        cx.notify();
    }

    /// The whole UI paints and measures with this. One place decides the family and the
    /// fallback chain, so no view has to know how the list is spelled.
    pub fn ui_font(&self, window: &Window) -> Font {
        let mut font = window.text_style().font();
        if let Some((family, fallbacks)) = self.resolved_font() {
            font.family = family.into();
            font.fallbacks = (!fallbacks.is_empty()).then(|| FontFallbacks::from_fonts(fallbacks));
        }
        font
    }

    /// The installed families to draw with, most preferred first.
    ///
    /// Names the platform does not know are dropped rather than passed through: GPUI asks the
    /// platform for the primary family and, when that fails, jumps straight to the system
    /// default, so an unknown first entry would silently discard the user's second choice.
    pub fn resolved_font(&self) -> Option<(String, Vec<String>)> {
        let mut families: Vec<String> = Vec::new();
        for name in font_stack(self.active_settings().font.as_deref()?) {
            let Some(installed) = self
                .fonts
                .iter()
                .find(|family| family.eq_ignore_ascii_case(&name))
            else {
                continue;
            };
            if !families.iter().any(|family| family == installed) {
                families.push(installed.clone());
            }
        }
        let mut families = families.into_iter();
        let primary = families.next()?;
        Some((primary, families.collect()))
    }

    /// Families asked for that are not installed, so the settings page can say which entries
    /// lost instead of leaving the user to guess.
    pub fn missing_fonts(&self) -> Vec<String> {
        font_stack(self.active_settings().font.as_deref().unwrap_or_default())
            .into_iter()
            .filter(|name| {
                !self
                    .fonts
                    .iter()
                    .any(|family| family.eq_ignore_ascii_case(name))
            })
            .collect()
    }

    /// The font box was edited. What the user typed is the source of truth while typing; what
    /// lands in the pending settings is its trimmed, empty-aware form.
    pub fn font_text_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        let trimmed = text.trim();
        let next = (!trimmed.is_empty()).then(|| trimmed.to_string());
        if let Some(draft) = self.draft.as_mut() {
            draft.font = next;
        }
        cx.notify();
    }

    /// The user-agent box was edited. Empty means "keep the built-in agent".
    pub fn ua_text_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        let next = non_empty(text);
        if let Some(draft) = self.draft.as_mut() {
            draft.user_agent = next;
        }
        cx.notify();
    }

    /// The proxy box was edited. Empty means "no proxy".
    pub fn proxy_text_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        let next = non_empty(text);
        if let Some(draft) = self.draft.as_mut() {
            draft.proxy = next;
        }
        cx.notify();
    }

    // ---------------------------------------------------------------- engine preferences

    /// The engine preferences as aria2 wants them. Read on every `addUri`, so changing a
    /// setting reaches the next download without restarting the engine.
    fn engine_options(&self) -> EngineOptions {
        EngineOptions {
            user_agent: self.settings.user_agent.clone(),
            // Clamp what a hand-edited database could put here: aria2 rejects
            // `--max-connection-per-server` above 16 and refuses to start, which would leave the
            // app with a dead engine and no obvious cause.
            connections: self.settings.connections.clamp(1, 16),
            max_concurrent: self.settings.max_concurrent.max(1),
            speed_limit: self.settings.speed_limit,
            proxy: self.settings.proxy.clone(),
            max_tries: self.settings.max_tries,
            timeout: self.settings.timeout.max(1),
        }
    }

    /// Push the engine-wide preferences to a running aria2. Per-download options do not need
    /// this — they travel with the next `addUri`.
    fn apply_global_settings(&self, cx: &mut Context<Self>) {
        let Some(engine) = self.aria2.clone() else {
            return;
        };
        let options = self.engine_options();
        // Best effort: if the engine has gone away the poll loop reports it, and the settings
        // page already has a banner for that.
        cx.background_spawn(async move {
            let _ = engine.apply_global(&options);
        })
        .detach();
    }

    pub fn set_connections(&mut self, value: u32, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        if draft.connections != value {
            draft.connections = value;
            cx.notify();
        }
    }

    pub fn set_max_concurrent(&mut self, value: u32, cx: &mut Context<Self>) {
        // Only staged: the running engine is told on Save (see `commit_settings`), because a
        // modal sheet gives the user nothing to watch it against.
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        if draft.max_concurrent != value {
            draft.max_concurrent = value;
            cx.notify();
        }
    }

    pub fn set_speed_limit(&mut self, value: u64, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        if draft.speed_limit != value {
            draft.speed_limit = value;
            cx.notify();
        }
    }

    pub fn set_max_tries(&mut self, value: u32, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        if draft.max_tries != value {
            draft.max_tries = value;
            cx.notify();
        }
    }

    pub fn set_timeout(&mut self, value: u32, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        if draft.timeout != value {
            draft.timeout = value;
            cx.notify();
        }
    }

    /// Stage the folder the picker returned. The folder is created, and becomes the one new
    /// downloads use, only on Save; existing rows keep the directory they were started in.
    pub fn set_download_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        let next = dir.to_string_lossy().into_owned();
        if draft.download_dir.as_deref() != Some(next.as_str()) {
            draft.download_dir = Some(next);
            cx.notify();
        }
    }

    /// Ask the platform for a folder. GPUI routes this to a native `IFileOpenDialog` with
    /// folder selection enabled, parented to the active window, and the answer arrives on
    /// the channel returned here.
    pub fn choose_download_dir(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(self.strings().choose_folder.into()),
        });

        cx.spawn(async move |this, cx| {
            let picked = match receiver.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                // Cancelled, or the platform had nothing to give us.
                Ok(Ok(None)) | Err(_) => None,
                Ok(Err(err)) => {
                    this.update(cx, |this, cx| {
                        let message = format!("{}: {err}", this.strings().picker_failed);
                        this.warn(message, cx);
                    })
                    .ok();
                    return;
                }
            };
            if let Some(dir) = picked {
                this.update(cx, |this, cx| this.set_download_dir(dir, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// Open the folder the history database lives in.
    pub fn reveal_store_dir(&mut self, cx: &mut Context<Self>) {
        if let Some(dir) = self.store.path().parent().map(Path::to_path_buf) {
            self.reveal_dir(dir, cx);
        }
    }

    /// Open a folder itself in the system file manager.
    pub fn reveal_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        cx.background_spawn(async move { open_folder(&dir) })
            .detach();
    }

    /// Show the finished file (or its folder) in the system file manager.
    pub fn reveal(&mut self, seq: u64, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(seq) else {
            return;
        };
        let file = self.tasks[index].path();
        let dir = self.tasks[index].dir.clone();
        cx.background_spawn(async move { reveal_in_file_manager(&file, &dir) })
            .detach();
    }

    // --------------------------------------------------------------------------- text boxes

    /// One of the sheet's boxes: a library input wired to whatever the sheet does when it changes,
    /// and to the sheet's own Escape rule.
    ///
    /// Four boxes share exactly this shape and half of it is easy to leave off — a box with a
    /// change handler but no dismissal rule still answers Escape, just not the way the sheet
    /// expects it to.
    fn settings_field(
        cx: &mut Context<Self>,
        placeholder: &'static str,
        changed: impl Fn(&mut Self, &str, &mut Window, &mut Context<Self>) + 'static,
    ) -> Entity<TextInput> {
        let on_change = cx.listener(changed);
        let on_dismiss =
            Self::dismissal(cx, |this, window, cx| this.leave_settings_field(window, cx));
        cx.new(move |cx| {
            TextInput::new(cx, placeholder)
                .on_change(on_change)
                .on_dismiss(on_dismiss)
        })
    }

    /// What Escape means inside a settings box: the font picker first, then the sheet itself.
    /// One layer at a time — the same rule the root applies when nothing at all is focused.
    fn leave_settings_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.font_menu_open {
            self.close_font_menu(window, cx);
        } else if self.settings_open {
            self.cancel_settings(window, cx);
        }
    }

    /// A box's Escape/Tab handler, bound to this app.
    ///
    /// `TextInput::on_dismiss` carries no event, so `Context::listener` cannot build it: the type
    /// is `Fn(&mut Window, &mut App)` with nothing in it to say which view it belongs to. A weak
    /// handle is the only thing that does.
    pub(crate) fn dismissal(
        cx: &Context<Self>,
        run: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl Fn(&mut Window, &mut App) + 'static {
        let weak = cx.weak_entity();
        move |window, cx| {
            weak.update(cx, |this, cx| run(this, window, cx)).ok();
        }
    }

    /// Whether any of the app's own boxes holds the caret.
    ///
    /// The root's single key handler needs this: a box answers Enter and Escape itself, and a
    /// second answer from the root would run the same action twice — close a dialog, then close
    /// whatever was underneath it.
    pub fn field_focused(&self, window: &Window, cx: &App) -> bool {
        let holds = |field: &Entity<TextInput>| field.read(cx).focus_handle().is_focused(window);
        holds(&self.input)
            || holds(&self.font_input)
            || holds(&self.font_search)
            || holds(&self.ua_input)
            || holds(&self.proxy_input)
            || self.add_dialog.as_ref().is_some_and(|dialog| {
                AddField::ALL
                    .into_iter()
                    .any(|which| holds(dialog.input(which)))
            })
    }

    /// Show or hide the font picker. Whatever the panel does with focus, it hands it back to the
    /// box it belongs to when it closes — otherwise the caret would be left in an element that is
    /// no longer drawn, and the keyboard would go nowhere at all.
    pub fn toggle_font_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.font_menu_open = !self.font_menu_open;
        if self.font_menu_open {
            self.font_search.update(cx, |field, cx| field.clear(cx));
            self.focus_field(&self.font_search, window, cx);
        } else {
            self.focus_field(&self.font_input, window, cx);
        }
        cx.notify();
    }

    pub fn close_font_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.font_menu_open {
            self.font_menu_open = false;
            self.focus_field(&self.font_input, window, cx);
            cx.notify();
        }
    }

    /// Hand the keyboard to one of the app's boxes.
    fn focus_field(&self, field: &Entity<TextInput>, window: &mut Window, cx: &mut Context<Self>) {
        let handle = field.read(cx).focus_handle().clone();
        window.focus(&handle, cx);
    }

    /// Append a family to the font list. Quoted when it has spaces, exactly as the box's own
    /// parser expects, and skipped when it is already there.
    pub fn choose_font(&mut self, family: &str, cx: &mut Context<Self>) {
        let entry = if family.contains(' ') {
            format!("\"{family}\"")
        } else {
            family.to_string()
        };
        let current = self.font_input.read(cx).text().to_string();
        let mut names: Vec<String> = crate::settings::font_stack(&current);
        if names.iter().any(|name| name == family) {
            return;
        }
        names.push(entry);
        let joined = names.join(", ");
        // Writing the box does not report a change — `set_text` is the app talking to the widget,
        // not the user talking to the app — so the sheet is told here.
        self.font_input
            .update(cx, |field, cx| field.set_text(joined.clone(), cx));
        self.font_text_changed(&joined, cx);
    }

    /// The installed families matching the picker's filter, in display order.
    pub fn font_matches(&self, cx: &App) -> Vec<&str> {
        let query = self.font_search.read(cx).text().trim().to_lowercase();
        self.fonts
            .iter()
            .filter(|family| query.is_empty() || family.to_lowercase().contains(&query))
            .map(String::as_str)
            .collect()
    }

    // ---------------------------------------------------------------- engine sync

    /// Fold one poll's worth of engine state into the list.
    fn apply(&mut self, snapshot: Snapshot) {
        let mut seen: HashSet<&str> = HashSet::new();
        for entry in &snapshot.tasks {
            let Some(gid) = entry["gid"].as_str() else {
                continue;
            };
            seen.insert(gid);
            let Some(task) = self
                .tasks
                .iter_mut()
                .find(|task| task.gid.as_deref() == Some(gid))
            else {
                // Downloads are only ever created by this app — we own a private aria2
                // instance — so an unrecognised gid is leftover engine state. Ignore it
                // rather than inventing a row for it.
                continue;
            };
            let before = task.status;
            task.absorb(entry);
            if before != task.status {
                self.dirty = true;
            }
        }

        // A task the engine no longer reports is not running any more.
        for task in self.tasks.iter_mut() {
            let stale = task.gid.as_deref().is_some_and(|gid| !seen.contains(gid));
            if stale && task.status.is_live() {
                task.status = Status::Paused;
                task.speed = 0;
                self.dirty = true;
            }
        }
    }

    pub fn warn(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        let message = message.into();
        self.notice = Some(message.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(6)).await;
            this.update(cx, |this, cx| {
                if this.notice.as_deref() == Some(message.as_str()) {
                    this.notice = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Run one RPC call off the UI thread and hand the outcome back to the view.
    fn dispatch<T, W, D>(&self, cx: &mut Context<Self>, work: W, done: D)
    where
        T: Send + 'static,
        W: FnOnce(&Aria2) -> anyhow::Result<T> + Send + 'static,
        D: FnOnce(&mut NexusApp, anyhow::Result<T>, &mut Context<NexusApp>) + 'static,
    {
        let Some(engine) = self.aria2.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { work(&engine) }).await;
            this.update(cx, |this, cx| done(this, result, cx)).ok();
        })
        .detach();
    }

    // ---------------------------------------------------------------- persistence

    /// Persist the preferences. Cheap enough to do whenever something changes.
    fn save_settings(&self) {
        self.settings.save(&self.store);
    }

    /// Write task rows that moved and log the transitions since the last call.
    ///
    /// A status change is written (and logged) immediately; byte counters ride the
    /// `PROGRESS_FLUSH` timer. Comparing against `seen` rather than a dirty flag means no
    /// transition can slip through between flushes.
    fn flush(&mut self) {
        if !self.store.enabled() {
            return;
        }
        let now = Instant::now();
        let due = now.duration_since(self.last_flush) >= PROGRESS_FLUSH;

        let mut pending: Vec<(Task, Option<Status>)> = Vec::new();
        for task in &self.tasks {
            let previous = self.seen.get(&task.seq).copied();
            if previous == Some((task.status, task.completed)) {
                continue;
            }
            let status_changed = previous.map(|(status, _)| status) != Some(task.status);
            if status_changed || due {
                pending.push((task.clone(), previous.map(|(status, _)| status)));
            }
        }
        if pending.is_empty() {
            return;
        }
        if due {
            self.last_flush = now;
        }

        for (task, previous) in pending {
            self.store.upsert_download(&task, task.created_at);
            self.seen.insert(task.seq, (task.status, task.completed));
            if previous != Some(task.status)
                && let Some(event) = Event::for_status(previous, task.status)
            {
                self.store.log(
                    event,
                    Some(task.seq),
                    Some(&task.name),
                    task.error.as_deref(),
                );
            }
        }
    }

    /// One-time move of `state.json` into the database. The file is renamed rather than
    /// deleted so the previous history is still recoverable by hand.
    fn import_legacy(&mut self) {
        if !self.store.enabled() || !self.store.is_empty() {
            return;
        }
        for task in &self.tasks {
            self.store.upsert_download(task, task.created_at);
        }
        self.save_settings();
        let _ = std::fs::rename(state_file(), data_dir().join("state.json.imported"));
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Persisted {
    #[serde(default)]
    tasks: Vec<Task>,
    #[serde(default)]
    download_dir: Option<String>,
    #[serde(default)]
    settings: Settings,
}

impl Persisted {
    fn load() -> Self {
        std::fs::read_to_string(state_file())
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }
}

/// Where the app keeps its state: the directory it was started in. Portable on purpose — the
/// database, the log and any one-time import backup sit beside whatever the user launched,
/// instead of being buried in the profile. Falls back to the temp directory if the process has
/// no working directory at all.
pub fn data_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir())
}

/// The palette the stored preference asks for.
///
/// `System` is left as `System`: the library resolves it against `App::window_appearance()`, which
/// is the only place that knows, and re-resolves on the observer below.
fn look_mode(mode: ThemeMode) -> LookMode {
    match mode {
        ThemeMode::System => LookMode::System,
        ThemeMode::Light => LookMode::Light,
        ThemeMode::Dark => LookMode::Dark,
    }
}

fn state_file() -> PathBuf {
    data_dir().join("state.json")
}

fn default_download_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join("Downloads"))
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| data_dir().join("downloads"))
}

/// `Some` for a non-blank, trimmed string; `None` for empty. Used by the optional engine
/// settings, where "unset" has exactly one in-memory spelling.
fn non_empty(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Accept the schemes aria2 understands; anything else is probably prose.
fn looks_like_uri(input: &str) -> bool {
    const SCHEMES: [&str; 8] = [
        "http://",
        "https://",
        "ftp://",
        "sftp://",
        "magnet:",
        "thunder://",
        "ed2k://",
        "http+unix://",
    ];
    let lower = input.to_ascii_lowercase();
    SCHEMES.iter().any(|scheme| lower.starts_with(scheme))
}

/// Is there anything of a download on disk yet? aria2 writes a `<file>.aria2` control file as
/// soon as it starts, which can exist while the payload itself does not.
fn has_download_data(path: &Path) -> bool {
    path.exists() || control_file(path).exists()
}

fn control_file(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.aria2", path.display()))
}

/// Erase what a forgotten download left on disk. `None` on success, otherwise the OS message for
/// why the bytes are still there.
///
/// aria2 keeps a `.aria2` control file beside a partial download, and a torrent's payload is a
/// directory rather than a file, so both shapes have to be handled. The retry exists because
/// Windows hangs on to the handle for a moment after the engine is told to forget the download,
/// and a single `remove_file` can lose that race.
fn discard_download(path: &Path) -> Option<String> {
    // aria2 deletes this itself the moment a download finishes, so a failure says nothing.
    let _ = std::fs::remove_file(control_file(path));

    let mut last = String::new();
    for attempt in 0..DELETE_TRIES {
        match remove_target(path) {
            Ok(()) => return None,
            // Gone is the goal, no matter who got there first.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
            Err(err) => last = err.to_string(),
        }
        if attempt + 1 < DELETE_TRIES {
            std::thread::sleep(DELETE_RETRY);
        }
    }
    Some(last)
}

fn remove_target(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Size of what is on disk at `path`, or `None` when it is a directory: a directory's own
/// metadata length says nothing useful about the payload inside it.
fn on_disk_size(path: &Path) -> Option<u64> {
    if path.is_dir() {
        return None;
    }
    std::fs::metadata(path).ok().map(|meta| meta.len())
}

fn reveal_in_file_manager(file: &Path, dir: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = Command::new("explorer.exe");
        if file.exists() {
            command.arg(format!("/select,{}", file.display()));
        } else {
            command.arg(dir);
        }
        let _ = command.creation_flags(CREATE_NO_WINDOW).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = file;
        let _ = Command::new("xdg-open").arg(dir).spawn();
    }
}

/// Open a folder in the system file manager.
///
/// Deliberately **not** `explorer /select`, which is what [`reveal_in_file_manager`] uses for a
/// file: `/select,<folder>` highlights the folder *inside its parent*, so "open the download
/// folder" landed the user one level up with the folder selected. Passing the path on its own opens
/// the folder itself.
fn open_folder(dir: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new("explorer.exe")
            .arg(dir)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("xdg-open").arg(dir).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    /// A fresh directory of our own, named with the process id so parallel tests cannot collide.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nexus-state-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The whole point of the button: the payload goes, not just the record of it.
    #[test]
    fn discarding_removes_the_payload_and_the_control_file() {
        let dir = scratch("payload");
        let file = dir.join("archlinux.iso");
        std::fs::write(&file, b"pretend this is 1.5 GB").unwrap();
        std::fs::write(control_file(&file), b"aria2 control").unwrap();

        assert!(discard_download(&file).is_none());
        assert!(!file.exists(), "the download itself should be gone");
        assert!(!control_file(&file).exists());
    }

    /// A torrent's payload is a directory, and `remove_file` on one fails with a sharing or
    /// permission error rather than doing the right thing.
    #[test]
    fn discarding_removes_a_whole_payload_directory() {
        let dir = scratch("tree");
        let payload = dir.join("ubuntu");
        std::fs::create_dir_all(payload.join("nested")).unwrap();
        std::fs::write(payload.join("nested").join("vmlinuz"), b"kernel").unwrap();

        assert!(discard_download(&payload).is_none());
        assert!(!payload.exists());
    }

    /// "Already gone" is the goal, so it must not be reported as a failure — this is the path a
    /// user hits after deleting the file in Explorer first.
    #[test]
    fn discarding_something_already_gone_is_not_an_error() {
        let dir = scratch("missing");
        assert!(discard_download(&dir.join("never-existed.iso")).is_none());
    }

    /// What the delete button promises, both halves at once: the bytes go, the record stays.
    ///
    /// Each half has its own test, but they live in different modules and are easy to get out of
    /// step — the row half changed most recently, and it must not have taken the file with it.
    #[test]
    fn forgetting_a_download_erases_the_bytes_but_keeps_the_row() {
        let dir = scratch("forget");
        let file = dir.join("archlinux.iso");
        std::fs::write(&file, b"pretend this is 1.5 GB").unwrap();

        let store = Store::open(dir.join("nexus.db"));
        let task = Task::placeholder(
            1,
            "https://example.com/archlinux.iso".into(),
            dir.display().to_string(),
        );
        store.upsert_download(&task, task.created_at);
        assert_eq!(store.counts().0, 1);

        // `remove_now` with `delete_files = true`, in the order it does them.
        store.mark_deleted(1);
        assert!(discard_download(&file).is_none());

        assert!(!file.exists(), "the file still has to be deleted");
        assert!(store.load_downloads().is_empty(), "and it leaves the list");
        assert_eq!(store.counts().0, 1, "but the row is not deleted");
        assert_eq!(store.highest_seq(), 1, "nor is its number given away");
    }
}
