//! Persistent UI settings.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::Language;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub window_x: Option<f32>,
    pub window_y: Option<f32>,
    /// UI language; `System` follows the Windows UI language.
    pub language: Language,
    /// Hide the card when no controller is reporting.
    pub auto_hide: bool,
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

    pub fn save(&self) {
        let Some(path) = config_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }
}

/// Returns `true` when the saved card rectangle intersects at least one
/// currently attached monitor. Prevents restoring the card off-screen after a
/// secondary display is unplugged.
#[cfg(windows)]
pub fn is_position_visible(x: f32, y: f32, width: f32, height: f32) -> bool {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetDC, GetDeviceCaps, MonitorFromRect, ReleaseDC, LOGPIXELSX, MONITOR_DEFAULTTONULL,
    };

    if !x.is_finite() || !y.is_finite() {
        return false;
    }

    unsafe {
        let hdc = GetDC(HWND::default());
        let scale = if !hdc.is_invalid() {
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

        // Require an inset portion of the card to overlap a monitor so a card
        // grazing 1 px off-screen is still reset to a safe default position.
        let inset_x = (width * 0.2).min(32.0);
        let inset_y = (height * 0.2).min(20.0);
        let rect = RECT {
            left: ((x + inset_x) * scale).round() as i32,
            top: ((y + inset_y) * scale).round() as i32,
            right: ((x + width - inset_x) * scale).round() as i32,
            bottom: ((y + height - inset_y) * scale).round() as i32,
        };
        !MonitorFromRect(&rect, MONITOR_DEFAULTTONULL).is_invalid()
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
    }

    #[test]
    fn language_and_auto_hide_round_trip() {
        let settings = Settings {
            window_x: None,
            window_y: Some(5.0),
            language: Language::TraditionalChinese,
            auto_hide: true,
        };
        let text = serde_json::to_string(&settings).unwrap();
        assert!(text.contains("zh-TW"));
        let parsed: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.language, Language::TraditionalChinese);
        assert!(parsed.auto_hide);
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
}
