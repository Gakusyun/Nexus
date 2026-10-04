//! The one editable text field every box in the app is built from.
//!
//! GPUI-CE ships no text input element, and the two halves that are easy to get wrong live here
//! so the URL bar, the font box and the picker's filter cannot each get them wrong differently:
//!
//! * **The platform side.** `Window::handle_input` with an `EntityInputHandler` is the only path
//!   IME and `WM_CHAR` take. Without it the Windows backend drops every composed character — the
//!   user types Chinese and literally nothing arrives — and `key_char` from `WM_KEYDOWN` is the
//!   only text that ever reaches the app.
//! * **The caret arithmetic.** Turning a click into a byte offset needs the same font and the
//!   same leftward scroll the line was drawn with, or the caret lands beside the character the
//!   user pointed at.
//!
//! `handle_input` asserts it runs during paint, so the platform registration is a `canvas`
//! overlaid on the field rather than a handler on the field's own `div`.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, ClipboardItem, ColorExt, Context, DispatchPhase, ElementInputHandler,
    Entity, FocusHandle, Font, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Rgba, SharedString, TextRun, Window, canvas, div, point, px,
};

use crate::state::{Field, NexusApp};
use nexus_look::TextEdit;
use nexus_look::widgets::text_edit::one_line;
use nexus_look::Theme;

/// What a keystroke meant, so each field can decide what Enter or Escape does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// The field consumed it: caret movement, editing, clipboard.
    Handled,
    /// Enter — the caller's "go" key.
    Submit,
    /// Escape or Tab — let go of the field, or close the layer that owns it.
    Dismiss,
    Ignored,
}

/// Apply a keystroke to a field.
///
/// **Printable characters are deliberately not inserted here.** The platform delivers them
/// through `ElementInputHandler::replace_text_in_range` — Windows sends plain ASCII typing as
/// `WM_CHAR`, not only IME composition — so also inserting `key_char` would double every letter.
pub fn apply_key(edit: &mut TextEdit, event: &KeyDownEvent, cx: &mut App) -> Outcome {
    let modifiers = event.keystroke.modifiers;
    let key = event.keystroke.key.as_str();

    if modifiers.control || modifiers.platform {
        return match key {
            "a" => {
                edit.select_all();
                Outcome::Handled
            }
            "c" => {
                if let Some(text) = edit.selected_text() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                Outcome::Handled
            }
            "x" => {
                if let Some(text) = edit.cut() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                Outcome::Handled
            }
            "v" => {
                if let Some(pasted) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    edit.insert(&one_line(&pasted));
                }
                Outcome::Handled
            }
            _ => Outcome::Ignored,
        };
    }

    let extend = modifiers.shift;
    match key {
        "backspace" => edit.backspace(),
        "delete" => edit.delete_forward(),
        "left" => edit.move_left(extend),
        "right" => edit.move_right(extend),
        "home" => edit.move_home(extend),
        "end" => edit.move_end(extend),
        "enter" => return Outcome::Submit,
        "escape" | "tab" => return Outcome::Dismiss,
        _ => return Outcome::Ignored,
    }
    Outcome::Handled
}

/// The colours a field draws with, so call sites do not thread the whole theme through.
#[derive(Clone, Copy)]
pub struct Ink {
    pub text: Rgba,
    pub faint: Rgba,
    pub accent: Rgba,
    pub selection: Rgba,
}

/// How wide the text is allowed to be before it has to scroll, from the window the field lives
/// in. Mirrors the flex layout, so it is an estimate; the only thing that depends on it is the
/// caret's scroll offset, and erring small keeps the caret visible.
fn available(window: &Window, chrome: f32) -> f32 {
    (window.viewport_size().width.as_f32() - chrome).max(60.0)
}

/// How far a line must slide left for the caret to stay in view. The caller supplies the size and
/// the font it draws with, because an element's own text style is not on the window during
/// `render` — only the window default is.
pub fn caret_shift(
    text: &str,
    cursor: usize,
    size: f32,
    window: &Window,
    font: &Font,
    available: f32,
) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let Some(line) = shape(text, size, window, font) else {
        return 0.0;
    };
    let prefix = line.split_at(cursor.min(text.len())).0.width().as_f32();
    (prefix + CARET_WIDTH - available).max(0.0)
}

