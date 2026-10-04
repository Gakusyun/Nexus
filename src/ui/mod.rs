//! The app's own view pieces: the task row's controls, its filter pill, its progress rail and
//! its state badge — the shapes Nexus-look does not ship because no other project would want
//! them. Every widget reads its colours from the `Theme` global, so call sites only supply data
//! and behaviour.
//!
//! Anything two screens share — a card, a table row, a button, a field — lives in `nexus_look`,
//! and this module may not grow a second copy of it. See `STYLE.md` in the library's repository.

mod root;
mod settings;
mod task_row;

use gpui::prelude::*;
use gpui::{
    App, Background, ClickEvent, ColorExt, ElementId, Rgba, SharedString, Svg, Window,
    WindowControlArea, div, px, relative, rgba, svg,
};

use crate::i18n::Strings;
use crate::model::Status;
use nexus_look::{CONTROL, Theme};

/// An icon, tinted explicitly.
///
/// The tint is a parameter rather than something you set on the parent because a `Svg`
/// **does not inherit `text_color`**: GPUI builds the style an `Svg` paints with from
/// `Style::default()` plus that element's own refinements (`Interactivity::compute_style_internal`),
/// never from the surrounding `Div`. And `Svg::paint` only draws when its own
/// `style.text.color` is `Some`, so an icon whose parent carried the colour is silently
/// skipped — no warning, no error, just an invisible button. Passing the colour here makes
/// that mistake impossible.
///
/// Hover tints layer on top with `.group_hover(group, ..)` against a parent `.group(group)`.
pub fn icon(path: impl Into<SharedString>, size: f32, tint: Rgba) -> Svg {
    svg().path(path).size(px(size)).flex_none().text_color(tint)
}

pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// The one square icon button: the title bar's gear and window controls, the task rows' actions
/// and the font picker's "+" are all this widget. The axis each call site actually varies —
/// size, tint, hitbox — is a builder method; the rest is fixed so the copies cannot drift.
///
/// Window controls have no handler: they pass a [`WindowControlArea`] and the platform owns the
/// click.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    group: SharedString,
    glyph: SharedString,
    glyph_size: f32,
    box_size: f32,
    radius: f32,
    tint: Rgba,
    hover_tint: Rgba,
    hover_bg: Rgba,
    /// Washed in the accent while its toggle is on.
    active: bool,
    /// Draws the hairline border that marks a standalone control.
    outlined: bool,
    /// When set, the platform — not a click handler — resolves the hitbox.
    area: Option<WindowControlArea>,
    handler: Option<ClickHandler>,
}

impl IconButton {
    /// `name` and `seq` identify both the element and its hover group, so no two buttons may
    /// share a pair. The group exists because the hover tint has to be applied to the `Svg`
    /// itself — see [`icon`] — and `group_hover` needs a name to match against.
    pub fn new(
        name: &'static str,
        seq: u64,
        glyph: &'static str,
        tint: Rgba,
        hover_tint: Rgba,
    ) -> Self {
        Self {
            id: (name, seq).into(),
            group: SharedString::from(format!("{name}-{seq}")),
            glyph: glyph.into(),
            glyph_size: 15.0,
            box_size: CONTROL,
            radius: 10.0,
            tint,
            hover_tint,
            hover_bg: rgba(0x00000000),
            active: false,
            outlined: false,
            area: None,
            handler: None,
        }
    }

    pub fn hover_bg(mut self, color: Rgba) -> Self {
        self.hover_bg = color;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.handler = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let Self {
            id,
            group,
            glyph,
            glyph_size,
            box_size,
            radius,
            tint,
            hover_tint,
            hover_bg,
            active,
            outlined,
            area,
            handler,
        } = self;
        let border = if active {
            theme.accent.opacity(0.5)
        } else {
            theme.border
        };
        let active_bg = theme.accent_wash;

        div()
            .id(id)
            .group(group.clone())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(box_size))
            .rounded(px(radius))
            .when(active, |element| element.bg(active_bg))
            .when(outlined, |element| element.border_1().border_color(border))
            .cursor_pointer()
            .hover(move |style| style.bg(hover_bg))
            .when_some(area, |element, area| element.window_control_area(area))
            .when_some(handler, |element, handler| element.on_click(handler))
            .child(
                icon(glyph, glyph_size, tint)
                    .group_hover(group, move |style| style.text_color(hover_tint)),
            )
    }
}

