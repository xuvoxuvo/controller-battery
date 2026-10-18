//! Persistent UI settings.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub window_x: Option<f32>,
    pub window_y: Option<f32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            window_x: None,
            window_y: None,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let mut dir = PathBuf::from(base);
    dir.push("ControllerBattery");
    Some(dir.join("settings.json"))
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
