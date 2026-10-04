//! The settings sheet: downloads, engine, appearance, language and data.
//!
//! The table itself is not this file's business. `SettingGroup`, `Row`, `Modal` and `SwatchGrid`
//! come from Nexus-look and the new-download dialog is built from the same three, which is what
//! makes the two read as one product rather than as a sheet plus a stack of forms. What is here is
//! Nexus: which preferences exist, what each control does to them, and the font picker — a
//! platform font list behind a filter, over a comma-separated draft, which no generic widget
//! could own.

use crate::i18n::Strings;
use crate::settings::{DEFAULT_ACCENT, Language, ThemeMode, font_stack, parse_accent};
use crate::state::{NexusApp, Panel};
use gpui::prelude::*;
use gpui::{AnyElement, App, ClickEvent, Context, Div, SharedString, Window, div, px};
use nexus_look::{
    Button, Choice, IconButton, Modal, RADIUS, Row, Segmented, SettingGroup, SwatchGrid, Theme,
    Tone, divider, hint, icon, layout, space, subheading, text,
};

use super::{CONNECTIONS, speed_options};

/// How many downloads run at once.
const MAX_CONCURRENT: [(&str, u32); 6] =
    [("1", 1), ("2", 2), ("3", 3), ("5", 5), ("8", 8), ("10", 10)];
/// Seconds before an idle or unreachable server is given up on.
const TIMEOUTS: [(&str, u32); 4] = [("15s", 15), ("30s", 30), ("60s", 60), ("120s", 120)];
/// How tall the font picker's list may get before it scrolls inside itself: six families at the
/// row's own height (a `BODY` line with `XS` above and below, then a little more). The panel
/// floats inside a card that is already capped, so a list with no end of its own would push the
/// sheet's footer off the bottom of it.
const PICKER_LIST: f32 = 176.0;

pub(super) fn settings_dialog(
    this: &NexusApp,
    strings: &'static Strings,
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> Modal {
    let theme_options: Vec<Choice> = ThemeMode::ALL
        .iter()
        .map(|mode| {
            let label = match mode {
                ThemeMode::System => strings.theme_system,
                ThemeMode::Light => strings.theme_light,
                ThemeMode::Dark => strings.theme_dark,
            };
            let mode = *mode;
            Choice::new(
                label,
                this.active_settings().theme == mode,
                cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_theme_mode(mode, window, cx)
                }),
            )
        })
        .collect();

    let language_options: Vec<Choice> = Language::ALL
        .iter()
        .map(|language| {
            let language = *language;
            Choice::new(
                language.label(),
                this.active_settings().language == language,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_language(language, cx)),
            )
        })
        .collect();

    // While the font box has focus, whatever follows the last comma is still being typed, so an
    // unfinished name is not a mistake yet. Reporting it would flash a warning on the way to every
    // correct spelling.
    let mut missing = this.missing_fonts();
    {
        let field = this.font_input.read(cx);
        if field.focus_handle().is_focused(window)
            && !field.text().trim_end().ends_with(',')
            && let Some(typing) = font_stack(field.text()).pop()
        {
            missing.retain(|name| *name != typing);
        }
    }
    let using = this.resolved_font().map(|(primary, _)| primary);
    let (records, error) = (this.store.counts(), this.store.error().map(str::to_string));

    let downloads =
        SettingGroup::new(strings.section_downloads, strings.downloads_summary).child(
            Row::new(strings.download_folder, strings.folder_note)
                .control(path_text(
                    this.active_settings()
                        .download_dir
                        .clone()
                        .unwrap_or_default(),
                    cx,
                ))
                .action(Button::secondary("choose-dir", strings.change).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.choose_download_dir(cx)),
                )),
        );

    let mut appearance = SettingGroup::new(strings.section_appearance, strings.appearance_summary)
        .child(
            Row::new(strings.theme, strings.theme_note)
                .control(Segmented::new("theme").choices(theme_options)),
        )
        .child(accent_row(this, strings, cx));
    // Read straight from the box rather than from a flag: the only way for it to be wrong is for
    // someone to be typing in it, and a flag that a text field owns would go stale the moment
    // something else redrew.
    let typed = this.accent_input.read(cx).text().to_string();
    if !typed.trim().is_empty() && parse_accent(&typed).is_none() {
        appearance = appearance.child(hint(strings.accent_invalid, Tone::Danger, window, cx));
    }
    appearance =
        appearance.child(Row::new(strings.font, strings.font_note).control(font_box(this, cx)));
    if this.panel == Some(Panel::Fonts) {
        appearance = appearance.child(font_picker(this, strings, window, cx));
    }
    if let Some(family) = using {
        appearance = appearance.child(hint(
            format!("{}{family}", strings.font_using),
            Tone::Neutral,
            window,
            cx,
        ));
    }
    if !missing.is_empty() {
        appearance = appearance.child(hint(
            format!("{}{}", strings.font_missing, missing.join(", ")),
            Tone::Danger,
            window,
            cx,
        ));
    }

    let language = SettingGroup::new(strings.section_language, strings.language_summary).child(
        Row::new(strings.language, "")
            .control(Segmented::new("language").choices(language_options)),
    );

    let data = SettingGroup::new(strings.section_data, strings.data_summary).child(
        Row::new(strings.database, strings.records(records.0, records.1))
            .control(path_text(
                this.store.path().to_string_lossy().into_owned(),
                cx,
            ))
            .action(
                Button::secondary("reveal-db", strings.open_folder)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.reveal_store_dir(cx))),
            ),
    );
    let data = if let Some(error) = error {
        data.child(hint(
            format!("{}: {error}", strings.storage_disabled),
            Tone::Danger,
            window,
            cx,
        ))
    } else {
        data
    };

    Modal::new("settings-card", strings.settings_title)
        .scrolling()
        .block(downloads)
        .block(divider(cx))
        .block(engine_group(this, strings, window, cx))
        .block(divider(cx))
        .block(appearance)
        .block(divider(cx))
        .block(language)
        .block(divider(cx))
        .block(data)
        .action(
            Button::secondary("settings-cancel", strings.cancel).on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.cancel_settings(window, cx)),
            ),
        )
        .action(Button::primary("settings-save", strings.save).on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| this.commit_settings(window, cx)),
        ))
}

