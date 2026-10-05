//! Icons are compiled into the binary, so Nexus is a single self-contained executable.
//!
//! GPUI rasterises an SVG into an alpha mask and tints it with the element's
//! `text_color`, so every icon here is monochrome by design.
//!
//! This is only the set Nexus needs *and* `nexus-look` does not ship — the general-purpose glyphs
//! (alert, folder, trash, close, gear, plus …) live in the library and are referenced as
//! `nexus_look::icons::*`. Anything both crates would own is a duplicate waiting to diverge, so
//! the rule is: if the library draws it, the library ships it.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

macro_rules! icons {
    ($($file:literal),* $(,)?) => {
        pub const ICONS: &[(&str, &[u8])] = &[
            $(($file, include_bytes!(concat!("../assets/", $file))),)*
        ];
    };
}

icons![
    "icons/bolt.svg",
    "icons/download.svg",
    "icons/logo.svg",
    "icons/pause.svg",
    "icons/play.svg",
    "icons/retry.svg",
    "icons/sliders.svg",
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}
