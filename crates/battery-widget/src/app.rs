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

use battery_core::model::{BatteryReading, ChargeState};

use crate::collector::{self, DeviceState, Shared};
use crate::settings::Settings;

const WIDTH: f32 = 230.0;
const PAD: f32 = 10.0;
const ROW_HEIGHT: f32 = 44.0;
const EMPTY_HEIGHT: f32 = 86.0;
/// Show at most this many rows; further devices are summarised.
const MAX_ROWS: usize = 5;
const CORNER_RADIUS: f32 = 16.0;

pub struct App {
    shared: Arc<Mutex<Shared>>,
    settings: Settings,
    last_saved: Option<egui::Pos2>,
    window_height: f32,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: Settings) -> Self {
        install_fonts(&cc.egui_ctx);

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::TRANSPARENT;
        visuals.window_fill = Color32::TRANSPARENT;
        cc.egui_ctx.set_visuals(visuals);

        let shared = Arc::new(Mutex::new(Shared::default()));
        collector::spawn(shared.clone());

        Self {
            shared,
            settings,
            last_saved: None,
            window_height: EMPTY_HEIGHT,
        }
    }
}

fn install_fonts(ctx: &egui::Context) {
    // Windows ships a few CJK fonts; load one at runtime instead of embedding
    // ~10 MB into the binary.
    const CANDIDATES: [&str; 4] = [
        "C:/Windows/Fonts/Deng.ttf",
        "C:/Windows/Fonts/simhei.ttf",
        "C:/Windows/Fonts/Dengb.ttf",
        "C:/Windows/Fonts/simsunb.ttf",
    ];

    for path in CANDIDATES {
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

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(250));

        let devices = collector::snapshot(&self.shared);
        let error = self.shared.lock().unwrap().error.clone();

        let visible = devices.len().min(MAX_ROWS);
        let overflow = devices.len().saturating_sub(MAX_ROWS);
        let height = if devices.is_empty() {
            EMPTY_HEIGHT
        } else {
            PAD * 2.0 + visible as f32 * ROW_HEIGHT + if overflow > 0 { 18.0 } else { 0.0 }
        };

        if (height - self.window_height).abs() > 0.5 {
            self.window_height = height;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(WIDTH, height)));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::TRANSPARENT))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());

                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if response.drag_stopped() {
                    self.persist_position(ctx);
                }

                response.context_menu(|ui| {
                    if ui.button("退出").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                paint_card(ui, rect, &devices, error.as_deref(), overflow);
            });

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
    error: Option<&str>,
    overflow: usize,
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
        let (title, hint) = match error {
            Some(message) => ("读取失败", message),
            None => ("未检测到手柄", "蓝牙配对或插 USB 线"),
        };
        painter.text(
            egui::pos2(rect.left() + 16.0, rect.top() + 14.0),
            Align2::LEFT_TOP,
            title,
            FontId::proportional(14.0),
            Color32::from_rgb(150, 156, 165),
        );
        painter.text(
            egui::pos2(rect.left() + 16.0, rect.top() + 46.0),
            Align2::LEFT_TOP,
            hint,
            FontId::proportional(11.0),
            Color32::from_rgb(110, 116, 128),
        );
        return;
    }

    for (index, (_key, state)) in devices.iter().take(MAX_ROWS).enumerate() {
        let top = rect.top() + PAD + index as f32 * ROW_HEIGHT;
        paint_row(painter, rect, top, state);

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
            format!("还有 {overflow} 个设备"),
            FontId::proportional(9.5),
            Color32::from_rgb(110, 116, 128),
        );
    }
}

fn paint_row(painter: &egui::Painter, card: Rect, top: f32, state: &DeviceState) {
    let reading: &BatteryReading = &state.reading;
    let stale = state.is_stale();
    let degraded = state.degraded;
    let muted = Color32::from_rgb(150, 156, 165);

    // Long product names (e.g. "8BitDo Ultimate 2 Wireless") would collide with
    // the percentage, so trim them and show the transport on the state line.
    let header = if reading.device.chars().count() > 18 {
        let trimmed: String = reading.device.chars().take(17).collect();
        format!("{trimmed}…")
    } else {
        reading.device.clone()
    };
    painter.text(
        egui::pos2(card.left() + 16.0, top + 5.0),
        Align2::LEFT_TOP,
        header,
        FontId::proportional(10.0),
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
        egui::pos2(card.right() - 16.0, top + 16.0),
        Align2::RIGHT_CENTER,
        percent_text,
        FontId::proportional(17.0),
        if stale || degraded {
            muted
        } else {
            Color32::from_rgb(240, 242, 246)
        },
    );

    let core_label = if stale {
        "空闲"
    } else if degraded {
        "状态未知"
    } else {
        reading.state.label()
    };
    let state_label = if reading.transport == battery_core::Transport::Unknown {
        core_label.to_string()
    } else {
        format!("{} · {core_label}", reading.transport.label())
    };
    let accent = accent_color(reading, degraded, stale);
    painter.text(
        egui::pos2(card.left() + 16.0, top + 30.0),
        Align2::LEFT_CENTER,
        &state_label,
        FontId::proportional(10.0),
        if stale || degraded { muted } else { accent },
    );

    let bar = Rect::from_min_size(
        egui::pos2(card.left() + 16.0, top + 38.0),
        Vec2::new(card.width() - 32.0, 3.0),
    );
    painter.rect_filled(bar, 1.5, Color32::from_rgb(58, 58, 66));
    let fraction = reading.percent.unwrap_or(0) as f32 / 100.0;
    if fraction > 0.0 {
        let filled = Rect::from_min_size(bar.min, Vec2::new(bar.width() * fraction, bar.height()));
        painter.rect_filled(filled, 1.5, accent);
    }
}

fn accent_color(reading: &BatteryReading, degraded: bool, stale: bool) -> Color32 {
    if stale || degraded {
        return Color32::from_rgb(107, 114, 128);
    }
    match reading.state {
        ChargeState::Charging => Color32::from_rgb(56, 189, 248),
        ChargeState::Full | ChargeState::Wired => Color32::from_rgb(52, 211, 153),
        _ => match reading.percent.unwrap_or(0) {
            p if p >= 50 => Color32::from_rgb(52, 211, 153),
            p if p >= 20 => Color32::from_rgb(251, 191, 36),
            _ => Color32::from_rgb(248, 113, 113),
        },
    }
}