/// A path that cannot be edited, beside the one button that acts on it.
///
/// Deliberately not a boxed field: nothing here can be typed, and a box would promise otherwise.
/// Plain text also lets the two "where things live" rows share one shape.
fn path_text(value: String, cx: &App) -> Div {
    let theme = Theme::of(cx);
    div()
        .w_full()
        .truncate()
        .text_size(px(text::BODY))
        .text_color(theme.text_muted)
        .child(value)
}

/// The engine group: user agent, connections, proxy, then the concurrency tier.
///
/// Split out of [`settings_dialog`] so each stays readable; it owns the rows, not the card chrome.
fn engine_group(
    this: &NexusApp,
    strings: &'static Strings,
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> SettingGroup {
    let connections: Vec<_> = CONNECTIONS
        .iter()
        .map(|&(label, value)| {
            Choice::new(
                label,
                this.active_settings().connections == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_connections(value, cx)),
            )
        })
        .collect();
    let concurrent: Vec<_> = MAX_CONCURRENT
        .iter()
        .map(|&(label, value)| {
            Choice::new(
                label,
                this.active_settings().max_concurrent == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_max_concurrent(value, cx)),
            )
        })
        .collect();
    let speed: Vec<_> = speed_options(strings)
        .iter()
        .map(|&(label, value)| {
            Choice::new(
                label,
                this.active_settings().speed_limit == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_speed_limit(value, cx)),
            )
        })
        .collect();
    let tries: Vec<_> = [(strings.unlimited, 0u32), ("3", 3), ("5", 5), ("10", 10)]
        .iter()
        .map(|&(label, value)| {
            Choice::new(
                label,
                this.active_settings().max_tries == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_max_tries(value, cx)),
            )
        })
        .collect();
    let timeout: Vec<_> = TIMEOUTS
        .iter()
        .map(|&(label, value)| {
            Choice::new(
                label,
                this.active_settings().timeout == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_timeout(value, cx)),
            )
        })
        .collect();

    SettingGroup::new(strings.section_engine, strings.engine_summary)
        .child(Row::new(strings.user_agent, strings.user_agent_note).control(this.ua_input.clone()))
        .child(
            Row::new(strings.connections, strings.connections_note)
                .control(Segmented::new("engine-connections").choices(connections)),
        )
        .child(Row::new(strings.proxy, strings.proxy_note).control(this.proxy_input.clone()))
        .child(subheading(strings.subsection_concurrency, cx))
        .child(
            Row::new(strings.max_concurrent, strings.max_concurrent_note)
                .control(Segmented::new("engine-concurrent").choices(concurrent)),
        )
        .child(
            Row::new(strings.speed_limit, strings.speed_limit_note)
                .control(Segmented::new("engine-speed").choices(speed)),
        )
        .child(
            Row::new(strings.max_tries, strings.max_tries_note)
                .control(Segmented::new("engine-tries").choices(tries)),
        )
        .child(
            Row::new(strings.timeout, strings.timeout_note)
                .control(Segmented::new("engine-timeout").choices(timeout)),
        )
        .child(hint(strings.engine_note, Tone::Neutral, window, cx))
}

