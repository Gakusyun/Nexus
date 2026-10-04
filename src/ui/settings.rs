//! The settings sheet: downloads, engine, appearance, language and data.
//!
//! Laid out as a sheet rather than as cards inside a card — one column of labels down the left,
//! one column of controls down the right, groups split by a hairline. See STYLE.md §6.

use crate::i18n::Strings;
use crate::settings::{Language, ThemeMode, font_stack};
use crate::state::{Field, NexusApp};
use nexus_look::Theme;
use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Context, IntoElement, SharedString, Window, div, px};

use super::{
    CONNECTIONS, CONTROL, CONTROL_GAP, CONTROL_HEIGHT, IconButton, Modal, TextButton, Variant,
    chrome_for, hint, icon, segmented, setting_row, settings_divider, settings_group,
    settings_subheading, speed_options, text_field,
};

/// The most room the plain path text may take before it truncates. Long enough for a typical path,
/// and capped so a very long one cannot squeeze the label column beside it.
const PATH_WIDTH: f32 = 360.0;
/// The font picker is narrower than the control column so it tucks under the font box rather than
/// under the card's edge.
const FONT_PICKER: f32 = 300.0;
/// Text the font box can hold: it shares its row with the "+" button, and pads 11 a side.
const FONT_TEXT: f32 = CONTROL - CONTROL_HEIGHT - CONTROL_GAP - 11.0 * 2.0 - 2.0;
/// Text the picker's filter can hold: the picker and the filter each pad 9 a side.
const PICKER_TEXT: f32 = FONT_PICKER - 9.0 * 2.0 - 2.0 - 9.0 * 2.0 - 2.0;

/// How many downloads run at once.
const MAX_CONCURRENT: [(&str, u32); 6] =
    [("1", 1), ("2", 2), ("3", 3), ("5", 5), ("8", 8), ("10", 10)];
/// Seconds before an idle or unreachable server is given up on.
const TIMEOUTS: [(&str, u32); 4] = [("15s", 15), ("30s", 30), ("60s", 60), ("120s", 120)];

pub(super) fn settings_dialog(
    this: &NexusApp,
    strings: &'static Strings,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    let muted = theme.text_muted;

    let mut missing = this.missing_fonts();
    // While the box has focus, whatever follows the last comma is still being typed, so an
    // unfinished name is not a mistake yet. Reporting it would flash a warning on the way to
    // every correct spelling.
    if this.font_focus.is_focused(window)
        && !this.font_draft.text().trim_end().ends_with(',')
        && let Some(typing) = font_stack(this.font_draft.text()).pop()
    {
        missing.retain(|name| *name != typing);
    }
    let using = this.resolved_font().map(|(primary, _)| primary);

    let theme_options = ThemeMode::ALL
        .iter()
        .map(|mode| {
            let label = match mode {
                ThemeMode::System => strings.theme_system,
                ThemeMode::Light => strings.theme_light,
                ThemeMode::Dark => strings.theme_dark,
            };
            let mode = *mode;
            (
                label,
                this.active_settings().theme == mode,
                cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_theme_mode(mode, window, cx)
                }),
            )
        })
        .collect();

    let language_options = Language::ALL
        .iter()
        .map(|language| {
            let language = *language;
            (
                language.label(),
                this.active_settings().language == language,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_language(language, cx)),
            )
        })
        .collect();

    let (records, error) = (this.store.counts(), this.store.error().map(str::to_string));

    Modal::new("settings-card", strings.settings_title)
        .scrolling()
        .block(
            settings_group(theme, strings.section_downloads, strings.downloads_summary).child(
                setting_row(
                    theme,
                    strings.download_folder,
                    strings.folder_note,
                    path_row(
                        theme,
                        this.active_settings()
                            .download_dir
                            .clone()
                            .unwrap_or_default(),
                        TextButton::new("choose-dir", strings.change, Variant::Primary).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.choose_download_dir(cx)),
                        ),
                    ),
                ),
            ),
        )
        .block(settings_divider(theme))
        .block(engine_group(this, strings, theme, window, cx))
        .block(settings_divider(theme))
        .block(
            settings_group(
                theme,
                strings.section_appearance,
                strings.appearance_summary,
            )
            .child(setting_row(
                theme,
                strings.theme,
                strings.theme_note,
                segmented(theme, "theme", theme_options),
            ))
            .child(setting_row(
                theme,
                strings.font,
                strings.font_note,
                font_box(this, strings, theme, window, cx),
            ))
            .when(this.font_menu_open, |element| {
                element.child(font_picker(this, strings, theme, window, cx))
            })
            .when_some(using, |element, family| {
                element.child(hint(format!("{}{family}", strings.font_using), muted))
            })
            .when(!missing.is_empty(), |element| {
                element.child(hint(
                    format!("{}{}", strings.font_missing, missing.join(", ")),
                    theme.danger,
                ))
            }),
        )
        .block(settings_divider(theme))
        .block(
            settings_group(theme, strings.section_language, strings.language_summary).child(
                setting_row(
                    theme,
                    strings.language,
                    "",
                    segmented(theme, "language", language_options),
                ),
            ),
        )
        .block(settings_divider(theme))
        .block(
            settings_group(theme, strings.section_data, strings.data_summary)
                .child(setting_row(
                    theme,
                    strings.database,
                    strings.records(records.0, records.1),
                    path_row(
                        theme,
                        this.store.path().to_string_lossy().into_owned(),
                        TextButton::new("reveal-db", strings.open_folder, Variant::Secondary)
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.reveal_store_dir(cx)
                                }),
                            ),
                    ),
                ))
                .when_some(error, |element, error| {
                    element.child(hint(
                        format!("{}: {error}", strings.storage_disabled),
                        theme.danger,
                    ))
                }),
        )
        .action(
            TextButton::new("settings-cancel", strings.cancel, Variant::Secondary).on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.cancel_settings(window, cx)),
            ),
        )
        .action(
            TextButton::new("settings-save", strings.save, Variant::Primary).on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.commit_settings(window, cx)),
            ),
        )
        .build(theme, window)
}

