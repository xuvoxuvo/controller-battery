//! User-visible text.
//!
//! Every string the user can see goes through this module: the card, the
//! context menu, the tray menu, the tooltip and the balloon. Strings live in
//! one table per language so adding a language is a matter of adding a table
//! and one line in [`Language::ALL`]; the tests below make sure a table is
//! never left with an empty field.
//!
//! `Language::System` follows the Windows UI language (read once and cached).

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use controller_battery_core::model::{ChargeState, Transport};
use serde::{Deserialize, Serialize};

use crate::collector::ErrorKind;

/// Language the user picked in the menu, persisted in `settings.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
    #[serde(rename = "zh-TW")]
    TraditionalChinese,
    #[serde(rename = "en")]
    English,
}

impl Language {
    pub const ALL: [Language; 4] = [
        Language::System,
        Language::SimplifiedChinese,
        Language::TraditionalChinese,
        Language::English,
    ];
}

/// A concrete language: `System` has already been resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    SimplifiedChinese,
    TraditionalChinese,
    English,
}

impl Resolved {
    fn from_index(index: u8) -> Self {
        match index {
            0 => Self::SimplifiedChinese,
            1 => Self::TraditionalChinese,
            _ => Self::English,
        }
    }

    fn index(self) -> u8 {
        match self {
            Self::SimplifiedChinese => 0,
            Self::TraditionalChinese => 1,
            Self::English => 2,
        }
    }
}

/// `u8::MAX` means "not configured yet"; [`resolved`] then detects the system
/// language.
const UNSET: u8 = u8::MAX;

static CURRENT: AtomicU8 = AtomicU8::new(UNSET);
static SYSTEM: OnceLock<Resolved> = OnceLock::new();

/// Apply the user's choice. Safe to call at any time; the tray thread reads the
/// same value on its next tick.
pub fn set(language: Language) {
    CURRENT.store(resolve(language).index(), Ordering::Relaxed);
}

/// The language actually in use.
pub fn resolved() -> Resolved {
    let stored = CURRENT.load(Ordering::Relaxed);
    if stored == UNSET {
        *SYSTEM.get_or_init(detect_system_language)
    } else {
        Resolved::from_index(stored)
    }
}

/// The string table for the current language.
pub fn current() -> &'static Strings {
    match resolved() {
        Resolved::SimplifiedChinese => &SIMPLIFIED_CHINESE,
        Resolved::TraditionalChinese => &TRADITIONAL_CHINESE,
        Resolved::English => &ENGLISH,
    }
}

fn resolve(language: Language) -> Resolved {
    match language {
        Language::System => *SYSTEM.get_or_init(detect_system_language),
        Language::SimplifiedChinese => Resolved::SimplifiedChinese,
        Language::TraditionalChinese => Resolved::TraditionalChinese,
        Language::English => Resolved::English,
    }
}

/// Windows UI language, translated into our three languages.
///
/// `LANGID` layout: primary language in the low byte, sub-language in bits
/// 10..16. Chinese is 0x04; traditional variants are the traditional, Hong Kong
/// and Macau sub-languages. Everything that is not Chinese falls back to
/// English, which is the broadest choice for an international user.
fn detect_system_language() -> Resolved {
    #[cfg(windows)]
    {
        let langid = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
        let primary = langid & 0xff;
        let sub = (langid >> 10) & 0x3f;
        match primary {
            0x04 => match sub {
                0x01 | 0x03 | 0x05 => Resolved::TraditionalChinese,
                _ => Resolved::SimplifiedChinese,
            },
            0x09 => Resolved::English,
            _ => Resolved::English,
        }
    }

    #[cfg(not(windows))]
    {
        Resolved::English
    }
}

/// One language's strings. `{}` is the only placeholder we need; the helper
/// methods fill it in.
pub struct Strings {
    pub app_name: &'static str,

    // Battery states and transports.
    pub charging: &'static str,
    pub discharging: &'static str,
    pub full: &'static str,
    pub battery_error: &'static str,
    pub not_reported: &'static str,
    pub level_only: &'static str,
    pub unknown: &'static str,
    pub idle: &'static str,
    pub usb: &'static str,
    pub bluetooth: &'static str,
    pub xinput: &'static str,

    // Card.
    pub no_device: &'static str,
    pub no_device_hint: &'static str,
    pub read_failed: &'static str,
    pub more_devices_fmt: &'static str,
    pub dualsense_open_failed: &'static str,

    // Context menu.
    pub hide_to_tray: &'static str,
    pub quit: &'static str,
    pub auto_hide: &'static str,
    pub language: &'static str,
    pub system_language: &'static str,
    pub low_integrity_hint: &'static str,
    pub tray_not_ready_hint: &'static str,

