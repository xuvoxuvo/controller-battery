//! Persistent UI settings.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::Language;

/// Window z-order preference, persisted in `settings.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WindowLevel {
    #[default]
    #[serde(rename = "top")]
    AlwaysOnTop,
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "bottom")]
    AlwaysOnBottom,
}

impl WindowLevel {
    pub const ALL: [WindowLevel; 3] = [
        WindowLevel::AlwaysOnTop,
        WindowLevel::Normal,
        WindowLevel::AlwaysOnBottom,
    ];

    pub fn to_egui(self) -> eframe::egui::WindowLevel {
        match self {
            Self::AlwaysOnTop => eframe::egui::WindowLevel::AlwaysOnTop,
            Self::Normal => eframe::egui::WindowLevel::Normal,
            Self::AlwaysOnBottom => eframe::egui::WindowLevel::AlwaysOnBottom,
        }
    }

    pub fn as_u8(self) -> u8 {
        match self {
            Self::AlwaysOnTop => 0,
            Self::Normal => 1,
            Self::AlwaysOnBottom => 2,
        }
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::AlwaysOnTop),
            1 => Some(Self::Normal),
            2 => Some(Self::AlwaysOnBottom),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub window_x: Option<f32>,
    pub window_y: Option<f32>,
    /// UI language; `System` follows the Windows UI language.
    pub language: Language,
    /// Hide the card when no controller is reporting.
    pub auto_hide: bool,
    /// Window z-order preference (top / normal / bottom).
    pub window_level: WindowLevel,
}

/// `%LOCALAPPDATA%\ControllerBattery` — settings and diagnostics live here.
pub fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let mut dir = PathBuf::from(base);
    dir.push("ControllerBattery");
    Some(dir)
}

fn config_path() -> Option<PathBuf> {
    Some(data_dir()?.join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) -> bool {
        let Some(path) = config_path() else {
            eprintln!("settings: cannot determine LOCALAPPDATA directory");
            return false;
        };
        if let Some(dir) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(dir) {
                eprintln!("settings: failed to create {}: {error}", dir.display());
                return false;
            }
        }
        let text = match serde_json::to_string_pretty(self) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("settings: failed to serialize: {error}");
                return false;
            }
        };
        if let Err(error) = std::fs::write(&path, text) {
            eprintln!("settings: failed to write {}: {error}", path.display());
            return false;
        }
        true
    }
}

/// Returns `true` when the saved card rectangle intersects at least one
/// currently attached monitor's work area. Checks each monitor with both its
/// own effective DPI and the primary monitor DPI so mixed-DPI multi-monitor
/// setups are not falsely reset.
#[cfg(windows)]
pub fn is_position_visible(x: f32, y: f32, width: f32, height: f32) -> bool {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetDC, GetDeviceCaps, GetMonitorInfoW, ReleaseDC, HDC, HMONITOR,
        LOGPIXELSX, MONITORINFO,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

    if !x.is_finite() || !y.is_finite() {
        return false;
    }

    // Require an inset portion of the card to overlap a monitor so a card
    // grazing 1 px off-screen is still reset to a safe default position.
    let inset_x = (width * 0.2).min(32.0);
    let inset_y = (height * 0.2).min(20.0);
    let card = (
        x + inset_x,
        y + inset_y,
        x + width - inset_x,
        y + height - inset_y,
    );

    unsafe {
        let hdc = GetDC(HWND::default());
        let primary_scale = if !hdc.is_invalid() {
            let dpi = GetDeviceCaps(hdc, LOGPIXELSX);
            let _ = ReleaseDC(HWND::default(), hdc);
            if dpi > 0 {
                dpi as f32 / 96.0
            } else {
                1.0
            }
        } else {
            1.0
        };

        struct State {
            card: (f32, f32, f32, f32),
            primary_scale: f32,
            visible: bool,
        }

        unsafe extern "system" fn callback(
            monitor: HMONITOR,
            _hdc: HDC,
            _clip: *mut RECT,
            lparam: LPARAM,
        ) -> BOOL {
            let state = &mut *(lparam.0 as *mut State);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return BOOL(1);
            }

            let mut dpi_x = 0u32;
            let mut dpi_y = 0u32;
            let monitor_scale =
                if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_ok()
                    && dpi_x > 0
                {
                    dpi_x as f32 / 96.0
                } else {
                    state.primary_scale
                };

            let work = info.rcWork;
            let (l, t, r, b) = state.card;
            for scale in [monitor_scale, state.primary_scale] {
                let pl = (l * scale).round() as i32;
                let pt = (t * scale).round() as i32;
                let pr = (r * scale).round() as i32;
                let pb = (b * scale).round() as i32;
                if pl < work.right && pr > work.left && pt < work.bottom && pb > work.top {
                    state.visible = true;
                    return BOOL(0);
                }
            }
            BOOL(1)
        }

        let mut state = State {
            card,
            primary_scale,
            visible: false,
        };
        let _ = EnumDisplayMonitors(
            HDC::default(),
            None,
            Some(callback),
            LPARAM(&mut state as *mut State as isize),
        );
        state.visible
    }
}

#[cfg(not(windows))]
pub fn is_position_visible(x: f32, y: f32, _width: f32, _height: f32) -> bool {
    x.is_finite() && y.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_files_still_load() {
        // Version 0.2 only stored the window position.
        let settings: Settings =
            serde_json::from_str(r#"{"window_x":12.0,"window_y":34.0}"#).unwrap();
        assert_eq!(settings.window_x, Some(12.0));
        assert_eq!(settings.window_y, Some(34.0));
        assert_eq!(settings.language, Language::System);
        assert!(!settings.auto_hide);
        assert_eq!(settings.window_level, WindowLevel::AlwaysOnTop);
    }

    #[test]
    fn language_and_auto_hide_round_trip() {
        let settings = Settings {
            window_x: None,
            window_y: Some(5.0),
            language: Language::TraditionalChinese,
            auto_hide: true,
            window_level: WindowLevel::AlwaysOnBottom,
        };
        let text = serde_json::to_string(&settings).unwrap();
        assert!(text.contains("zh-TW"));
        assert!(text.contains("bottom"));
        let parsed: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.language, Language::TraditionalChinese);
        assert!(parsed.auto_hide);
        assert_eq!(parsed.window_level, WindowLevel::AlwaysOnBottom);
    }

    #[test]
    fn every_language_has_a_stable_name() {
        for language in Language::ALL {
            let text = serde_json::to_string(&language).unwrap();
            let parsed: Language = serde_json::from_str(&text).unwrap();
            assert_eq!(parsed, language);
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn every_window_level_has_a_stable_name() {
        for level in WindowLevel::ALL {
            let text = serde_json::to_string(&level).unwrap();
            let parsed: WindowLevel = serde_json::from_str(&text).unwrap();
            assert_eq!(parsed, level);
            assert!(!text.is_empty());
            assert_eq!(WindowLevel::from_u8(level.as_u8()), Some(level));
        }
    }
}
