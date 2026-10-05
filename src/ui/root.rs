//! The root view: title bar, command bar, filter row, the task list, and the three cards that
//! float over it — confirmation, settings and new download.

use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, ColorExt, Context, FontWeight, IntoElement, KeyDownEvent, Render,
    Window, div, px,
};

use super::settings::settings_dialog;
use super::{CONNECTIONS, Chip, icon, speed_options};
use crate::i18n::Strings;
use crate::model::fmt_speed;
use crate::state::{AddField, Engine, Filter, NexusApp};
use nexus_look::IconButton as LookIconButton;
use nexus_look::{Button, Choice, Modal, Row, Segmented, Theme, Tone, hint, space, text};

/// The page's horizontal gutter. Taken from the language rather than chosen here, so a screenshot
/// of Nexus and a screenshot of any other Nexus-look app line up as they should.
pub const PAGE_PADDING: f32 = nexus_look::space::XL;
const GAP_BUTTON: f32 = nexus_look::space::MD;

impl Render for NexusApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let strings = self.strings();

        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.bg)
            .text_color(theme.text)
            .font(self.ui_font(window))
            // One keyboard handler for the whole view. Every box is a descendant, so its keys
            // bubble here — but a box that has focus has already answered Enter and Escape
            // itself, and what is left for the root is the layers with no box to ask: the two
            // dialogs and the card behind them.
            .on_key_down(cx.listener(handle_keys))
            .child(title_bar(strings, self.settings_open, cx))
            .child(self.body(strings, &theme, cx))
            .when_some(self.confirm.as_ref(), |element, confirm| {
                element.child(confirm_dialog(confirm, strings, cx))
            })
            .when(self.settings_open, |element| {
                element.child(settings_dialog(self, strings, window, cx))
            })
            .when(self.add_dialog.is_some(), |element| {
                element.child(add_dialog(self, strings, window, cx))
            })
    }
}

impl NexusApp {
    fn body(&self, strings: &'static Strings, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let visible = self.visible();
        let empty = visible.is_empty();

        // Rows are built imperatively and type-erased: `task_row` needs `&mut Context`
        // (for `cx.listener`), which a `FnMut` closure in `.children(iter.map(..))`
        // cannot hand out.
        let mut rows: Vec<AnyElement> = Vec::with_capacity(visible.len());
        for &index in &visible {
            rows.push(
                super::task_row::task_row(self.tasks[index].clone(), strings, theme, cx)
                    .into_any_element(),
            );
        }

        let list: AnyElement = if empty {
            empty_state(theme, self.filter, strings).into_any_element()
        } else {
            div()
                .id("task-list")
                .flex()
                .flex_col()
                .gap(px(10.0))
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .children(rows)
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            // The command bar is a toolbar, so it spans the window and the gutter below it applies
            // to the *content* only. A strip inset by the page gutter is not a toolbar and not a
            // card — it is a white rectangle floating in the middle of the page, which is exactly
            // how it read.
            .child(command_bar(self, strings, theme, cx))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .gap(px(13.0))
                    .px(px(PAGE_PADDING))
                    .py(px(nexus_look::space::XL))
                    .when_some(engine_banner(self, strings, theme), |element, banner| {
                        element.child(banner)
                    })
                    .child(filter_bar(self, strings, theme, cx))
                    .child(list)
                    // The notice floats over this column rather than living in it, and is added
                    // last so it paints above the rows. There is at most one at a time; a second
                    // replaces the first (see `NexusApp::warn`).
                    .when_some(self.notice.clone(), |element, notice| {
                        element.child(nexus_look::Toast::new("notice", notice))
                    }),
            )
            .into_any_element()
    }
}

// ---------------------------------------------------------------------- title bar

