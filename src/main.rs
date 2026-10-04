//! Nexus — a small, focused download manager for Windows.
//!
//! The UI is GPUI-CE; every byte on the wire is moved by a separate `aria2c.exe` that we
//! own, drive over JSON-RPC, and tear down with the app.

// Without this the binary is linked as a console application and Windows pops up a terminal
// window alongside the UI. It is not the download engine's console: `aria2c.exe` is spawned
// with `CREATE_NO_WINDOW`. The cost is that a panic has nowhere to print, so `install_log`
// writes one to disk instead.
#![windows_subsystem = "windows"]

mod aria2;
mod assets;
mod i18n;
mod model;
mod settings;
mod state;
mod store;
mod text_edit;
mod theme;
mod ui;

use std::io::Write;

use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

use crate::state::NexusApp;

fn main() {
    install_log();

    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            theme::init(cx);

            let bounds = Bounds::centered(None, size(px(960.0), px(660.0)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(720.0), px(460.0))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Nexus".into()),
                        // We draw our own title bar (see `ui::root::title_bar`).
                        appears_transparent: true,
                        traffic_light_position: None,
                    }),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| NexusApp::new(window, cx)),
            )
            .expect("failed to open the Nexus window");

            cx.activate(true);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        });
}

/// Record panics in `nexus.log` next to the working directory, since a windowed binary has no
/// stderr to show them on. Appended, so a crash after a few successful runs does not erase the
/// trail.
fn install_log() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let path = state::data_dir().join("nexus.log");
        if std::fs::create_dir_all(state::data_dir()).is_ok()
            && let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
        {
            let _ = writeln!(file, "panic at {info}");
        }
        previous(info);
    }));
}
