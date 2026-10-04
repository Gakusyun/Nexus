//! Nexus visual language: one dark palette, resolved once and stored as a GPUI global.

use gpui::{
    App, Background, ColorExt, Global, Rgba, linear_color_stop, linear_gradient, rgb, rgba,
};

/// Every colour the UI is allowed to use. Keeping them in one place is what makes the
/// app feel coherent instead of assembled from ad-hoc literals.
#[derive(Clone)]
pub struct Theme {
    /// Window backdrop.
    pub bg: Rgba,
    /// Raised surfaces (cards, the command bar).
    pub surface: Rgba,
    /// Surface under the cursor.
    pub surface_hover: Rgba,
    /// Hairline borders.
    pub border: Rgba,
    /// Even quieter borders, for inner dividers.
    pub border_soft: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub text_faint: Rgba,
    pub accent: Rgba,
    /// Accent at low alpha, for badge backgrounds.
    pub accent_wash: Rgba,
    pub success: Rgba,
    pub success_wash: Rgba,
    pub danger: Rgba,
    pub danger_wash: Rgba,
    pub warning: Rgba,
    /// Second stop of the progress-bar gradient.
    pub cyan: Rgba,
    /// Backdrop behind a modal: the window colour, mostly opaque.
    pub scrim: Rgba,
}

impl Global for Theme {}

impl Theme {
    pub fn dark() -> Self {
        Self {
            bg: rgb(0x0a0b0e),
            surface: rgb(0x13161b),
            surface_hover: rgb(0x191d24),
            border: rgb(0x232833),
            border_soft: rgb(0x1a1e26),
            text: rgb(0xeceef3),
            text_muted: rgb(0x99a1af),
            text_faint: rgb(0x5c6472),
            accent: rgb(0x7c5cff),
            accent_wash: rgb(0x7c5cff).opacity(0.14),
            success: rgb(0x3dd68c),
            success_wash: rgb(0x3dd68c).opacity(0.13),
            danger: rgb(0xf2555f),
            danger_wash: rgb(0xf2555f).opacity(0.13),
            warning: rgb(0xf0b429),
            cyan: rgb(0x3dd9eb),
            scrim: rgba(0x07080bc4),
        }
    }

    /// The same design in daylight: the accent is darkened so it still carries contrast
    /// against white, and the washes stay light.
    pub fn light() -> Self {
        Self {
            bg: rgb(0xf6f7f9),
            surface: rgb(0xffffff),
            surface_hover: rgb(0xeef0f4),
            border: rgb(0xd9dee7),
            border_soft: rgb(0xe7eaf0),
            text: rgb(0x16181d),
            text_muted: rgb(0x5c6472),
            text_faint: rgb(0x8b93a1),
            accent: rgb(0x5b3ce0),
            accent_wash: rgb(0x5b3ce0).opacity(0.1),
            success: rgb(0x118a58),
            success_wash: rgb(0x118a58).opacity(0.1),
            danger: rgb(0xcc2b36),
            danger_wash: rgb(0xcc2b36).opacity(0.09),
            warning: rgb(0xa97400),
            cyan: rgb(0x0e93a8),
            scrim: rgba(0x1b1e26b4),
        }
    }

    pub fn for_mode(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }

    /// The signature Nexus sweep, used on progress fills and the logo plate.
    pub fn sweep(&self) -> Background {
        linear_gradient(
            90.0,
            linear_color_stop(self.accent, 0.0),
            linear_color_stop(self.cyan, 1.0),
        )
    }

    pub fn of(cx: &App) -> &Theme {
        cx.global::<Theme>()
    }
}

pub fn init(cx: &mut App) {
    cx.set_global(Theme::dark());
}
