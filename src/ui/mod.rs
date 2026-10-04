//! Reusable view pieces. Every widget reads its colours from the `Theme` global, so
//! call sites only supply data and behaviour.

mod root;
mod settings;
mod task_row;
pub(crate) mod text_field;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Background, ClickEvent, ColorExt, ElementId, FontWeight, Rgba, SharedString,
    Svg, Window, WindowControlArea, div, linear_color_stop, linear_gradient, px, relative, rgb,
    rgba, svg,
};

use crate::i18n::Strings;
use crate::model::Status;
use nexus_look::Theme;

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
            box_size: CONTROL_HEIGHT,
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

    pub fn glyph_size(mut self, size: f32) -> Self {
        self.glyph_size = size;
        self
    }

    pub fn box_size(mut self, size: f32) -> Self {
        self.box_size = size;
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    pub fn hover_bg(mut self, color: Rgba) -> Self {
        self.hover_bg = color;
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn outlined(mut self) -> Self {
        self.outlined = true;
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

/// How a [`TextButton`] is weighted. Every labelled button in the app is one of these, so the
/// command bar's call to action, the settings page and the confirm dialog cannot drift apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    /// Accent-washed: the affirmative action ("Done", "Change…").
    Primary,
    /// Bordered and neutral: the default ("Open", "Cancel").
    Secondary,
    /// Borderless and faint: an inline action such as "Clear finished".
    Ghost,
    /// Outlined in red, and filled red on hover: the destructive choice.
    Danger,
    /// The command bar's gradient call to action, disabled until there is something to add.
    Hero,
}

/// A labelled button, optionally with a leading icon.
#[derive(IntoElement)]
pub struct TextButton {
    id: ElementId,
    label: SharedString,
    variant: Variant,
    leading: Option<(SharedString, f32)>,
    enabled: bool,
    width: Option<f32>,
    handler: Option<ClickHandler>,
}

impl TextButton {
    pub fn new(id: impl Into<ElementId>, label: &'static str, variant: Variant) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            variant,
            leading: None,
            enabled: true,
            width: None,
            handler: None,
        }
    }

    /// A glyph drawn before the label, tinted like the label.
    pub fn leading(mut self, glyph: &'static str, size: f32) -> Self {
        self.leading = Some((glyph.into(), size));
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
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

impl RenderOnce for TextButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let Self {
            id,
            label,
            variant,
            leading,
            enabled,
            width,
            handler,
        } = self;
        let hero = variant == Variant::Hero;

        let fill: Background = match (variant, enabled) {
            (Variant::Hero, false) => theme.surface_hover.into(),
            (Variant::Hero, true) => theme.accent.into(),
            (Variant::Primary, _) => theme.accent_wash.into(),
            (Variant::Danger, _) => theme.danger_wash.into(),
            _ => rgba(0x00000000).into(),
        };
        let (tint, border, hover_bg, hover_tint) = match variant {
            Variant::Primary => (
                theme.accent,
                Some(theme.accent),
                theme.surface_hover,
                theme.accent,
            ),
            Variant::Secondary => (
                theme.text_muted,
                Some(theme.border),
                theme.surface_hover,
                theme.text_muted,
            ),
            Variant::Ghost => (
                theme.text_faint,
                None,
                theme.surface_hover,
                theme.text_muted,
            ),
            Variant::Danger => (
                theme.danger,
                Some(theme.danger),
                theme.danger,
                rgb(0xffffff),
            ),
            Variant::Hero if enabled => (rgb(0xffffff), None, rgba(0x00000000), rgb(0xffffff)),
            Variant::Hero => (theme.text_faint, None, rgba(0x00000000), theme.text_faint),
        };

        let mut button = div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .h(px(if hero { 38.0 } else { CONTROL_HEIGHT }))
            .rounded(px(if hero { 11.0 } else { 10.0 }))
            .when_some(width, |element, width| element.w(px(width)))
            .when(!hero, |element| element.px(px(12.0)))
            .when_some(border, |element, border| {
                element.border_1().border_color(border)
            })
            .bg(fill)
            .text_size(px(if hero { 12.5 } else { 12.0 }))
            .font_weight(FontWeight::MEDIUM)
            .text_color(tint)
            .when(!hero, |element| {
                element.hover(move |style| style.bg(hover_bg).text_color(hover_tint))
            });

        if enabled {
            button = button
                .cursor_pointer()
                .when_some(handler, |element, handler| element.on_click(handler));
        } else {
            button = button.cursor_not_allowed();
        }
        if let Some((glyph, size)) = leading {
            button = button.child(icon(glyph, size, tint));
        }
        button.child(label)
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
            .h(px(CONTROL_HEIGHT))
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

/// A radio-style row of choices: one button per option, the active one washed in accent. Shared by
/// the settings page and the add-download dialog so a choice looks the same in both.
pub fn segmented(
    theme: &Theme,
    id: &'static str,
    options: Vec<(
        &'static str,
        bool,
        impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    )>,
) -> impl IntoElement {
    let (border, hover_bg, accent, wash, text, muted) = (
        theme.border,
        theme.surface_hover,
        theme.accent,
        theme.accent_wash,
        theme.text,
        theme.text_muted,
    );

    let mut row = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(4.0))
        .p(px(SEGMENT_PAD))
        .rounded(px(10.0))
        .border_1()
        .border_color(border);

    for (index, (label, selected, handler)) in options.into_iter().enumerate() {
        row = row.child(
            div()
                .id((id, index))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                // The options plus the container's padding and 1px borders add up to exactly
                // `CONTROL_HEIGHT`, so a picker and the box beside it share a top and bottom edge.
                .h(px(CONTROL_HEIGHT - SEGMENT_PAD * 2.0 - 2.0))
                .px(px(12.0))
                .rounded(px(7.0))
                .cursor_pointer()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if selected { accent } else { muted })
                .when(selected, |element| element.bg(wash))
                .hover(move |style| {
                    style
                        .bg(hover_bg)
                        .text_color(if selected { accent } else { text })
                })
                .on_click(handler)
                .child(label),
        );
    }

    row
}

/// A quiet line of help under a control.
pub fn hint(text: impl Into<SharedString>, faint: Rgba) -> impl IntoElement {
    div()
        .text_size(px(11.0))
        .text_color(faint)
        .child(text.into())
}

// ------------------------------------------------------------------- settings sheet
//
// A settings sheet is a flat list of groups separated by [`settings_divider`], never a card inside
// a card. That buys one column of labels down the left and one of controls down the right, which
// is what makes a long list scannable. See STYLE.md §6.

/// The hairline between two groups of a settings sheet.
pub fn settings_divider(theme: &Theme) -> impl IntoElement {
    div().flex_none().h(px(1.0)).w_full().bg(theme.border_soft)
}

/// A group of a settings sheet: a bold title, an optional one-line summary, then rows.
///
/// An empty summary is dropped, so a group whose rows already say what it is needs no summary.
pub fn settings_group(
    theme: &Theme,
    title: impl Into<SharedString>,
    summary: impl Into<SharedString>,
) -> gpui::Div {
    let (text, faint) = (theme.text, theme.text_faint);
    let summary: SharedString = summary.into();
    let has_summary = !summary.is_empty();
    div().flex().flex_col().flex_none().gap(px(13.0)).child(
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(2.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text)
                    .child(title.into()),
            )
            .when(has_summary, |element| {
                element.child(div().text_size(px(11.0)).text_color(faint).child(summary))
            }),
    )
}