/// A path that cannot be edited, beside the one button that acts on it.
///
/// Deliberately not a boxed field: nothing here can be typed, and a box would promise otherwise.
/// Plain text also lets the two "where things live" rows share one shape instead of two.
fn path_row(theme: &Theme, value: String, action: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(12.0))
        .child(path_text(theme, value))
        .child(action)
}

fn path_text(theme: &Theme, value: String) -> impl IntoElement {
    div()
        .flex_none()
        .max_w(px(PATH_WIDTH))
        .truncate()
        .text_size(px(12.5))
        .text_color(theme.text_muted)
        .child(value)
}

/// The engine group: user agent, connections, proxy, then the concurrency tier.
///
/// Split out of [`settings_dialog`] so each stays readable; it owns the rows, not the card chrome.
fn engine_group(
    this: &NexusApp,
    strings: &'static Strings,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    let faint = theme.text_faint;

    let connections: Vec<_> = CONNECTIONS
        .iter()
        .map(|&(label, value)| {
            (
                label,
                this.active_settings().connections == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_connections(value, cx)),
            )
        })
        .collect();
    let concurrent: Vec<_> = MAX_CONCURRENT
        .iter()
        .map(|&(label, value)| {
            (
                label,
                this.active_settings().max_concurrent == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_max_concurrent(value, cx)),
            )
        })
        .collect();
    let speed: Vec<_> = speed_options(strings)
        .iter()
        .map(|&(label, value)| {
            (
                label,
                this.active_settings().speed_limit == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_speed_limit(value, cx)),
            )
        })
        .collect();
    let tries: Vec<_> = [(strings.unlimited, 0u32), ("3", 3), ("5", 5), ("10", 10)]
        .iter()
        .map(|&(label, value)| {
            (
                label,
                this.active_settings().max_tries == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_max_tries(value, cx)),
            )
        })
        .collect();
    let timeout: Vec<_> = TIMEOUTS
        .iter()
        .map(|&(label, value)| {
            (
                label,
                this.active_settings().timeout == value,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_timeout(value, cx)),
            )
        })
        .collect();

    settings_group(theme, strings.section_engine, strings.engine_summary)
        .child(setting_row(
            theme,
            strings.user_agent,
            strings.user_agent_note,
            text_field::control_box(
                this,
                theme,
                window,
                cx,
                "engine-user-agent",
                Field::UserAgent,
                strings.user_agent_placeholder,
                CONTROL,
            ),
        ))
        .child(setting_row(
            theme,
            strings.connections,
            strings.connections_note,
            segmented(theme, "engine-connections", connections),
        ))
        .child(setting_row(
            theme,
            strings.proxy,
            strings.proxy_note,
            text_field::control_box(
                this,
                theme,
                window,
                cx,
                "engine-proxy",
                Field::Proxy,
                strings.proxy_placeholder,
                CONTROL,
            ),
        ))
        .child(settings_subheading(theme, strings.subsection_concurrency))
        .child(setting_row(
            theme,
            strings.max_concurrent,
            strings.max_concurrent_note,
            segmented(theme, "engine-concurrent", concurrent),
        ))
        .child(setting_row(
            theme,
            strings.speed_limit,
            strings.speed_limit_note,
            segmented(theme, "engine-speed", speed),
        ))
        .child(setting_row(
            theme,
            strings.max_tries,
            strings.max_tries_note,
            segmented(theme, "engine-tries", tries),
        ))
        .child(setting_row(
            theme,
            strings.timeout,
            strings.timeout_note,
            segmented(theme, "engine-timeout", timeout),
        ))
        .child(hint(strings.engine_note, faint))
}

