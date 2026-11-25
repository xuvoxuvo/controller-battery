//! Controller battery widget — a small always-on-top card showing the battery
//! level of a connected game controller.

// Release builds must not open a console window; debug builds keep it so that
// panics and logs stay visible while developing.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod autohide;
mod collector;
mod i18n;
mod icon_glyph;
mod settings;
mod tray;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let settings = settings::Settings::load();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([230.0, 92.0])
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false);

    if let (Some(x), Some(y)) = (settings.window_x, settings.window_y) {
        viewport = viewport.with_position([x, y]);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Controller Battery",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, settings)))),
    )
}
