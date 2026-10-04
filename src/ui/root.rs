//! The root view: title bar, command bar, filter row, and the task list.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, ColorExt, Context, FontWeight, IntoElement, KeyDownEvent, Render,
    Window, div, px,
};

use super::settings::settings_dialog;
use super::text_field::{self, Outcome, apply_key};
use nexus_look::IconButton as LookIconButton;
use super::{
    CONNECTIONS, CONTROL, CONTROL_GAP, Chip, Modal, TextButton, Variant, hint, icon,
    segmented, setting_row, speed_options,
};
use crate::i18n::Strings;
use crate::model::fmt_speed;
use crate::state::{AddField, Engine, Field, Filter, NexusApp};
use nexus_look::Theme;

/// The page's horizontal gutter. Taken from the language rather than chosen here, so a screenshot
/// of Nexus and a screenshot of any other Nexus-look app line up as they should.
pub const PAGE_PADDING: f32 = nexus_look::space::XL;
const GAP_BUTTON: f32 = 12.0;

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
            // One keyboard handler for the whole view. Every field is a descendant, so their keys
            // bubble here, and `active_field` says which buffer they belong to — far safer than
            // hoping each box's own node ends up on the focus dispatch path.
            .on_key_down(cx.listener(handle_keys))
            .child(title_bar(strings, self.settings_open, window, cx))
            .child(self.body(strings, &theme, window, cx))
            .when_some(self.confirm.as_ref(), |element, confirm| {
                element.child(confirm_dialog(confirm, strings, &theme, window, cx))
            })
            .when(self.settings_open, |element| {
                element.child(settings_dialog(self, strings, &theme, window, cx))
            })
            .when(self.add_dialog.is_some(), |element| {
                element.child(add_dialog(self, strings, &theme, window, cx))
            })
    }
}

impl NexusApp {
    fn body(
        &self,
        strings: &'static Strings,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
            .child(command_bar(self, strings, theme, window, cx))
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
                        element.child(nexus_look::Toast::new("notice", notice).build(window, cx))
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
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    nexus_look::TitleBar::new(strings.app_name)
        .logo("icons/logo.svg")
        .action(
            LookIconButton::new("open-settings", nexus_look::icons::GEAR)
                .active(settings_open)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings(cx))),
        )
        .build(window, cx)
}

// -------------------------------------------------------------------- command bar

fn command_bar(
    this: &NexusApp,
    strings: &'static Strings,
    theme: &Theme,
    _window: &mut Window,
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

/// The view's single keyboard handler; `apply_key` owns everything about a field, and this only
/// decides what Enter and Escape mean for the field that happens to hold the caret.
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

    // The command bar owns its own editing keys — the library's input buffers, moves the caret and
    // talks to the IME itself. What is left for the app is what Enter and Escape *mean* here, and
    // only the app can know that.
    if this.input.read(cx).focus_handle().is_focused(window) {
        match key {
            "enter" => this.submit(cx),
            "escape" => {
                this.notice = None;
                cx.notify();
            }
            _ => {}
        }
        return;
    }

    // Clicking away blurs every box, so there may be no field at all. The dialogs still answer
    // Escape, and the add dialog answers Enter, even with nothing focused.
    let Some(field) = this.active_field(window) else {
        if this.add_dialog.is_some() {
            match key {
                "escape" => this.close_add_dialog(window, cx),
                "enter" => this.start_add_download(window, cx),
                _ => {}
            }
        } else if key == "escape" {
            // Escape backs out one layer at a time: the font picker, then the settings card.
            if this.font_menu_open {
                this.close_font_menu(window, cx);
            } else if this.settings_open {
                this.cancel_settings(window, cx);
            }
        }
        return;
    };

    let Some(edit) = this.field_mut(field) else {
        return;
    };
    let outcome = apply_key(edit, event, cx);
    match outcome {
        Outcome::Handled => this.field_changed(field, cx),
        Outcome::Submit => {
            if field == Field::Add(AddField::Uri) {
                this.start_add_download(window, cx);
            }
        }
        Outcome::Dismiss => {
            // Escape closes the topmost layer in one step: the add dialog, then the font picker,
            // then the settings card.
            if this.add_dialog.is_some() {
                this.close_add_dialog(window, cx);
            } else if this.font_menu_open {
                this.close_font_menu(window, cx);
            } else if this.settings_open {
                this.cancel_settings(window, cx);
            } else {
                window.blur();
                cx.notify();
            }
        }
        Outcome::Ignored => {}
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
            TextButton::new("clear-finished", strings.clear_finished, Variant::Ghost)
                .leading("icons/trash.svg", 13.0)
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
                .child(icon("icons/alert.svg", 15.0, danger))
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
    theme: &Theme,
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    let (text, muted) = (theme.text, theme.text_muted);

    // One line per fact. Joining them into a single line (what this used to do) meant a long name
    // pushed the path into a break wherever it happened to land — the path ended up split across
    // "C:\Users\Xuejun" and "\Downloads".
    let facts = confirm.facts.iter().fold(
        div().flex().flex_col().flex_none().gap(px(3.0)),
        |list, fact| {
            list.child(
                div()
                    .w_full()
                    .truncate()
                    .text_size(px(11.5))
                    .text_color(muted)
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
        .gap(px(12.0))
        .when(!confirm.subject.is_empty(), |row| {
            row.child(
                div()
                    .flex_1()
                    .truncate()
                    .text_size(px(12.5))
                    .text_color(text)
                    .child(confirm.subject.clone()),
            )
        })
        .when(!confirm.size.is_empty(), |row| {
            row.child(
                div()
                    .flex_none()
                    .text_size(px(12.5))
                    .text_color(text)
                    .child(confirm.size.clone()),
            )
        });
    let has_header = !confirm.subject.is_empty() || !confirm.size.is_empty();

    Modal::new("confirm-card", confirm.heading.clone())
        .narrow(CONFIRM_WIDTH)
        .block(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(px(5.0))
                .when(has_header, |element| element.child(header))
                .child(facts),
        )
        .action(
            TextButton::new("confirm-cancel", strings.cancel, Variant::Secondary)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.dismiss_confirm(cx))),
        )
        .action(
            TextButton::new("confirm-keep", strings.keep_file, Variant::Secondary).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| this.resolve_confirm(false, cx)),
            ),
        )
        .action(
            TextButton::new("confirm-delete", strings.delete_file, Variant::Danger).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| this.resolve_confirm(true, cx)),
            ),
        )
        .build(theme, window)
}