/// The width of the caret bar. One number, shared with the IME's range maths, so the quad the
/// platform is told about is exactly the quad drawn.
const CARET_WIDTH: f32 = 1.5;

/// How tall the caret is for a font of `size` — a touch taller than the em box, the way a text
/// cursor reads next to glyphs of any script.
fn caret_height(size: f32) -> f32 {
    (size * 1.2).max(12.0)
}

/// Where the caret sits along the line, in pixels from the text's own left edge. Shaped whole,
/// exactly as hit-testing and the IME do it, so the quad lands on the same glyph boundary a click
/// would.
pub fn caret_offset(text: &str, cursor: usize, size: f32, window: &Window, font: &Font) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    match shape(text, size, window, font) {
        Some(line) => line.split_at(cursor.min(text.len())).0.width().as_f32(),
        None => 0.0,
    }
}

/// The caret itself: an overlay, never a flex child.
///
/// A `div` in the row pushes every glyph after it sideways each time it disappears and comes back
/// — the line twitches on every blink, and the gap between the characters on either side changes.
/// Positioned absolutely at the measured glyph boundary it cannot take part in layout at all; the
/// row keeps its height from the placeholder, so centring it is a plain `items_center` rather than
/// a guessed offset.
fn caret(x: f32, size: f32, ink: Ink) -> impl IntoElement + use<> {
    div()
        .absolute()
        .left(px(x))
        .top_0()
        .bottom_0()
        .w(px(CARET_WIDTH))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(px(CARET_WIDTH))
                .h(px(caret_height(size)))
                .rounded_full()
                .bg(ink.accent),
        )
}

/// Shape one line with an explicit font. Used for hit-testing and for the IME's caret query, both
/// of which run outside `render` and so cannot rely on the window's inherited style.
pub fn shape(text: &str, size: f32, window: &Window, font: &Font) -> Option<gpui::ShapedLine> {
    let run = TextRun {
        len: text.len(),
        font: font.clone(),
        ..Default::default()
    };
    Some(window.text_system().shape_line(
        SharedString::from(text.to_string()),
        px(size),
        std::slice::from_ref(&run),
        None,
    ))
}

/// The inside of a field: the text, the selection behind it, the IME composition underlined, and
/// the caret. `shift` is how far the line is slid left so the caret stays in view, and `caret_at`
/// is `Some` with the distance along the line where the caret belongs, or `None` while it blinks
/// off. Both are measured by the caller, which is where the font and the window live.
fn contents(
    edit: &TextEdit,
    shift: f32,
    caret_at: Option<f32>,
    size: f32,
    focused: bool,
    placeholder: &'static str,
    ink: Ink,
) -> impl IntoElement + use<> {
    let mut row = div()
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .flex_none()
        .ml(px(-shift))
        .text_size(px(size))
        .text_color(ink.text);

    if edit.is_empty() {
        // The placeholder is the app's filler, so it steps aside the moment the field is in use.
        // That is the whole reason a click on an empty box should look like it cleared — but it
        // keeps holding the row's height while hidden, or an empty focused box would collapse to
        // nothing and take the caret's centring with it.
        row = row.child(
            div()
                .text_color(if focused {
                    ink.faint.opacity(0.0)
                } else {
                    ink.faint
                })
                .child(placeholder),
        );
        return match caret_at {
            Some(x) => row.child(caret(x, size, ink)),
            None => row,
        };
    }

    let text = edit.text();
    let len = edit.len();
    let selection = edit.selection();
    let marked = edit.marked();
    // Split at every edge that changes how a run is drawn, so each piece is a plain span with one
    // background and one decoration.
    let mut edges = vec![0, len];
    for range in [selection.as_ref(), marked.as_ref()].into_iter().flatten() {
        edges.push(range.start);
        edges.push(range.end);
    }
    edges.sort_unstable();
    edges.dedup();

    for pair in edges.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start == end {
            continue;
        }
        let selected = selection
            .as_ref()
            .is_some_and(|range| start >= range.start && end <= range.end);
        let composing = marked
            .as_ref()
            .is_some_and(|range| start >= range.start && end <= range.end);
        row = row.child(
            div()
                .flex_none()
                .when(selected, |span| span.bg(ink.selection))
                .when(composing, |span| span.underline())
                .child(SharedString::from(text[start..end].to_string())),
        );
    }

    // Last, so it paints over the glyph it sits beside; out of flow, so the segments around it
    // keep exactly the spacing they have while it is hidden.
    if let Some(x) = caret_at {
        row = row.child(caret(x, size, ink));
    }

    row
}