/// The title bar is the library's: the drag strip, the four window controls and the rule about
/// which of them may contain the others are all in `nexus_look::TitleBar`. Nexus supplies the
/// product mark, the name and the gear.
fn title_bar(
    strings: &Strings,
    settings_open: bool,
    cx: &mut Context<NexusApp>,
) -> nexus_look::TitleBar {
    nexus_look::TitleBar::new(strings.app_name)
        .logo("icons/logo.svg")
        .action(
            LookIconButton::new("open-settings", nexus_look::icons::GEAR)
                .active(settings_open)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings(cx))),
        )
}

// -------------------------------------------------------------------- command bar

fn command_bar(
    this: &NexusApp,
    strings: &'static Strings,
    theme: &Theme,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    let filled = !this.input.read(cx).text().trim().is_empty();
    let ready = matches!(this.engine, Engine::Online);
    let enabled = filled && ready;

    let submit = nexus_look::Button::primary("command-submit", strings.add)
        .large()
        .icon(nexus_look::icons::PLUS)
        .disabled(!enabled)
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit(cx)));

    // The advanced path. The quick bar stays the one-paste default; this opens the dialog for a
    // download that needs its own name, folder, user agent or connection count.
    let advanced = LookIconButton::new("open-advanced", "icons/sliders.svg")
        .large()
        .outlined()
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_add_dialog(window, cx)));

    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_none()
        .w_full()
        .gap(px(GAP_BUTTON))
        .px(px(PAGE_PADDING))
        .py(px(nexus_look::space::MD))
        // No fill of its own: the toolbar is the window, held apart from the list below by one
        // hairline. A `surface` strip put a white band across the top of a grey page.
        .border_b_1()
        .border_color(theme.border_soft)
        .child(div().flex_1().min_w(px(0.0)).child(this.input.clone()))
        .child(submit)
        .child(advanced)
}

// ------------------------------------------------------------------- keyboard input

/// The view's single keyboard handler. Every box owns its own editing keys — the library's input
/// buffers, moves the caret and talks to the IME itself — so what is left for the root is what
/// Enter and Escape *mean* when no box is talking: which layer is open, and which one closes.
///
/// It lives on the root rather than on each box because a key event is dispatched along the path
/// from the window root to whatever holds the focus: the root is on every one of those paths,
/// where a box's own node is only there if that box is what happens to be focused.
fn handle_keys(
    this: &mut NexusApp,
    event: &KeyDownEvent,
    window: &mut Window,
    cx: &mut Context<NexusApp>,
) {
    let key = event.keystroke.key.as_str();

    // A modal dialog owns the keyboard while it is open, so nothing types through it.
    if this.confirm.is_some() {
        if key == "escape" {
            this.dismiss_confirm(cx);
        }
        return;
    }

    // A focused box has already answered Enter and Escape — on_submit and on_dismiss are wired to
    // the rule of whatever layer it is in. Answering again here would run the same action twice.
    if this.field_focused(window, cx) {
        return;
    }

    // With the dialog open and nothing focused (it opens with the URL box focused, but a click on
    // the card's padding blurs it), the dialog still answers the two keys that matter.
    if this.add_dialog.is_some() {
        match key {
            "escape" => this.close_add_dialog(window, cx),
            "enter" => this.start_add_download(window, cx),
            _ => {}
        }
        return;
    }

    if key == "escape" {
        // Escape backs out one layer at a time: the open panel, then the settings card.
        if this.panel.is_some() {
            this.close_panel(window, cx);
        } else if this.settings_open {
            this.cancel_settings(window, cx);
        }
    }
}

// --------------------------------------------------------------------- filter row

