//! Controller battery widget — a small always-on-top card showing the battery
//! level of a connected game controller.

// Release builds must not open a console window; debug builds keep it so that
// panics and logs stay visible while developing.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod autohide;
mod autostart;
mod collector;
mod i18n;
mod icon_glyph;
mod menu;
mod settings;
mod single;
mod tray;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // Keep the guard alive for the whole run: dropping it releases the mutex.
    let instance = single::acquire();
    if instance.is_none() {
        // Another instance is running and has been asked to show its card.
        return Ok(());
    }
    let _instance = instance;

    let settings = settings::Settings::load();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([app::WIDTH, app::EMPTY_HEIGHT])
        .with_resizable(true)
        .with_decorations(false)
        .with_transparent(true)
        .with_window_level(settings.window_level.to_egui())
        .with_taskbar(false);

    if let (Some(x), Some(y)) = (settings.window_x, settings.window_y) {
        if settings::is_position_visible(x, y, app::WIDTH, app::EMPTY_HEIGHT) {
            viewport = viewport.with_position([x, y]);
        }
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