/// Register a field with the platform and with the mouse.
///
/// Must be laid out during paint (`handle_input` asserts as much), which is why this is a `canvas`
/// overlay rather than a plain handler on the field's `div` — only an `Element` is handed its own
/// bounds, and the text input handler needs them to put the IME candidate window in the right
/// place. The parent must be `relative`, and the canvas covers it edge to edge.
fn surface(
    field: Field,
    focus: FocusHandle,
    entity: Entity<NexusApp>,
    shift: f32,
) -> impl IntoElement + use<> {
    let tapped = focus.clone();
    canvas(
        move |_bounds, _window, _cx| {},
        move |bounds, (), window, app| {
            if focus.is_focused(window) {
                window.handle_input(
                    &focus,
                    ElementInputHandler::new(bounds, entity.clone()),
                    app,
                );
            }
            mouse(field, tapped.clone(), entity.clone(), bounds, shift, window);
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// A click puts the caret where it landed and starts a selection; a drag with the button held
/// extends it; letting go ends it.
fn mouse(
    field: Field,
    focus: FocusHandle,
    entity: Entity<NexusApp>,
    bounds: Bounds<Pixels>,
    shift: f32,
    window: &mut Window,
) {
    // The offset along the line that a pointer at `x` is over: the text is drawn `shift` pixels
    // to the left, so put that back before measuring.
    let along = {
        let origin = bounds.origin.x - px(shift);
        move |x: Pixels| (x - origin).as_f32()
    };

    window.on_mouse_event({
        let entity = entity.clone();
        let focus = focus.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !bounds.contains(&event.position)
            {
                return;
            }
            let along = along(event.position.x);
            entity.update(cx, |this, cx| {
                window.focus(&focus, cx);
                this.press_field(field, along, window, cx);
            });
        }
    });

    window.on_mouse_event({
        let entity = entity.clone();
        move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.pressed_button != Some(MouseButton::Left)
                || !bounds.contains(&event.position)
            {
                return;
            }
            let along = along(event.position.x);
            entity.update(cx, |this, cx| this.drag_field(field, along, window, cx));
        }
    });

    window.on_mouse_event(move |_: &MouseUpEvent, phase, _window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        entity.update(cx, |this, _| this.dragging = None);
    });
}

pub fn size(field: Field) -> f32 {
    match field {
        Field::Font | Field::FontSearch | Field::UserAgent | Field::Proxy | Field::Add(_) => 12.5,
    }
}

/// A single caret quad's width, so hit-test helpers and the IME agree with what is drawn.
pub fn caret_width() -> Pixels {
    px(CARET_WIDTH)
}

/// `Bounds` for a byte range, in window coordinates. Used by the input handler to tell the IME
/// where the caret is, so its candidate window appears next to the text.
pub fn range_bounds(
    line: &gpui::ShapedLine,
    bytes: std::ops::Range<usize>,
    element: Bounds<Pixels>,
    shift: f32,
    height: Pixels,
) -> Bounds<Pixels> {
    let start = line.split_at(bytes.start).0.width();
    let end = line.split_at(bytes.end).0.width();
    let origin = element.origin - point(px(shift), px(0.0));
    Bounds::new(
        origin + point(start, px(0.0)),
        gpui::size((end - start).max(caret_width()), height),
    )
}

