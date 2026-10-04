//! The root view: title bar, command bar, filter row, and the task list.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, ColorExt, Context, FontWeight, IntoElement, KeyDownEvent, Render,
    Rgba, Window, WindowControlArea, div, px, rgb,
};

use super::settings::settings_dialog;
use super::text_field::{self, Outcome, apply_key};
use super::{
    CONNECTIONS, CONTROL, CONTROL_GAP, Chip, IconButton, Modal, TextButton, Variant, hint, icon,
    segmented, setting_row, speed_options,
};
use crate::i18n::Strings;
use crate::model::fmt_speed;
use crate::state::{AddField, Engine, Field, Filter, NexusApp};
use crate::theme::Theme;

/// Horizontal page gutter. Also used to derive the width available to the URL field.
pub const PAGE_PADDING: f32 = 20.0;
const BAR_PADDING: f32 = 16.0;
const BAR_ICON: f32 = 17.0;
const GAP_ICON: f32 = 11.0;
const GAP_BUTTON: f32 = 12.0;
const BUTTON_WIDTH: f32 = 108.0;
const BUTTON_HEIGHT: f32 = 38.0;
const COMMAND_HEIGHT: f32 = 54.0;
const INPUT_FONT: f32 = 13.5;

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
            .child(title_bar(
                &theme,
                strings,
                self.maximized,
                self.settings_open,
                cx,
            ))
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
            .gap(px(13.0))
            .px(px(PAGE_PADDING))
            .pb(px(PAGE_PADDING))
            .child(command_bar(self, strings, theme, window, cx))
            .when_some(banner(self, strings, theme), |element, banner| {
                element.child(banner)
            })
            .child(filter_bar(self, strings, theme, cx))
            .child(list)
            .into_any_element()
    }
}

// ---------------------------------------------------------------------- title bar

fn title_bar(
    theme: &Theme,
    strings: &Strings,
    maximized: bool,
    settings_open: bool,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    let (text, accent) = (theme.text, theme.accent);
    let (muted, hover_bg, danger) = (theme.text_muted, theme.surface_hover, theme.danger);

    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .h(px(46.0))
        .pl(px(19.0))
        .pr(px(9.0))
        .child(
            // The drag strip must be a *sibling* of the window buttons, never their
            // ancestor. The platform resolves a hit test by walking the window-control
            // hitboxes in registration order and taking the first match, and paint
            // registers a parent before its children. So a `Drag` area on an ancestor
            // captures the whole strip: pressing Minimise/Maximise/Close returns
            // `HTCAPTION`, and the buttons never see `HTMINBUTTON`/`HTMAXBUTTON`/
            // `HTCLOSE` — the window just drags instead. Keeping `Drag` on this branch
            // (and the buttons a sibling) leaves exactly one control area under the
            // cursor over the buttons.
            div()
                .flex()
                .flex_row()
                .items_center()
                .flex_1()
                .h_full()
                .gap(px(9.0))
                .window_control_area(WindowControlArea::Drag)
                .child(icon("icons/logo.svg", 17.0, accent))
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(text)
                        .child(strings.app_name),
                ),
        )
        .child(
            // A plain button, not a window control: it has to sit outside the drag strip (see
            // the note above), so it gets its own hitbox and an ordinary click handler.
            div().flex_none().mr(px(6.0)).child(
                IconButton::new(
                    "open-settings",
                    0,
                    "icons/gear.svg",
                    if settings_open { accent } else { muted },
                    if settings_open { accent } else { text },
                )
                .hover_bg(hover_bg)
                .active(settings_open)
                .box_size(30.0)
                .radius(8.0)
                .glyph_size(14.0)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings(cx))),
            ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.0))
                .child(window_button(
                    "win-min",
                    "icons/min.svg",
                    WindowControlArea::Min,
                    muted,
                    hover_bg,
                    muted,
                ))
                // The area stays `Max` either way: the platform maps it to
                // `HTMAXBUTTON`, which the OS already toggles between maximise and
                // restore on its own. Only the glyph has to follow the state.
                .child(window_button(
                    "win-max",
                    if maximized {
                        "icons/restore.svg"
                    } else {
                        "icons/max.svg"
                    },
                    WindowControlArea::Max,
                    muted,
                    hover_bg,
                    muted,
                ))
                .child(window_button(
                    "win-close",
                    "icons/x.svg",
                    WindowControlArea::Close,
                    muted,
                    danger,
                    rgb(0xffffff),
                )),
        )
}