fn filter_bar(
    this: &NexusApp,
    strings: &'static Strings,
    theme: &Theme,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    let (live, finished) = (
        this.count(Filter::Downloading),
        this.count(Filter::Completed),
    );
    let muted = theme.text_muted;
    let accent = theme.accent;
    // Both derived from the rows themselves, so the header can never describe a transfer that is
    // not happening — pausing drops the speed and the count in the same frame.
    let speed = this.speed();
    let live_count = live as u64;

    let mut right = div().flex().flex_row().items_center().gap(px(18.0));

    if live > 0 || speed > 0 {
        right = right.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(7.0))
                .text_size(px(12.0))
                .text_color(muted)
                .child(icon("icons/bolt.svg", 13.0, accent))
                .child(format!(
                    "{}  ·  {}",
                    fmt_speed(speed),
                    strings.active_count(live_count)
                )),
        );
    }

    if finished > 0 {
        right = right.child(
            Button::ghost("clear-finished", strings.clear_finished)
                .icon(nexus_look::icons::TRASH)
                .on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.request_clear_finished(cx)),
                ),
        );
    }

    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .children(Filter::ALL.into_iter().map(|filter| {
                    Chip::new(
                        ("filter", filter as usize),
                        filter.label(strings),
                        this.count(filter),
                        this.filter == filter,
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.set_filter(filter, cx)),
                    )
                })),
        )
        .child(right)
}

// -------------------------------------------------------------------------- banner

/// The engine is gone, and it is going to stay gone until the user does something about it.
///
/// It stays *in* the column, unlike the transient notice (`Toast`), because it describes a standing
/// condition rather than an answer to something just typed: it is part of the page until the engine
/// answers again, and it carries a long reason that has to be readable while the list is used.
fn engine_banner(this: &NexusApp, strings: &Strings, theme: &Theme) -> Option<AnyElement> {
    if let Engine::Failed(reason) = &this.engine {
        let (danger, wash, muted) = (theme.danger, theme.danger_wash, theme.text_muted);
        return Some(
            div()
                .flex()
                .flex_row()
                .flex_none()
                .items_start()
                .gap(px(10.0))
                .px(px(14.0))
                .py(px(11.0))
                .rounded(px(12.0))
                .bg(wash)
                .border_1()
                .border_color(danger.opacity(0.28))
                .child(icon(nexus_look::icons::ALERT, 15.0, danger))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .text_color(danger)
                                .child(strings.engine_unavailable),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(muted)
                                .child(reason.clone()),
                        ),
                )
                .into_any_element(),
        );
    }

    None
}

// ---------------------------------------------------------------------- empty state

fn empty_state(theme: &Theme, filter: Filter, strings: &Strings) -> impl IntoElement + use<> {
    let (title, hint) = match filter {
        Filter::All => (strings.empty_all_title, strings.empty_all_hint),
        Filter::Downloading => (strings.empty_active_title, strings.empty_active_hint),
        Filter::Completed => (strings.empty_finished_title, strings.empty_finished_hint),
    };
    let (surface, border, faint, muted) = (
        theme.surface,
        theme.border,
        theme.text_faint,
        theme.text_muted,
    );

    div()
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(13.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(58.0))
                .rounded(px(18.0))
                .bg(surface)
                .border_1()
                .border_color(border)
                .child(icon("icons/download.svg", 24.0, faint)),
        )
        .child(
            div()
                .text_size(px(14.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(muted)
                .child(title),
        )
        .child(div().text_size(px(12.0)).text_color(faint).child(hint))
}

// ------------------------------------------------------------------ confirm dialog

/// Modal shown before a removal that would touch files on disk. Every way out is a button, so a
/// stray click can never delete anything.
fn confirm_dialog(
    confirm: &crate::state::Confirm,
    strings: &'static Strings,
    cx: &mut Context<NexusApp>,
) -> Modal {
    let theme = Theme::of(cx);

    // One line per fact. Joining them into a single line (what this used to do) meant a long name
    // pushed the path into a break wherever it happened to land — the path ended up split across
    // "C:\Users\Xuejun" and "\Downloads".
    let facts = confirm.facts.iter().fold(
        div().flex().flex_col().flex_none().gap(px(space::XS)),
        |list, fact| {
            list.child(
                div()
                    .w_full()
                    .truncate()
                    .text_size(px(text::CAPTION))
                    .text_color(theme.text_muted)
                    .child(fact.clone()),
            )
        },
    );

    // What is being removed and how big it is are the two things the answer turns on, so they
    // share a line with the size pushed to the right edge — the same side the settings sheet puts
    // a value on. The location goes underneath because it is the one that runs long.
    let header = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(space::MD))
        .when(!confirm.subject.is_empty(), |row| {
            row.child(
                div()
                    .flex_1()
                    .truncate()
                    .text_size(px(text::BODY))
                    .text_color(theme.text)
                    .child(confirm.subject.clone()),
            )
        })
        .when(!confirm.size.is_empty(), |row| {
            row.child(
                div()
                    .flex_none()
                    .text_size(px(text::BODY))
                    .text_color(theme.text)
                    .child(confirm.size.clone()),
            )
        });
    let has_header = !confirm.subject.is_empty() || !confirm.size.is_empty();

    Modal::new("confirm-card", confirm.heading.clone())
        .narrow(nexus_look::layout::MODAL_W_NARROW)
        .block(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(px(space::SM))
                .when(has_header, |element| element.child(header))
                .child(facts),
        )
        .action(
            Button::secondary("confirm-cancel", strings.cancel)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.dismiss_confirm(cx))),
        )
        .action(
            Button::secondary("confirm-keep", strings.keep_file).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| this.resolve_confirm(false, cx)),
            ),
        )
        .action(
            Button::danger("confirm-delete", strings.delete_file).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| this.resolve_confirm(true, cx)),
            ),
        )
}