/// How a box takes its width.
///
/// The layout differs per screen — the link bar is alone on its row, the settings boxes share one
/// with a button — and nothing else about a field does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Width {
    /// The whole row.
    Full,
    /// Alongside a trailing sibling: grow, but never past it.
    Fill,
    /// Whatever the column gives.
    Auto,
}

/// The per-screen numbers and colours. Everything else about a field is fixed and lives in
/// [`field`].
#[derive(Clone, Copy)]
pub struct Metrics {
    pub size: f32,
    pub height: f32,
    pub radius: f32,
    pub padding: f32,
    /// Room the surrounding layout eats, so the caret knows when to scroll the line. Erring small
    /// only ever keeps the caret visible.
    pub chrome: f32,
    pub gap: f32,
    pub bg: Rgba,
    pub width: Width,
}

/// What a call site decides about one box: which buffer, which handle, and how it looks. Every
/// other part of a field is fixed and lives in [`field`].
pub struct FieldSpec<'a> {
    /// Needed for the hitbox, so the box can be clicked as a whole rather than only on its text.
    pub id: &'static str,
    pub target: Field,
    pub edit: &'a TextEdit,
    pub focus: &'a FocusHandle,
    pub placeholder: &'static str,
    pub caret_on: bool,
    pub metrics: Metrics,
    pub leading: Option<AnyElement>,
    pub trailing: Option<AnyElement>,
}

/// One box, from its chrome to its platform wiring.
///
/// Every screen used to assemble this by hand out of `contents`, `surface` and half a dozen raw
/// divs, and the copies drifted — the settings page, for one, never called `track_focus`, and a
/// box that does not track its handle still *looks* focused (the caret and the border only ask
/// `is_focused`) while owning no node in the dispatch tree. Keys aimed at it were dispatched from
/// the window root instead and dropped, which is a bug that reads as "backspace does nothing" and
/// "escape does not release the box".
///
/// `contents` and `surface` are private for the same reason: assembling a field by hand is how
/// that happened, so now there is exactly one way.
pub fn field(
    spec: FieldSpec<'_>,
    theme: &Theme,
    font: &Font,
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    let FieldSpec {
        id,
        target,
        edit,
        focus,
        placeholder,
        caret_on,
        metrics,
        leading,
        trailing,
    } = spec;
    let focused = focus.is_focused(window);
    let shift = caret_shift(
        edit.text(),
        edit.cursor(),
        metrics.size,
        window,
        font,
        available(window, metrics.chrome),
    );
    // The caret only exists while the box holds focus and the blink is on; measuring it here is
    // what lets `contents` treat it as a pure overlay.
    let caret_at = (focused && caret_on)
        .then(|| caret_offset(edit.text(), edit.cursor(), metrics.size, window, font));
    let activation = focus.clone();
    let dismiss = focus.clone();

    div()
        .id(id)
        // The handle has to be tracked here rather than at the call site: this is the element the
        // caret is measured in and the one the platform sends the input to.
        .track_focus(focus)
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(metrics.gap))
        .h(px(metrics.height))
        .px(px(metrics.padding))
        .rounded(px(metrics.radius))
        .bg(metrics.bg)
        .border_1()
        .border_color(if focused {
            theme.accent.opacity(0.5)
        } else {
            theme.border
        })
        // The text cursor over the whole box: the bar is what the user aims at, while the caret
        // still lands on the character they pointed at (see `surface`).
        .cursor_text()
        .when(metrics.width == Width::Full, |element| {
            element.w_full().flex_none()
        })
        .when(metrics.width == Width::Fill, |element| {
            element.flex_1().min_w(px(0.0))
        })
        .when(metrics.width == Width::Auto, |element| element.flex_none())
        // Clicking outside the box drops its focus. This is the framework's intended idiom for
        // "clicked elsewhere": `on_mouse_down_out` runs in the **capture** phase, before the
        // box's own focus handlers, so a click that *does* land in a box re-focuses it in the
        // same dispatch. A handler on the root would not work — every mouse handler, element and
        // window-level alike, shares one listener list that is dispatched in reverse registration
        // order, so an ancestor's handler runs *last* and would undo the focus.
        .on_mouse_down_out(cx.listener(move |_, _: &MouseDownEvent, window, cx| {
            if dismiss.is_focused(window) {
                window.blur();
                cx.notify();
            }
        }))
        // Clicking the chrome — the padding, the icon — focuses the box; only the text row knows
        // where inside it the pointer landed.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_, _: &MouseDownEvent, window, cx| {
                window.focus(&activation, cx);
                cx.notify();
            }),
        )
        .children(leading)
        .child(
            div()
                .relative()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_row()
                .items_center()
                .overflow_hidden()
                .child(contents(
                    edit,
                    shift,
                    caret_at,
                    metrics.size,
                    focused,
                    placeholder,
                    ink(theme),
                ))
                .child(surface(target, focus.clone(), cx.entity(), shift)),
        )
        .children(trailing)
}