/// The colour row: eight swatches, the hex box when the custom panel is open, and the way back
/// to the default.
///
/// The grid and the eight presets are the library's — "which colours" is part of the language —
/// while the two buttons and the hex box are Nexus's, because only Nexus knows the labels and what
/// the default is.
fn accent_row(this: &NexusApp, strings: &'static Strings, cx: &mut Context<NexusApp>) -> Row {
    let mut swatches =
        SwatchGrid::new("accent")
            .selected(this.active_settings().accent)
            // The dot and the reset point at one colour, and both live in `settings.rs`.
            .default(DEFAULT_ACCENT)
            .on_pick(cx.listener(|this, value: &u32, _, cx| this.set_accent(*value, cx)))
            .action(
                Button::ghost("accent-custom", strings.accent_custom).on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| this.toggle_accent_panel(window, cx),
                )),
            )
            .action(
                Button::ghost("accent-reset", strings.accent_reset).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.set_accent(DEFAULT_ACCENT, cx)),
                ),
            );
    if this.panel == Some(Panel::Accent) {
        swatches = swatches.hex(this.accent_input.clone());
    }
    Row::new(strings.accent, strings.accent_note).control(swatches)
}

/// The font box: the family list, plus the button that opens the picker beside it.
fn font_box(this: &NexusApp, cx: &mut Context<NexusApp>) -> Div {
    let open = this.panel == Some(Panel::Fonts);

    div()
        .flex()
        .flex_row()
        .flex_none()
        .w_full()
        .gap(px(space::SM))
        // The box takes what is left, so the button's own size never has to be subtracted from a
        // measured width — and the caret scrolls against whatever the row turned out to be.
        .child(div().flex_1().min_w(px(0.0)).child(this.font_input.clone()))
        .child(
            IconButton::new("font-add", nexus_look::icons::PLUS)
                .outlined()
                .active(open)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.toggle_font_menu(window, cx)
                })),
        )
}

/// The picker: a filter box over every family the platform reports, one row each. Clicking a row
/// appends that family to the list in the box above.
///
/// It is Nexus's own widget rather than a library one: a *font* list is nothing but a platform
/// fact with a preview attached, and no other project would want this shape.
fn font_picker(
    this: &NexusApp,
    strings: &'static Strings,
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> Div {
    // Read the two colours the panel needs and let the borrow go: the rows below take `cx` for
    // their click handlers, and a `&Theme` still alive at that point would be a second borrow.
    let (bg, border) = {
        let theme = Theme::of(cx);
        (theme.bg, theme.border)
    };
    let matches = this.font_matches(cx);
    let chosen: Vec<String> = font_stack(this.font_input.read(cx).text());
    // Collected up front so the rows own their names: the list is borrowed from the view, and the
    // click handlers have to outlive this frame.
    let rows: Vec<SharedString> = matches
        .iter()
        .map(|family| SharedString::from(family.to_string()))
        .collect();
    let empty = rows.is_empty();

    let list = div()
        .id("font-list")
        .flex()
        .flex_col()
        .flex_none()
        .max_h(px(PICKER_LIST))
        .overflow_y_scroll()
        .children(rows.into_iter().map(|family| {
            let picked = chosen.iter().any(|name| name == family.as_ref());
            font_option(family, picked, cx)
        }));

    div().flex().flex_row().flex_none().justify_end().child(
        div()
            .flex()
            .flex_col()
            .flex_none()
            // The control column's own width, so the panel sits exactly over the box that opened
            // it instead of wandering off the right edge of the table.
            .w(px(layout::CONTROL_COL))
            .gap(px(space::SM))
            .p(px(space::SM))
            .rounded(px(RADIUS))
            .bg(bg)
            .border_1()
            .border_color(border)
            .child(this.font_search.clone())
            .child(list)
            .when(empty, |element| {
                element.child(hint(strings.font_no_match, Tone::Neutral, window, cx))
            }),
    )
}

/// One entry in the picker, drawn in its own face so the list doubles as a preview. One already in
/// the list is ticked and does nothing, so it cannot be added twice.
fn font_option(family: SharedString, picked: bool, cx: &mut Context<NexusApp>) -> AnyElement {
    let theme = Theme::of(cx);
    let (hover, accent) = (theme.surface_hover, theme.accent);
    let label = family.clone();
    let value = family.to_string();

    div()
        .id(family)
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .justify_between()
        .gap(px(space::SM))
        .px(px(space::SM))
        .py(px(space::XS))
        .rounded(px(RADIUS))
        .cursor_pointer()
        .text_size(px(text::BODY))
        .text_color(theme.text)
        .hover(move |style| style.bg(hover))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.choose_font(&value, cx)))
        .child(div().truncate().font_family(label.clone()).child(label))
        .when(picked, |element| {
            element.child(icon(nexus_look::icons::CHECK, nexus_look::ICON_SM, accent))
        })
        .into_any_element()
}
