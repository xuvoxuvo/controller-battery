//! The floating card UI.
//!
//! One compact row per connected controller; the window grows vertically as
//! devices come and go. The card outline is a superellipse (a "squircle")
//! rather than a plain rounded rectangle, which avoids the hard corner
//! transition that reads as aliasing at high DPI.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, Vec2};

use controller_battery_core::model::{BatteryReading, ChargeState};

use crate::collector::{self, DeviceState, ErrorKind, Shared};
use crate::i18n::{self, Language, Resolved, Strings};
use crate::menu;
use crate::settings::Settings;
use crate::tray;

const WIDTH: f32 = 230.0;
const PAD: f32 = 10.0;
const ROW_HEIGHT: f32 = 48.0;
const EMPTY_HEIGHT: f32 = PAD * 2.0 + ROW_HEIGHT;
/// Show at most this many rows; further devices are summarised.
const MAX_ROWS: usize = 5;
const CORNER_RADIUS: f32 = 16.0;

pub struct App {
    shared: Arc<Mutex<Shared>>,
    settings: Settings,
    /// Settings already applied to the UI/tray, so a change is noticed once.
    applied_language: Language,
    applied_auto_hide: bool,
    last_saved: Option<egui::Pos2>,
    window_height: f32,
    /// eframe's window handle, needed to hide the card from the context menu.
    window: Option<isize>,
    /// Hiding runs after the frame, not in the middle of it.
    hide_requested: bool,
    /// The context menu is a native Win32 popup; open it after the frame.
    menu_requested: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: Settings) -> Self {
        i18n::set(settings.language);
        install_fonts(&cc.egui_ctx, i18n::resolved());

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::TRANSPARENT;
        visuals.window_fill = Color32::TRANSPARENT;
        cc.egui_ctx.set_visuals(visuals);

        let applied_language = settings.language;
        let applied_auto_hide = settings.auto_hide;

        let shared = Arc::new(Mutex::new(Shared::default()));
        collector::spawn(shared.clone());
        let window = tray::spawn(cc, shared.clone());
        tray::set_auto_hide(applied_auto_hide);

        Self {
            shared,
            settings,
            applied_language,
            applied_auto_hide,
            last_saved: None,
            window_height: EMPTY_HEIGHT,
            window,
            hide_requested: false,
            menu_requested: false,
        }
    }
}

fn install_fonts(ctx: &egui::Context, language: Resolved) {
    // Windows ships a few CJK fonts; load one at runtime instead of embedding
    // ~10 MB into the binary. A fallback is loaded for every language because
    // device names come from Windows and may be Chinese even in the English UI.
    const SIMPLIFIED: [&str; 4] = [
        "C:/Windows/Fonts/Deng.ttf",
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simhei.ttf",
        "C:/Windows/Fonts/simsun.ttc",
    ];
    const TRADITIONAL: [&str; 4] = [
        "C:/Windows/Fonts/msjh.ttc",
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/Deng.ttf",
        "C:/Windows/Fonts/simsun.ttc",
    ];

    let candidates = match language {
        Resolved::TraditionalChinese => TRADITIONAL,
        _ => SIMPLIFIED,
    };

    for path in candidates {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mut fonts = egui::FontDefinitions::default();
        fonts
            .font_data
            .insert("cjk".to_owned(), egui::FontData::from_owned(bytes));
        // Append as a fallback so Latin text keeps the default font and only
        // CJK glyphs come from the system font.
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("cjk".to_owned());
        }
        ctx.set_fonts(fonts);
        return;
    }
}

impl eframe::App for App {
    /// Clear with full transparency so the card keeps its corners.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    /// Remove the tray icon before the process goes away.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        tray::shutdown();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(250));

        let devices = collector::snapshot(&self.shared);
        let error = self.shared.lock().unwrap().error;
        let countdown = tray::countdown();
        let strings = i18n::current();

        // While the auto-hide countdown runs, the card shows the "no controller"
        // screen instead of grey idle rows with a countdown, so a disconnected
        // controller never sits around in a faded row.
        let show_empty = countdown.is_some() && !devices.is_empty();
        let row_devices: &[(String, DeviceState)] = if show_empty {
            &devices[..0]
        } else {
            &devices[..]
        };
        let visible = if show_empty {
            1
        } else {
            devices.len().clamp(1, MAX_ROWS)
        };
        let overflow = if show_empty {
            0
        } else {
            devices.len().saturating_sub(MAX_ROWS)
        };
        // The empty state uses the same height as a single row so that a
        // controller sleeping and waking does not resize the window.
        let height =
            PAD * 2.0 + visible as f32 * ROW_HEIGHT + if overflow > 0 { 18.0 } else { 0.0 };

