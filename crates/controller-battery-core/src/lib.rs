//! Battery reporting for game controllers.
//!
//! The crate is split so that protocol parsing is testable without any
//! operating-system access:
//!
//! * [`dualsense`] — PlayStation 5 DualSense / DualSense Edge HID reports
//! * [`dualshock4`] — PlayStation 4 DualShock 4 HID reports
//! * [`switch_pro`] — Nintendo Switch Pro Controller & Joy-Con HID reports
//! * [`eightbitdo`] — 8BitDo Ultimate 2 Wireless in DInput mode (HID)
//! * [`xinput`] — Xbox-compatible controllers (Windows XInput)
//! * [`bluetooth`] — Bluetooth LE Battery Service (Windows PnP + WinRT)
//! * [`model`] — the device-independent reading type shared by all backends

pub mod dualsense;
pub mod dualshock4;
pub mod model;
pub mod switch_pro;

#[cfg(windows)]
pub mod bluetooth;
#[cfg(windows)]
pub mod eightbitdo;
#[cfg(windows)]
pub mod hid;
#[cfg(windows)]
pub mod xinput;

pub use model::{BatteryReading, ChargeState, Freshness, Precision, Transport};
