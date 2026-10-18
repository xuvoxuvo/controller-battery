//! The floating card UI.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use egui::{Align2, Color32, FontId, Rect, Rounding, Stroke, Vec2};

use battery_core::model::{BatteryReading, ChargeState, Freshness};

use crate::collector::{self, Shared};
use crate::settings::Settings;

const STALE_AFTER: Duration = Duration::from_secs(5);

pub struct App {
    shared: Arc<Mutex<Shared>>,
    settings: Settings,
    last_saved: Option<Vec2>,
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
            .insert("cjk".to_owned(), egui::FontData::from_owned(bytes).into());
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, "cjk".to_owned());
        }
        ctx.set_fonts(fonts);
        return;
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(250));

        let (reading, degraded, stale) = {
            let shared = self.shared.lock().unwrap();
            (
                shared.reading.clone(),
                shared.degraded,
                shared.is_stale(STALE_AFTER),
            )
        };

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

                paint_card(ui, rect, reading.as_ref(), degraded, stale);
            });

        // Remember the position occasionally, in case the window was moved by
        // the OS (multi-monitor changes, snapping, ...).
        if self.last_saved.is_none() {
            self.last_saved = ctx.input(|i| i.viewport().outer_rect).map(|r| r.min);
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

fn paint_card(ui: &egui::Ui, rect: Rect, reading: Option<&BatteryReading>, degraded: bool, stale: bool) {
    let painter = ui.painter();
    let bg = Color32::from_rgb(22, 22, 26);
    let border = Color32::from_rgb(46, 48, 54);

    painter.rect_filled(rect, Rounding::same(16.0), bg);
    painter.rect_stroke(rect, Rounding::same(16.0), Stroke::new(1.0, border));

    let Some(reading) = reading else {
        painter.text(
            egui::pos2(rect.left() + 14.0, rect.top() + 10.0),
            Align2::LEFT_TOP,
            "未检测到手柄",
            FontId::proportional(15.0),
            Color32::from_rgb(150, 156, 165),
        );
        painter.text(
            egui::pos2(rect.left() + 14.0, rect.top() + 48.0),
            Align2::LEFT_TOP,
            "蓝牙配对或插 USB 线",
            FontId::proportional(11.0),
            Color32::from_rgb(110, 116, 128),
        );
        return;
    };

    let accent = accent_color(reading, degraded, stale);
    let muted = Color32::from_rgb(150, 156, 165);

    // Header: device + transport.
    let header = format!("{} · {}", reading.device.to_uppercase(), reading.transport.label());
    painter.text(
        egui::pos2(rect.left() + 14.0, rect.top() + 9.0),
        Align2::LEFT_TOP,
        header,
        FontId::proportional(10.0),
        muted,
    );

    // Big percentage.
    let percent_text = format!("{}%", reading.percent.unwrap_or(0));
    let percent_text = if degraded { format!("≈{percent_text}") } else { percent_text };
    painter.text(
        egui::pos2(rect.left() + 12.0, rect.top() + 24.0),
        Align2::LEFT_TOP,
        percent_text,
        FontId::proportional(30.0),
        if stale || degraded { muted } else { Color32::from_rgb(240, 242, 246) },
    );

    // State on the right.
    let state_label = if stale {
        "空闲"
    } else if degraded {
        "状态未知"
    } else {
        reading.state.label()
    };
    painter.text(
        egui::pos2(rect.right() - 14.0, rect.top() + 40.0),
        Align2::RIGHT_CENTER,
        state_label,
        FontId::proportional(13.0),
        if stale || degraded { muted } else { accent },
    );

    // Battery bar.
    let bar = Rect::from_min_size(
        egui::pos2(rect.left() + 14.0, rect.bottom() - 16.0),
        Vec2::new(rect.width() - 28.0, 5.0),
    );
    painter.rect_filled(bar, Rounding::same(2.0), Color32::from_rgb(58, 58, 66));
    let fraction = reading.percent.unwrap_or(0) as f32 / 100.0;
    if fraction > 0.0 {
        let filled = Rect::from_min_size(bar.min, Vec2::new(bar.width() * fraction, bar.height()));
        painter.rect_filled(filled, Rounding::same(2.0), accent);
    }
}

fn accent_color(reading: &BatteryReading, degraded: bool, stale: bool) -> Color32 {
    if stale || degraded {
        return Color32::from_rgb(107, 114, 128);
    }
    match reading.state {
        ChargeState::Charging => Color32::from_rgb(56, 189, 248),
        ChargeState::Full | ChargeState::Wired => Color32::from_rgb(52, 211, 153),
        _ => {
            let pct = reading.percent.unwrap_or(0);
            if pct >= 50 {
                Color32::from_rgb(52, 211, 153)
            } else if pct >= 20 {
                Color32::from_rgb(251, 191, 36)
            } else {
                Color32::from_rgb(248, 113, 113)
            }
        }
    }
}

#[allow(dead_code)]
fn freshness_label(freshness: Freshness) -> &'static str {
    freshness.label()
}