    // Tray.
    pub tray_show: &'static str,
    pub tray_hide: &'static str,
    pub hidden_title: &'static str,
    pub hidden_body: &'static str,
    pub tray_no_device: &'static str,
    pub tray_count_fmt: &'static str,
}

impl Strings {
    pub fn charge_state(&self, state: ChargeState) -> &'static str {
        match state {
            ChargeState::Charging => self.charging,
            ChargeState::Discharging => self.discharging,
            ChargeState::Full => self.full,
            ChargeState::BatteryError => self.battery_error,
            ChargeState::NotReported => self.not_reported,
            ChargeState::LevelOnly => self.level_only,
            ChargeState::Unknown => self.unknown,
        }
    }

    pub fn transport(&self, transport: Transport) -> &'static str {
        match transport {
            Transport::Usb => self.usb,
            Transport::Bluetooth => self.bluetooth,
            Transport::Unknown => self.xinput,
        }
    }

    pub fn error(&self, kind: ErrorKind) -> &'static str {
        match kind {
            ErrorKind::DualSenseOpenFailed => self.dualsense_open_failed,
        }
    }

    pub fn language_name(&self, language: Language) -> &'static str {
        match language {
            Language::System => self.system_language,
            // Native names, so the menu is readable whatever the current
            // language is.
            Language::SimplifiedChinese => "简体中文",
            Language::TraditionalChinese => "繁體中文",
            Language::English => "English",
        }
    }

    pub fn more_devices(&self, count: usize) -> String {
        self.more_devices_fmt.replace("{}", &count.to_string())
    }

    pub fn tray_count(&self, count: usize) -> String {
        self.tray_count_fmt.replace("{}", &count.to_string())
    }
}

static SIMPLIFIED_CHINESE: Strings = Strings {
    app_name: "Controller Battery",
    charging: "充电中",
    discharging: "使用中",
    full: "已充满",
    battery_error: "电池异常",
    not_reported: "未报告电量",
    level_only: "仅电量",
    unknown: "状态未知",
    idle: "空闲",
    usb: "USB",
    bluetooth: "蓝牙",
    xinput: "XInput",
    no_device: "未检测到手柄",
    no_device_hint: "蓝牙配对或插 USB 线",
    read_failed: "读取失败",
    more_devices_fmt: "还有 {} 个设备",
    dualsense_open_failed: "DualSense 打开失败（可能被其他程序占用）",
    hide_to_tray: "隐藏到托盘",
    quit: "退出",
    auto_hide: "空闲时自动隐藏",
    language: "语言",
    system_language: "跟随系统",
    low_integrity_hint: "当前进程以低权限运行，Windows 不允许注册托盘图标；请从资源管理器双击启动",
    tray_not_ready_hint: "托盘图标尚未就绪，正在自动重试",
    tray_show: "显示卡片",
    tray_hide: "隐藏卡片",
    hidden_title: "已隐藏到托盘",
    hidden_body: "左键单击托盘图标可恢复卡片",
    tray_no_device: "未检测到手柄",
    tray_count_fmt: "（共 {} 个）",
};

static TRADITIONAL_CHINESE: Strings = Strings {
    app_name: "Controller Battery",
    charging: "充電中",
    discharging: "使用中",
    full: "已充滿",
    battery_error: "電池異常",
    not_reported: "未回報電量",
    level_only: "僅電量",
    unknown: "狀態未知",
    idle: "閒置",
    usb: "USB",
    bluetooth: "藍牙",
    xinput: "XInput",
    no_device: "未偵測到手把",
    no_device_hint: "藍牙配對或插上 USB 線",
    read_failed: "讀取失敗",
    more_devices_fmt: "還有 {} 個裝置",
    dualsense_open_failed: "DualSense 開啟失敗（可能被其他程式占用）",
    hide_to_tray: "隱藏到系統匣",
    quit: "結束",
    auto_hide: "閒置時自動隱藏",
    language: "語言",
    system_language: "跟隨系統",
    low_integrity_hint: "目前行程以低權限執行，Windows 不允許註冊系統匣圖示；請從檔案總管雙擊啟動",
    tray_not_ready_hint: "系統匣圖示尚未就緒，正在自動重試",
    tray_show: "顯示卡片",
    tray_hide: "隱藏卡片",
    hidden_title: "已隱藏到系統匣",
    hidden_body: "左鍵單擊系統匣圖示可恢復卡片",
    tray_no_device: "未偵測到手把",
    tray_count_fmt: "（共 {} 個）",
};