/// What a [`card`] box loses between the caller's content edge and the text it can hold: a 12px
/// padding a side plus a 1px border. Callers subtract this from their column width to get the
/// `text_width` they hand back.
pub const CARD_INSET: f32 = 26.0;

/// A box for a settings sheet or the add-download dialog: the height, radius, padding, gap and font
/// size are fixed here, so no two screens can drift. Only the width, background and the **text
/// width** differ.
///
/// `text_width` is how much text the caller's layout leaves for the box, not the derived chrome:
/// converting it is this module's job ([`super::chrome_for`]), because getting that conversion wrong
/// is the one mistake a caller can make here and it is invisible — the caret simply scrolls the line
/// at the wrong moment.
pub struct CardSpec<'a> {
    pub id: &'static str,
    pub target: Field,
    pub edit: &'a TextEdit,
    pub focus: &'a FocusHandle,
    pub placeholder: &'static str,
    pub caret_on: bool,
    pub text_width: f32,
    pub bg: Rgba,
    pub width: Width,
}

pub fn card(
    spec: CardSpec<'_>,
    theme: &Theme,
    font: &Font,
    window: &Window,
    cx: &mut Context<NexusApp>,
) -> impl IntoElement + use<> {
    let CardSpec {
        id,
        target,
        edit,
        focus,
        placeholder,
        caret_on,
        text_width,
        bg,
        width,
    } = spec;
    field(
        FieldSpec {
            id,
            target,
            edit,
            focus,
            placeholder,
            caret_on,
            metrics: Metrics {
                size: size(target),
                height: super::CONTROL_HEIGHT,
                radius: 10.0,
                padding: 12.0,
                chrome: super::chrome_for(window, text_width),
                gap: 8.0,
                bg,
                width,
            },
            leading: None,
            trailing: None,
        },
        theme,
        font,
        window,
        cx,
    )
}

/// A box in a modal row's control column: captionless (the row owns the caption and the note) and
/// `width` wide, so every row's control ends on the same right edge.
///
/// This is the only way a modal builds a text box. The settings sheet and the new-download dialog
/// both come through here, which is why a UA field in one is literally the same widget as a UA field
/// in the other.
#[allow(clippy::too_many_arguments)]
pub fn control_box(
    this: &NexusApp,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<NexusApp>,
    id: &'static str,
    field: Field,
    placeholder: &'static str,
    width: f32,
) -> gpui::Div {
    let (Some(edit), Some(focus)) = (this.field(field), this.focus_handle(field)) else {
        return div();
    };
    let font = this.ui_font(window);
    div().w(px(width)).child(card(
        CardSpec {
            id,
            target: field,
            edit,
            focus,
            placeholder,
            caret_on: this.caret_on,
            text_width: width - CARD_INSET,
            bg: theme.surface_hover,
            width: Width::Full,
        },
        theme,
        &font,
        window,
        cx,
    ))
}

/// The colours every text field draws with.
fn ink(theme: &Theme) -> Ink {
    Ink {
        text: theme.text,
        faint: theme.text_faint,
        accent: theme.accent,
        selection: theme.accent.opacity(0.24),
    }
}
