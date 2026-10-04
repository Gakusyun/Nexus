//! Icons are compiled into the binary, so Nexus is a single self-contained executable.
//!
//! GPUI rasterises an SVG into an alpha mask and tints it with the element's
//! `text_color`, so every icon here is monochrome by design.

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
    "icons/alert.svg",
    "icons/bolt.svg",
    "icons/check.svg",
    "icons/clock.svg",
    "icons/download.svg",
    "icons/folder.svg",
    "icons/gear.svg",
    "icons/link.svg",
    "icons/logo.svg",
    "icons/max.svg",
    "icons/min.svg",
    "icons/pause.svg",
    "icons/play.svg",
    "icons/plus.svg",
    "icons/restore.svg",
    "icons/retry.svg",
    "icons/sliders.svg",
    "icons/trash.svg",
    "icons/x.svg",
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