/// A native window control: the platform handles the click via the control area, so there is no
/// handler here on purpose.
fn window_button(
    id: &'static str,
    glyph: &'static str,
    area: WindowControlArea,
    tint: Rgba,
    hover_bg: Rgba,
    hover_tint: Rgba,
) -> impl IntoElement + use<> {
    IconButton::new(id, 0, glyph, tint, hover_tint)
        .hover_bg(hover_bg)
        .box_size(30.0)
        .radius(8.0)
        .glyph_size(13.0)
        .area(area)
}

// -------------------------------------------------------------------- command bar

fn command_bar(
    this: &NexusApp,
    strings: &'static Strings,
    theme: &Theme,
    window: &mut Window,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    let focused = this.focus.is_focused(window);
    let filled = !this.input.text().trim().is_empty();
    let ready = matches!(this.engine, Engine::Online);
    let enabled = filled && ready;
    let (accent, faint) = (theme.accent, theme.text_faint);
    // Long links would otherwise be clipped at exactly the point the user is typing, so the row
    // slides left to keep the caret in view. The measurement needs the field's own font rather
    // than the window default, which during `render` still reports its own style.
    let font = this.ui_font(window);

    let submit = TextButton::new("command-submit", strings.add, Variant::Hero)
        .leading("icons/plus.svg", 15.0)
        .width(BUTTON_WIDTH)
        .enabled(enabled)
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit(cx)))
        .into_any_element();

    // The advanced path. The quick bar stays the one-paste default; this opens the dialog for a
    // download that needs its own name, folder, user agent or connection count.
    let advanced = IconButton::new(
        "open-advanced",
        0,
        "icons/sliders.svg",
        theme.text_muted,
        theme.text,
    )
    .hover_bg(theme.surface_hover)
    .box_size(BUTTON_HEIGHT)
    .radius(11.0)
    .glyph_size(17.0)
    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_add_dialog(window, cx)));

    let trailing = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(GAP_BUTTON))
        .child(submit)
        .child(advanced)
        .into_any_element();

    text_field::field(
        text_field::FieldSpec {
            id: "command-bar",
            target: Field::Link,
            edit: &this.input,
            focus: &this.focus,
            placeholder: strings.placeholder,
            caret_on: this.caret_on,
            metrics: text_field::Metrics {
                size: INPUT_FONT,
                height: COMMAND_HEIGHT,
                radius: 15.0,
                padding: BAR_PADDING,
                chrome: input_chrome(),
                gap: GAP_ICON,
                bg: theme.surface,
                width: text_field::Width::Full,
            },
            leading: Some(
                icon(
                    "icons/link.svg",
                    BAR_ICON,
                    if focused { accent } else { faint },
                )
                .into_any_element(),
            ),
            trailing: Some(trailing),
        },
        theme,
        &font,
        window,
        cx,
    )
}

/// Everything on the command bar's row except the text itself, which is what the caret has to
/// measure against to know when the line must scroll.
fn input_chrome() -> f32 {
    PAGE_PADDING * 2.0
        + BAR_PADDING * 2.0
        + BAR_ICON
        + GAP_ICON
        + GAP_BUTTON
        + BUTTON_WIDTH
        + GAP_BUTTON
        + BUTTON_HEIGHT
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
        Outcome::Submit => match field {
            Field::Link => this.submit(cx),
            Field::Add(_) => this.start_add_download(window, cx),
            _ => {}
        },
        Outcome::Dismiss => {
            // Escape closes the topmost layer in one step: the add dialog, then the font picker,
            // then the settings card.
            if this.add_dialog.is_some() {
                this.close_add_dialog(window, cx);
            } else if this.font_menu_open {
                this.close_font_menu(window, cx);
            } else if this.settings_open {
                this.cancel_settings(window, cx);
            } else if field == Field::Link {
                this.notice = None;
                cx.notify();
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

/// Engine failures and transient notices share one strip under the command bar.
fn banner(this: &NexusApp, strings: &Strings, theme: &Theme) -> Option<AnyElement> {
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

    if let Some(notice) = &this.notice {
        let (warning, muted) = (theme.warning, theme.text_muted);
        return Some(
            div()
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .gap(px(9.0))
                .px(px(13.0))
                .py(px(9.0))
                .rounded(px(11.0))
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(icon("icons/alert.svg", 14.0, warning))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(muted)
                        .child(notice.clone()),
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