/// The second tier of a group that outgrew one heading, like the engine's concurrency settings.
pub fn settings_subheading(theme: &Theme, title: &'static str) -> impl IntoElement {
    div()
        .pt(px(2.0))
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.text_muted)
        .child(title)
}

/// One row of a settings sheet: label and note on the left, the control on the right.
///
/// An empty note is dropped, so a label that speaks for itself stays on a single line. The control
/// is wrapped in a `flex_none` box so a wide control (a segmented picker, a field) never squeezes
/// the label column to nothing.
pub fn setting_row(
    theme: &Theme,
    label: impl Into<SharedString>,
    note: impl Into<SharedString>,
    control: impl IntoElement,
) -> impl IntoElement {
    let (text, faint) = (theme.text, theme.text_faint);
    let note: SharedString = note.into();
    let has_note = !note.is_empty();
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(20.0))
        .min_h(px(40.0))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(3.0))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(text)
                        .child(label.into()),
                )
                .when(has_note, |element| {
                    element.child(div().text_size(px(11.0)).text_color(faint).child(note))
                }),
        )
        .child(div().flex_none().child(control))
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

/// Turn "this box is N pixels wide" into the chrome figure `text_field` subtracts from the window
/// width. `render` cannot measure an element, so a fixed-width card derives the number from its
/// own layout instead. Erring small only ever keeps the caret visible.
pub fn chrome_for(window: &Window, field_width: f32) -> f32 {
    (window.viewport_size().width.as_f32() - field_width).max(0.0)
}