// -------------------------------------------------------------- add-download dialog

/// The advanced new-download dialog. The quick bar stays the one-paste path; this is for the
/// times a download needs its own name, folder, user agent, proxy or connection count.
///
/// Every row is the settings sheet's `Row`, so this card is the same table with different rows
/// rather than a stack of forms: the two line up field for field, control column to control
/// column, and neither can drift into its own idea of where a label ends.
fn add_dialog(
    this: &NexusApp,
    strings: &'static Strings,
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> Modal {
    let dialog = this.add_dialog.as_ref().expect("rendered only when open");

    let connections: Vec<_> = CONNECTIONS
        .iter()
        .map(|&(label, value)| {
            Choice::new(
                label,
                dialog.connections == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_add_connections(value, cx)),
            )
        })
        .collect();
    let speed: Vec<_> = speed_options(strings)
        .iter()
        .map(|&(label, value)| {
            Choice::new(
                label,
                dialog.speed_limit == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_add_speed_limit(value, cx)),
            )
        })
        .collect();

    let box_of = |which| dialog.input(which).clone();

    Modal::new("add-card", strings.add_title)
        .scrolling()
        .block(Row::new(strings.download_link, "").control(box_of(AddField::Uri)))
        .block(
            Row::new(strings.save_to, "")
                .control(box_of(AddField::Dir))
                .action(
                    Button::secondary("add-choose-dir", strings.choose).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.choose_add_dir(cx)),
                    ),
                ),
        )
        .block(Row::new(strings.file_name, "").control(box_of(AddField::Name)))
        .block(
            Row::new(strings.user_agent, strings.user_agent_note)
                .control(box_of(AddField::UserAgent)),
        )
        .block(
            Row::new(strings.connections, strings.connections_note)
                .control(Segmented::new("add-connections").choices(connections)),
        )
        .block(Row::new(strings.proxy, "").control(box_of(AddField::Proxy)))
        .block(Row::new(strings.referer, "").control(box_of(AddField::Referer)))
        .block(
            Row::new(strings.per_download_speed_limit, "")
                .control(Segmented::new("add-speed").choices(speed)),
        )
        .block(hint(strings.per_download_note, Tone::Neutral, window, cx))
        .action(Button::secondary("add-cancel", strings.cancel).on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| this.close_add_dialog(window, cx)),
        ))
        .action(
            Button::primary("add-start", strings.start_download).on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.start_add_download(window, cx)),
            ),
        )
}