/// The font box: the family list, plus the button that opens the picker.
fn font_box(
    this: &NexusApp,
    strings: &'static Strings,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    let (accent, text_muted) = (theme.accent, theme.text_muted);
    let open = this.font_menu_open;
    let font = this.ui_font(window);

    let input = text_field::field(
        text_field::FieldSpec {
            id: "font-input",
            target: Field::Font,
            edit: &this.font_draft,
            focus: &this.font_focus,
            placeholder: strings.font_placeholder,
            caret_on: this.caret_on,
            metrics: text_field::Metrics {
                size: text_field::size(Field::Font),
                height: CONTROL_HEIGHT,
                radius: 10.0,
                padding: 11.0,
                chrome: chrome_for(window, FONT_TEXT),
                gap: 8.0,
                bg: theme.surface_hover,
                width: text_field::Width::Fill,
            },
            leading: None,
            trailing: None,
        },
        theme,
        &font,
        window,
        cx,
    );

    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .w(px(CONTROL))
        .gap(px(CONTROL_GAP))
        .child(input)
        .child(
            IconButton::new(
                "font-add",
                0,
                "icons/plus.svg",
                if open { accent } else { text_muted },
                if open { accent } else { text_muted },
            )
            .hover_bg(theme.surface_hover)
            .active(open)
            .outlined()
            .box_size(CONTROL_HEIGHT)
            .radius(10.0)
            .glyph_size(14.0)
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.toggle_font_menu(window, cx)),
            ),
        )
}

/// The picker: a filter box over every family the platform reports, one row each. Clicking a row
/// appends that family to the list in the box above.
fn font_picker(
    this: &NexusApp,
    strings: &'static Strings,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    let matches = this.font_matches();
    let chosen: Vec<String> = font_stack(this.font_draft.text());
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
        .max_h(px(176.0))
        .overflow_y_scroll()
        .children(rows.into_iter().map(|family| {
            let picked = chosen.iter().any(|name| name == family.as_ref());
            font_option(family, picked, theme, cx)
        }));

    let faint = theme.text_faint;
    let font = this.ui_font(window);

    let search = text_field::field(
        text_field::FieldSpec {
            id: "font-search",
            target: Field::FontSearch,
            edit: &this.font_query,
            focus: &this.search_focus,
            placeholder: strings.font_search_placeholder,
            caret_on: this.caret_on,
            metrics: text_field::Metrics {
                size: text_field::size(Field::FontSearch),
                height: 28.0,
                radius: 8.0,
                padding: 9.0,
                chrome: chrome_for(window, PICKER_TEXT),
                gap: 8.0,
                bg: theme.surface_hover,
                width: text_field::Width::Auto,
            },
            leading: None,
            trailing: None,
        },
        theme,
        &font,
        window,
        cx,
    );

    div().flex().flex_row().flex_none().justify_end().child(
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(FONT_PICKER))
            .gap(px(8.0))
            .p(px(9.0))
            .rounded(px(12.0))
            .bg(theme.bg)
            .border_1()
            .border_color(theme.border)
            .child(search)
            .child(list)
            .when(empty, |element| {
                element.child(hint(strings.font_no_match, faint))
            }),
    )
}

/// One entry in the picker, drawn in its own face so the list doubles as a preview. One already in
/// the list is ticked and does nothing, so it cannot be added twice.
fn font_option(
    family: SharedString,
    picked: bool,
    theme: &Theme,
    cx: &mut Context<NexusApp>,
) -> AnyElement {
    let (text, hover, accent) = (theme.text, theme.surface_hover, theme.accent);
    let label = family.clone();
    let value = family.to_string();

    div()
        .id(family)
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .h(px(28.0))
        .px(px(8.0))
        .rounded(px(7.0))
        .cursor_pointer()
        .text_size(px(12.5))
        .text_color(text)
        .hover(move |style| style.bg(hover))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.choose_font(&value, cx)))
        .child(div().truncate().font_family(label.clone()).child(label))
        .when(picked, |element| {
            element.child(icon("icons/check.svg", 13.0, accent))
        })
        .into_any_element()
}