static ENGLISH: Strings = Strings {
    app_name: "Controller Battery",
    charging: "Charging",
    discharging: "In use",
    full: "Full",
    battery_error: "Battery error",
    not_reported: "Not reported",
    level_only: "Level only",
    unknown: "Status unknown",
    idle: "Idle",
    usb: "USB",
    bluetooth: "Bluetooth",
    xinput: "XInput",
    no_device: "No controller detected",
    no_device_hint: "Pair over Bluetooth or plug in USB",
    read_failed: "Read failed",
    more_devices_fmt: "{} more device(s)",
    dualsense_open_failed: "Could not open DualSense (it may be in use by another app)",
    hide_to_tray: "Hide to tray",
    quit: "Quit",
    auto_hide: "Auto-hide when idle",
    language: "Language",
    system_language: "System",
    low_integrity_hint:
        "Running with low integrity: Windows blocks tray icons. Start the app from Explorer.",
    tray_not_ready_hint: "The tray icon is not ready yet; retrying automatically",
    tray_show: "Show card",
    tray_hide: "Hide card",
    hidden_title: "Hidden to tray",
    hidden_body: "Left-click the tray icon to bring the card back",
    tray_no_device: "No controller detected",
    tray_count_fmt: " ({} total)",
};

#[cfg(test)]
mod tests {
    use super::*;

    fn all_tables() -> [(&'static str, &'static Strings); 3] {
        [
            ("zh-CN", &SIMPLIFIED_CHINESE),
            ("zh-TW", &TRADITIONAL_CHINESE),
            ("en", &ENGLISH),
        ]
    }

    #[test]
    fn no_table_has_an_empty_string() {
        for (name, table) in all_tables() {
            for (field, value) in table.fields() {
                assert!(!value.trim().is_empty(), "{name}: {field} is empty");
            }
        }
    }

    #[test]
    fn every_charge_state_and_transport_is_translated() {
        for (name, table) in all_tables() {
            for state in [
                ChargeState::Charging,
                ChargeState::Discharging,
                ChargeState::Full,
                ChargeState::BatteryError,
                ChargeState::NotReported,
                ChargeState::LevelOnly,
                ChargeState::Unknown,
            ] {
                assert!(!table.charge_state(state).is_empty(), "{name}: {state:?}");
            }
            for transport in [Transport::Usb, Transport::Bluetooth, Transport::Unknown] {
                assert!(
                    !table.transport(transport).is_empty(),
                    "{name}: {transport:?}"
                );
            }
            assert!(!table.error(ErrorKind::DualSenseOpenFailed).is_empty());
        }
    }

    #[test]
    fn placeholders_are_filled() {
        for (name, table) in all_tables() {
            assert!(!table.more_devices(3).contains("{}"), "{name}");
            assert!(table.more_devices(3).contains('3'), "{name}");
            assert!(!table.tray_count(3).contains("{}"), "{name}");
            assert!(table.tray_count(3).contains('3'), "{name}");
        }
    }

    #[test]
    fn language_names_are_native() {
        let table = &SIMPLIFIED_CHINESE;
        assert_eq!(table.language_name(Language::English), "English");
        assert_eq!(table.language_name(Language::SimplifiedChinese), "简体中文");
        assert_eq!(
            table.language_name(Language::TraditionalChinese),
            "繁體中文"
        );
        assert!(!table.language_name(Language::System).is_empty());
    }

    #[test]
    fn system_language_resolution_never_panics() {
        // Whatever this machine's UI language is, resolving must produce one
        // of the three tables.
        assert!(matches!(
            resolve(Language::System),
            Resolved::SimplifiedChinese | Resolved::TraditionalChinese | Resolved::English
        ));
    }

    impl Strings {
        /// Field/value pairs for the completeness test; keeping this list next
        /// to the struct means a new field must be added here too.
        fn fields(&self) -> Vec<(&'static str, &'static str)> {
            vec![
                ("app_name", self.app_name),
                ("charging", self.charging),
                ("discharging", self.discharging),
                ("full", self.full),
                ("battery_error", self.battery_error),
                ("not_reported", self.not_reported),
                ("level_only", self.level_only),
                ("unknown", self.unknown),
                ("idle", self.idle),
                ("usb", self.usb),
                ("bluetooth", self.bluetooth),
                ("xinput", self.xinput),
                ("no_device", self.no_device),
                ("no_device_hint", self.no_device_hint),
                ("read_failed", self.read_failed),
                ("more_devices_fmt", self.more_devices_fmt),
                ("dualsense_open_failed", self.dualsense_open_failed),
                ("hide_to_tray", self.hide_to_tray),
                ("quit", self.quit),
                ("auto_hide", self.auto_hide),
                ("language", self.language),
                ("system_language", self.system_language),
                ("low_integrity_hint", self.low_integrity_hint),
                ("tray_not_ready_hint", self.tray_not_ready_hint),
                ("tray_show", self.tray_show),
                ("tray_hide", self.tray_hide),
                ("hidden_title", self.hidden_title),
                ("hidden_body", self.hidden_body),
                ("tray_no_device", self.tray_no_device),
                ("tray_count_fmt", self.tray_count_fmt),
            ]
        }
    }
}