        // Compare against the real window size: the compositor may ignore or
        // clamp a request, and a stale field would stop us from retrying.
        let actual = ctx.input(|i| i.viewport().inner_rect).map(|r| r.height());
        if actual.is_none_or(|h| (h - height).abs() > 0.5) {
            self.window_height = height;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(WIDTH, height)));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::TRANSPARENT))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());

                // Touching the card cancels the auto-hide countdown and aborts a
                // fade that already started, instead of waiting for the next
                // one-second timer tick.
                if response.clicked()
                    || response.secondary_clicked()
                    || response.drag_started()
                    || response.dragged()
                {
                    tray::note_user_interaction();
                }

                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if response.drag_stopped() {
                    self.persist_position(ctx);
                }

                if response.secondary_clicked() {
                    self.menu_requested = true;
                }

                paint_card(ui, rect, row_devices, error, overflow, countdown, strings);
            });

        // A native popup must open after the frame is painted, or the card
        // would freeze half-drawn behind the menu's modal loop.
        if std::mem::take(&mut self.menu_requested) {
            if let Some(window) = self.window {
                match menu::show(window, &self.settings, tray::available()) {
                    Some(menu::Command::ToggleAutoHide) => {
                        self.settings.auto_hide = !self.settings.auto_hide;
                    }
                    Some(menu::Command::SetLanguage(language)) => {
                        self.settings.language = language;
                    }
                    Some(menu::Command::HideToTray) => self.hide_requested = true,
                    Some(menu::Command::Quit) => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    None => {}
                }
            }
        }

        // Hide after the frame is painted, not in the middle of it: the card
        // keeps its last content and the tray icon brings it back.
        if std::mem::take(&mut self.hide_requested) {
            if let Some(window) = self.window {
                if tray::hide(window) {
                    tray::hint_hidden();
                }
            }
        }

        // Settings changed from the context menu during this frame: apply them
        // once, persist, and let the next frame use the new strings.
        if self.settings.language != self.applied_language {
            self.applied_language = self.settings.language;
            i18n::set(self.applied_language);
            install_fonts(ctx, i18n::resolved());
            self.settings.save();
            ctx.request_repaint();
        }
        if self.settings.auto_hide != self.applied_auto_hide {
            self.applied_auto_hide = self.settings.auto_hide;
            tray::set_auto_hide(self.applied_auto_hide);
            self.settings.save();
        }

        if let Some(pos) = ctx.input(|i| i.viewport().outer_rect).map(|r| r.min) {
            if self.last_saved != Some(pos) {
                self.settings.window_x = Some(pos.x);
                self.settings.window_y = Some(pos.y);
                self.settings.save();
                self.last_saved = Some(pos);
            }
        }
    }
}

impl App {
    fn persist_position(&self, ctx: &egui::Context) {
        if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            let mut settings = self.settings.clone();
            settings.window_x = Some(rect.min.x);
            settings.window_y = Some(rect.min.y);
            settings.save();
        }
    }
}

/// Points of a superellipse-based rounded rectangle.
///
/// Unlike a circular arc, the curvature ramps up gradually towards the corner
/// (the "continuous curvature" look used by Apple), and the polygon has enough
/// segments that the tessellator's anti-aliasing stays smooth at 200% DPI.
fn squircle_points(rect: Rect, radius: f32) -> Vec<Pos2> {
    const EXPONENT: f32 = 4.0;
    const STEPS_PER_CORNER: usize = 24;

    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let (left, top, right, bottom) = (rect.left(), rect.top(), rect.right(), rect.bottom());

    // (corner centre x, corner centre y, start angle in degrees)
    let corners = [
        (left + r, top + r, 180.0_f32),
        (right - r, top + r, 270.0),
        (right - r, bottom - r, 0.0),
        (left + r, bottom - r, 90.0),
    ];

    let mut points = Vec::with_capacity(4 * (STEPS_PER_CORNER + 1));
    for (cx, cy, start) in corners {
        for step in 0..=STEPS_PER_CORNER {
            let degrees = start + 90.0 * (step as f32 / STEPS_PER_CORNER as f32);
            let radians = degrees.to_radians();
            let (sin, cos) = radians.sin_cos();
            let x = cx + r * cos.signum() * cos.abs().powf(2.0 / EXPONENT);
            let y = cy + r * sin.signum() * sin.abs().powf(2.0 / EXPONENT);
            points.push(egui::pos2(x, y));
        }
    }
    points
}