/// The width of the two "sheet" modals — settings and new download. They share one figure so
/// opening one after the other does not look like a layout bug.
pub const MODAL_WIDTH: f32 = 720.0;
/// Least width a sheet ever takes; the settings sheet's label/control rows stop fitting below it.
pub const MODAL_MIN_WIDTH: f32 = 640.0;
/// Space kept outside the card so it never touches the window edge.
const MODAL_MARGIN: f32 = 80.0;
/// The card's own horizontal padding, shared by every sheet.
pub const MODAL_PAD: f32 = 24.0;
/// Width of a modal row's control column: the right-hand side that every label, note and box lines
/// up against. The settings sheet and the new-download dialog both use it, which is what makes the
/// two read as the same table instead of a table plus a stack of forms.
pub const CONTROL: f32 = 320.0;
/// Gap between a box and the button that shares its row.
pub const CONTROL_GAP: f32 = 8.0;
/// The height every row-level control is built to — a text box, a segmented picker, a button, a
/// filter chip. One number, so nothing in a settings or dialog row can come out a couple of pixels
/// taller than the thing next to it. Larger controls (the command bar, a hero button, the title
/// bar's window buttons) are their own shapes and do not use it.
pub const CONTROL_HEIGHT: f32 = 31.0;
/// A segmented picker's inner padding; its options are sized from what is left of `CONTROL_HEIGHT`.
const SEGMENT_PAD: f32 = 3.0;

/// How wide a sheet actually gets. Fixed, but shrunk to fit a narrow window (the app allows 720
/// wide), so views must derive their field widths from this rather than from a constant:
/// `text_field` cannot measure a box at render time, and a stale width makes the caret scroll the
/// text at the wrong moment.
pub fn modal_width(window: &Window) -> f32 {
    (window.viewport_size().width.as_f32() - MODAL_MARGIN).clamp(MODAL_MIN_WIDTH, MODAL_WIDTH)
}

/// How tall a modal's scrolling body may get: the window minus the card's chrome (header, footer,
/// padding, gaps) and a margin top and bottom, so a card always floats instead of filling the
/// screen. The floor keeps the body usable on very short windows.
pub fn modal_body_max(window: &Window) -> f32 {
    // The card's own chrome (title + paddings + action row) is ~131, so anything beyond ~170 is
    // dead space that costs a row: at 660 high the new-download sheet needs 465 and used to be
    // capped at 420, which left its first row clipped.
    (window.viewport_size().height.as_f32() - 170.0).clamp(180.0, 620.0)
}

/// Space between a card's parts, and between the blocks inside its body.
pub const MODAL_GAP: f32 = 14.0;
/// Corner radius and title size of a modal card.
const MODAL_RADIUS: f32 = 16.0;
const MODAL_TITLE: f32 = 15.0;
/// Gap between neighbouring action buttons.
const MODAL_ACTION_GAP: f32 = 8.0;
/// Vertical padding, and the space above the action row's hairline.
const MODAL_PY: f32 = 16.0;
const MODAL_ACTION_TOP: f32 = 12.0;

