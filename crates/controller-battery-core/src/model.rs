//! Device independent battery model.

/// How the controller is attached to the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Usb,
    Bluetooth,
    /// XInput does not tell us; the controller may be wired or wireless.
    Unknown,
}

impl Transport {
    pub fn label(self) -> &'static str {
        match self {
            Transport::Usb => "USB",
            Transport::Bluetooth => "蓝牙",
            Transport::Unknown => "XInput",
        }
    }
}

/// Charging state as reported by the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeState {
    Charging,
    Discharging,
    Full,
    /// Battery reports an error condition (voltage/temperature/charge error).
    BatteryError,
    /// XInput reports `WIRED`, which third-party pads use for "no battery
    /// info" even when they are wireless. Do not claim "wired" from this alone.
    NotReported,
    /// Only a battery level is known (Bluetooth Battery Service).
    LevelOnly,
    Unknown,
}

impl ChargeState {
    pub fn label(self) -> &'static str {
        match self {
            ChargeState::Charging => "充电中",
            ChargeState::Discharging => "使用中",
            ChargeState::Full => "已充满",
            ChargeState::BatteryError => "电池异常",
            ChargeState::NotReported => "未报告电量",
            ChargeState::LevelOnly => "仅电量",
            ChargeState::Unknown => "状态未知",
        }
    }
}

/// How precise the percentage is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    /// DualSense reports one of 11 buckets (0..=10), i.e. 10% granularity.
    Buckets10,
    /// XInput only exposes empty/low/medium/full.
    Coarse,
    /// Exact percentage from the Bluetooth Battery Service.
    Exact,
}

impl Precision {
    pub fn label(self) -> &'static str {
        match self {
            Precision::Buckets10 => "10%",
            Precision::Coarse => "粗略",
            Precision::Exact => "1%",
        }
    }
}

/// Whether the value is being refreshed right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Reports are arriving and changing.
    Live,
    /// No new data for a while; the shown value is the last known one.
    Stale,
    Unknown,
}

impl Freshness {
    pub fn label(self) -> &'static str {
        match self {
            Freshness::Live => "实时",
            Freshness::Stale => "空闲",
            Freshness::Unknown => "",
        }
    }
}

/// One snapshot of a controller's battery.
#[derive(Debug, Clone)]
pub struct BatteryReading {
    pub device: String,
    pub transport: Transport,
    pub percent: Option<u8>,
    pub state: ChargeState,
    pub precision: Precision,
    pub freshness: Freshness,
    /// Extra explanation shown under the value, e.g. a degraded report.
    pub note: Option<String>,
}

impl BatteryReading {
    /// Human readable battery percentage, e.g. `25%` or `--`.
    pub fn percent_text(&self) -> String {
        match self.percent {
            Some(p) => format!("{p}%"),
            None => "--".to_string(),
        }
    }
}

/// 64-bit FNV-1a fingerprint of a raw HID input report.
///
/// Used to detect whether a controller is actually producing new packets or
/// Windows is repeating the last buffered report while the device is idle.
pub fn report_fingerprint(report: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in report {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Map the DualSense 0..=10 bucket to a mid-point percentage.
///
/// The hardware only distinguishes 10% steps, so the mid-point (bucket * 10 + 5)
/// is the most accurate single number that can be shown.
pub fn bucket_to_percent(bucket: u8) -> u8 {
    (bucket.min(10) as u16 * 10 + 5).min(100) as u8
}