// -------------------------------------------------------------- add-download dialog

/// The confirmation box is the one card that is not a sheet: a single question and two or three
/// buttons. At the sheet width it would be mostly empty space.
const CONFIRM_WIDTH: f32 = 410.0;

/// The "save to" row's sibling button. Fixed width, so the box beside it has an exact width to hand
/// to the caret instead of an estimate.
const CHOOSE_BUTTON: f32 = 96.0;
/// What the box beside that button gets.
const DIR_BOX: f32 = CONTROL - CHOOSE_BUTTON - CONTROL_GAP;

/// One row of the new-download dialog: a label, and a box in the control column.
///
/// This is [`setting_row`] + [`text_field::control_box`] — the same two pieces the settings sheet is
/// built from, so the dialog reads as the same table rather than as a stack of forms.
#[allow(clippy::too_many_arguments)]
fn add_row(
    this: &NexusApp,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<NexusApp>,
    label: &'static str,
    note: &'static str,
    id: &'static str,
    field: AddField,
    placeholder: &'static str,
) -> impl IntoElement {
    setting_row(
        theme,
        label,
        note,
        text_field::control_box(
            this,
            theme,
            window,
            cx,
            id,
            Field::Add(field),
            placeholder,
            CONTROL,
        ),
    )
}

/// A row of choices, e.g. the connection count or the speed limit.
fn choice_row(
    theme: &Theme,
    label: &'static str,
    note: &'static str,
    id: &'static str,
    options: Vec<(
        &'static str,
        bool,
        impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    )>,
) -> impl IntoElement {
    setting_row(theme, label, note, segmented(theme, id, options))
}

/// The advanced new-download dialog. The quick bar stays the one-paste path; this is for the
/// times a download needs its own name, folder, user agent, proxy or connection count.
fn add_dialog(
    this: &NexusApp,
    strings: &'static Strings,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    let dialog = this.add_dialog.as_ref().expect("rendered only when open");
    let faint = theme.text_faint;

    let connections: Vec<_> = CONNECTIONS
        .iter()
        .map(|&(label, value)| {
            (
                label,
                dialog.connections == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_add_connections(value, cx)),
            )
        })
        .collect();
    let speed: Vec<_> = speed_options(strings)
        .iter()
        .map(|&(label, value)| {
            (
                label,
                dialog.speed_limit == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_add_speed_limit(value, cx)),
            )
        })
        .collect();

    Modal::new("add-card", strings.add_title)
        .scrolling()
        .block(add_row(
            this,
            theme,
            window,
            cx,
            strings.download_link,
            "",
            "add-uri",
            AddField::Uri,
            strings.placeholder,
        ))
        .block(save_row(this, strings, theme, window, cx))
        .block(add_row(
            this,
            theme,
            window,
            cx,
            strings.file_name,
            "",
            "add-name",
            AddField::Name,
            strings.file_name_placeholder,
        ))
        .block(add_row(
            this,
            theme,
            window,
            cx,
            strings.user_agent,
            strings.user_agent_note,
            "add-ua",
            AddField::UserAgent,
            strings.user_agent_placeholder,
        ))
        .block(choice_row(
            theme,
            strings.connections,
            strings.connections_note,
            "add-connections",
            connections,
        ))
        .block(add_row(
            this,
            theme,
            window,
            cx,
            strings.proxy,
            "",
            "add-proxy",
            AddField::Proxy,
            strings.proxy_placeholder,
        ))
        .block(add_row(
            this,
            theme,
            window,
            cx,
            strings.referer,
            "",
            "add-referer",
            AddField::Referer,
            strings.referer_placeholder,
        ))
        .block(choice_row(
            theme,
            strings.per_download_speed_limit,
            "",
            "add-speed",
            speed,
        ))
        .block(hint(strings.per_download_note, faint))
        .action(
            TextButton::new("add-cancel", strings.cancel, Variant::Secondary).on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.close_add_dialog(window, cx)),
            ),
        )
        .action(
            TextButton::new("add-start", strings.start_download, Variant::Primary).on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.start_add_download(window, cx)),
            ),
        )
        .build(theme, window)
}

/// The "save to" row: the folder box, and the button that opens the folder picker on the same line.
fn save_row(
    this: &NexusApp,
    strings: &'static Strings,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement {
    setting_row(
        theme,
        strings.save_to,
        "",
        div()
            .flex()
            .flex_row()
            .flex_none()
            .w(px(CONTROL))
            .items_center()
            .gap(px(CONTROL_GAP))
            .child(text_field::control_box(
                this,
                theme,
                window,
                cx,
                "add-dir",
                Field::Add(AddField::Dir),
                strings.dir_placeholder,
                DIR_BOX,
            ))
            .child(
                TextButton::new("add-choose-dir", strings.choose, Variant::Secondary)
                    .width(CHOOSE_BUTTON)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_add_dir(cx))),
            ),
    )
}