/// The one modal card.
///
/// Every popup in the app — settings, new download, delete confirmation — is this skeleton: a scrim
/// that swallows clicks without closing, a card of the shared width, a title, a body, and a
/// right-aligned action row above a hairline. Views supply content and nothing else, which is what
/// keeps the three from drifting into three widths, paddings, backgrounds and button rows — exactly
/// what had happened.
///
/// ```ignore
/// Modal::new("add-card", strings.add_title)
///     .scrolling()
///     .block(uri_field)
///     .block(save_row)
///     .action(cancel)
///     .action(start)
///     .build(theme, window)
/// ```
///
/// The scrim has no listener on purpose: clicking beside a card only takes focus off a box, it does
/// not throw the card away. Closing is always an explicit action in the footer.
pub struct Modal {
    id: &'static str,
    title: SharedString,
    /// `None` uses the shared sheet width; `Some` overrides it — only the confirmation box, which is
    /// a one-line question and looks silly stretched to the sheet width.
    width: Option<f32>,
    /// Whether the body scrolls inside [`modal_body_max`] and carries the fade that says so.
    scrolling: bool,
    blocks: Vec<AnyElement>,
    actions: Vec<AnyElement>,
}

impl Modal {
    pub fn new(id: &'static str, title: impl Into<SharedString>) -> Self {
        Self {
            id,
            title: title.into(),
            width: None,
            scrolling: false,
            blocks: Vec::new(),
            actions: Vec::new(),
        }
    }

    /// A card narrower than the sheet width.
    pub fn narrow(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Let the body scroll. The fade is added here because gpui-ce 0.2.2 draws no scrollbar — see
    /// [`scroll_fade`].
    pub fn scrolling(mut self) -> Self {
        self.scrolling = true;
        self
    }

    /// One block of the body. The spacing between blocks is the skeleton's business, so callers
    /// never set a gap of their own.
    pub fn block(mut self, block: impl IntoElement) -> Self {
        self.blocks.push(block.into_any_element());
        self
    }

    /// One button. Secondary actions first, the main action last.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn build(self, theme: &Theme, window: &Window) -> impl IntoElement + use<> {
        let width = self.width.unwrap_or_else(|| modal_width(window));
        let blocks = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(MODAL_GAP))
            .children(self.blocks);
        let body = if self.scrolling {
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_none()
                .child(
                    div()
                        .id(ElementId::named_usize(self.id, 0))
                        .flex()
                        .flex_col()
                        .flex_none()
                        .max_h(px(modal_body_max(window)))
                        .pb(px(SCROLL_FADE))
                        .overflow_y_scroll()
                        .child(blocks),
                )
                .child(scroll_fade(theme.surface))
                .into_any_element()
        } else {
            blocks.into_any_element()
        };

        div()
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.scrim)
            .child(
                div()
                    .id(self.id)
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(px(MODAL_GAP))
                    .w(px(width))
                    .px(px(MODAL_PAD))
                    .py(px(MODAL_PY))
                    .rounded(px(MODAL_RADIUS))
                    .bg(theme.surface)
                    .border_1()
                    .border_color(theme.border)
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .text_size(px(MODAL_TITLE))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(self.title),
                    )
                    .child(body)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_none()
                            .items_center()
                            .justify_end()
                            .gap(px(MODAL_ACTION_GAP))
                            .border_t_1()
                            .border_color(theme.border_soft)
                            .pt(px(MODAL_ACTION_TOP))
                            .children(self.actions),
                    ),
            )
    }
}

/// Height of [`scroll_fade`] — and therefore the bottom padding every fading scroller carries.
pub const SCROLL_FADE: f32 = 18.0;

/// The gradient that frosts the bottom edge of a scrolling modal body, so the next section fades
/// out instead of being sliced in half.
///
/// gpui-ce 0.2.2 never draws a scrollbar: `scrollbar_width` only reserves layout space and already
/// defaults to `0`, and there is no `overflow_fade`. So this gradient is the only cue that there is
/// more to see. Pair it with `pb(px(SCROLL_FADE))` on the scroller — that padding is what the
/// gradient covers when the body is short or already scrolled to the end, so real content is never
/// faded.
///
/// The overlay carries no listener, cursor or hover group, which is what keeps it hitbox-free:
/// `should_insert_hitbox` returns false, so it cannot swallow the wheel events that belong to the
/// scroller underneath.
pub fn scroll_fade(bg: Rgba) -> impl IntoElement {
    div()
        .absolute()
        .bottom_0()
        .left_0()
        .w_full()
        .h(px(SCROLL_FADE))
        .bg(linear_gradient(
            180.0,
            linear_color_stop(bg.opacity(0.0), 0.0),
            linear_color_stop(bg, 1.0),
        ))
}