/// A filter pill with an item count.
#[derive(IntoElement)]
pub struct Chip {
    id: ElementId,
    label: SharedString,
    count: usize,
    selected: bool,
    handler: ClickHandler,
}

impl Chip {
    pub fn new(
        id: impl Into<ElementId>,
        label: &'static str,
        count: usize,
        selected: bool,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            count,
            selected,
            handler: Box::new(handler),
        }
    }
}

impl RenderOnce for Chip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let (text, text_hover, accent) = (theme.text_faint, theme.text_muted, theme.accent);
        let (border, surface_hover) = (theme.border, theme.surface_hover);
        let wash = theme.accent_wash;

        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(7.0))
            .h(px(CONTROL))
            .px(px(12.0))
            .rounded_full()
            .border_1()
            .cursor_pointer()
            .text_size(px(12.5))
            .when(self.selected, |element| {
                element
                    .bg(wash)
                    .border_color(accent.opacity(0.4))
                    .text_color(accent)
            })
            .when(!self.selected, |element| {
                element
                    .border_color(rgba(0x00000000))
                    .text_color(text)
                    .hover(move |style| {
                        style
                            .bg(surface_hover)
                            .text_color(text_hover)
                            .border_color(border)
                    })
            })
            .child(self.label)
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.0))
                    .opacity(0.75)
                    .child(self.count.to_string()),
            )
            .on_click(self.handler)
    }
}

/// The thin progress rail under each task.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Accent,
    Success,
    Danger,
}

#[derive(IntoElement)]
pub struct ProgressBar {
    progress: Option<f32>,
    tone: Tone,
    height: f32,
}

impl ProgressBar {
    pub fn new(progress: Option<f32>, tone: Tone) -> Self {
        Self {
            progress,
            tone,
            height: 5.0,
        }
    }
}

impl RenderOnce for ProgressBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let fill: Background = match self.tone {
            Tone::Accent => theme.accent.into(),
            Tone::Success => theme.success.into(),
            Tone::Danger => theme.danger.into(),
        };
        // `None` means the total size is not known yet (metadata is still arriving), so
        // there is nothing honest to draw: the track stays empty rather than showing a
        // decorative sliver that reads as progress. The row's own label says
        // "Fetching metadata…" and the percentage reads "—" until numbers arrive.
        let width = self
            .progress
            .map(|value| value.clamp(0.0, 1.0))
            .unwrap_or(0.0);

        div()
            .flex_none()
            .w_full()
            .h(px(self.height))
            .rounded_full()
            .bg(theme.border_soft)
            .overflow_hidden()
            .when(width > 0.0, |element| {
                element.child(div().h_full().w(relative(width)).rounded_full().bg(fill))
            })
    }
}

/// A state pill: coloured dot plus label.
#[derive(IntoElement)]
pub struct StatusBadge {
    status: Status,
    strings: &'static Strings,
}

impl StatusBadge {
    pub fn new(status: Status, strings: &'static Strings) -> Self {
        Self { status, strings }
    }
}

impl RenderOnce for StatusBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let (tint, wash) = match self.status {
            Status::Active => (theme.accent, theme.accent_wash),
            Status::Complete => (theme.success, theme.success_wash),
            Status::Error => (theme.danger, theme.danger_wash),
            Status::Waiting | Status::Paused => (theme.text_muted, theme.surface_hover),
        };
        let label = self.status.label(self.strings);

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .h(px(21.0))
            .px(px(9.0))
            .rounded_full()
            .bg(wash)
            .text_color(tint)
            .text_size(px(11.0))
            .child(div().flex_none().size(px(5.0)).rounded_full().bg(tint))
            .child(label)
    }
}

/// Connections-per-download presets. aria2 caps `--max-connection-per-server` at 16, so there is
/// no point offering more. Shared by the engine settings and the add-download dialog.
pub const CONNECTIONS: [(&str, u32); 5] = [("1", 1), ("2", 2), ("4", 4), ("8", 8), ("16", 16)];
/// Speed-limit presets in bytes per second; `0` is unlimited. Used for both the engine's overall
/// limit and a single download's limit, so the two pickers offer the same choices.
pub fn speed_options(strings: &Strings) -> [(&'static str, u64); 5] {
    [
        (strings.unlimited, 0),
        ("1 MB/s", 1_048_576),
        ("5 MB/s", 5_242_880),
        ("10 MB/s", 10_485_760),
        ("50 MB/s", 52_428_800),
    ]
}