fn paint_card(
    ui: &egui::Ui,
    rect: Rect,
    devices: &[(String, DeviceState)],
    error: Option<ErrorKind>,
    overflow: usize,
    countdown: Option<u8>,
    strings: &Strings,
) {
    let painter = ui.painter();
    // Inset by half a point so the outline stroke is not clipped by the window
    // edge, which is what produced the hard staircase before.
    let shape_rect = rect.shrink(0.5);
    let outline = squircle_points(shape_rect, CORNER_RADIUS);
    painter.add(egui::Shape::convex_polygon(
        outline,
        Color32::from_rgb(22, 22, 26),
        Stroke::new(0.8_f32, Color32::from_rgb(56, 58, 66)),
    ));

    if devices.is_empty() {
        // The countdown wins over a stale error: the card is about to hide and
        // should say why, not show an old read failure.
        let (title, hint) = if let Some(seconds) = countdown {
            let title = if seconds == 0 {
                strings.no_device_hiding.to_string()
            } else {
                strings.no_device_countdown(seconds)
            };
            (title, strings.no_device_hint.to_string())
        } else if let Some(kind) = error {
            (
                strings.read_failed.to_string(),
                strings.error(kind).to_string(),
            )
        } else {
            (
                strings.no_device.to_string(),
                strings.no_device_hint.to_string(),
            )
        };
        let max_width = rect.width() - 32.0;
        paint_fitted(
            painter,
            egui::pos2(rect.left() + 16.0, rect.top() + 14.0),
            Align2::LEFT_TOP,
            &title,
            &[14.0, 13.0, 12.0, 11.0, 10.0],
            max_width,
            Color32::from_rgb(150, 156, 165),
        );
        paint_fitted(
            painter,
            egui::pos2(rect.left() + 16.0, rect.top() + 46.0),
            Align2::LEFT_TOP,
            &hint,
            &[11.0, 10.0, 9.5],
            max_width,
            Color32::from_rgb(110, 116, 128),
        );
        return;
    }

    for (index, (_key, state)) in devices.iter().take(MAX_ROWS).enumerate() {
        let top = rect.top() + PAD + index as f32 * ROW_HEIGHT;
        paint_row(painter, rect, top, state, countdown, strings);

        if index + 1 < devices.len().min(MAX_ROWS) {
            let y = top + ROW_HEIGHT - 0.5;
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 16.0, y),
                    egui::pos2(rect.right() - 16.0, y),
                ],
                Stroke::new(1.0_f32, Color32::from_rgb(34, 36, 42)),
            );
        }
    }

    if overflow > 0 {
        painter.text(
            egui::pos2(rect.left() + 16.0, rect.bottom() - 16.0),
            Align2::LEFT_CENTER,
            strings.more_devices(overflow),
            FontId::proportional(9.5),
            Color32::from_rgb(110, 116, 128),
        );
    }
}

/// The countdown replaces the row's "idle" text, so no extra strip is needed.
fn countdown_text(seconds: u8, strings: &Strings) -> String {
    if seconds == 0 {
        strings.hiding_now.to_string()
    } else {
        strings.hide_countdown(seconds)
    }
}

/// Draw `text`, shrinking the font until it fits `max_width` (and adding an
/// ellipsis only if even the smallest size does not). English labels are much
/// wider than the Chinese ones, and the card is only 230 px wide.
fn paint_fitted(
    painter: &egui::Painter,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    sizes: &[f32],
    max_width: f32,
    color: Color32,
) {
    for &size in sizes {
        let font = FontId::proportional(size);
        let width = painter
            .layout_no_wrap(text.to_owned(), font.clone(), color)
            .size()
            .x;
        if width <= max_width {
            painter.text(pos, anchor, text, font, color);
            return;
        }
    }
    let size = *sizes.last().unwrap_or(&10.0);
    let font = FontId::proportional(size);
    let fitted = fit_width(painter, text, &font, max_width);
    painter.text(pos, anchor, fitted, font, color);
}

fn paint_row(
    painter: &egui::Painter,
    card: Rect,
    top: f32,
    state: &DeviceState,
    countdown: Option<u8>,
    strings: &Strings,
) {
    let reading: &BatteryReading = &state.reading;
    let stale = state.is_stale();
    let degraded = state.degraded;
    let muted = Color32::from_rgb(150, 156, 165);

    // The name owns the first line; trim it with an ellipsis only when the
    // measured width would overflow the card.
    let name_font = FontId::proportional(10.0);
    let name = fit_width(painter, &reading.device, &name_font, card.width() - 32.0);
    painter.text(
        egui::pos2(card.left() + 16.0, top + 3.0),
        Align2::LEFT_TOP,
        name,
        name_font,
        muted,
    );

    let percent_text = match reading.percent {
        Some(p) => format!("{p}%"),
        None => "--".to_string(),
    };
    let percent_text = if degraded {
        format!("≈{percent_text}")
    } else {
        percent_text
    };
    painter.text(
        egui::pos2(card.left() + 16.0, top + 18.0),
        Align2::LEFT_TOP,
        percent_text,
        FontId::proportional(16.0),
        if stale || degraded {
            muted
        } else {
            Color32::from_rgb(240, 242, 246)
        },
    );

    // While the card is idle, the countdown takes the place of the "idle"
    // label instead of adding another row of chrome.
    let core_label = if stale {
        match countdown {
            Some(seconds) => countdown_text(seconds, strings),
            None => strings.idle.to_string(),
        }
    } else if degraded {
        strings.unknown.to_string()
    } else {
        strings.charge_state(reading.state).to_string()
    };
    let state_label = if reading.transport == controller_battery_core::Transport::Unknown {
        core_label
    } else {
        format!("{} · {core_label}", strings.transport(reading.transport))
    };
    // Right-aligned, but never wide enough to run into the percentage.
    let state_label = fit_width(
        painter,
        &state_label,
        &FontId::proportional(9.5),
        card.width() - 70.0,
    );
    let accent = accent_color(reading, degraded, stale);
    painter.text(
        egui::pos2(card.right() - 16.0, top + 30.0),
        Align2::RIGHT_CENTER,
        &state_label,
        FontId::proportional(9.5),
        if stale || degraded { muted } else { accent },
    );

    let bar = Rect::from_min_size(
        egui::pos2(card.left() + 16.0, top + 42.0),
        Vec2::new(card.width() - 32.0, 3.0),
    );
    painter.rect_filled(bar, 1.5, Color32::from_rgb(58, 58, 66));
    let fraction = reading.percent.unwrap_or(0) as f32 / 100.0;
    if fraction > 0.0 {
        let filled = Rect::from_min_size(bar.min, Vec2::new(bar.width() * fraction, bar.height()));
        painter.rect_filled(filled, 1.5, accent);
    }
}

/// Shorten `text` with an ellipsis until it fits `max_width`.
fn fit_width(painter: &egui::Painter, text: &str, font: &FontId, max_width: f32) -> String {
    let width = painter
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x;
    if width <= max_width {
        return text.to_owned();
    }

    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let mut candidate: String = chars.iter().collect();
        candidate.push('…');
        let width = painter
            .layout_no_wrap(candidate.clone(), font.clone(), Color32::WHITE)
            .size()
            .x;
        if width <= max_width {
            return candidate;
        }
    }
    "…".to_string()
}

fn accent_color(reading: &BatteryReading, degraded: bool, stale: bool) -> Color32 {
    if stale || degraded {
        return Color32::from_rgb(107, 114, 128);
    }
    match reading.state {
        ChargeState::Charging => Color32::from_rgb(56, 189, 248),
        ChargeState::Full => Color32::from_rgb(52, 211, 153),
        _ => match reading.percent.unwrap_or(0) {
            p if p >= 50 => Color32::from_rgb(52, 211, 153),
            p if p >= 20 => Color32::from_rgb(251, 191, 36),
            _ => Color32::from_rgb(248, 113, 113),
        },
    }
}
