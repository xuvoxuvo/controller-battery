//! Battery reporting for game controllers.
//!
//! The crate is split so that protocol parsing is testable without any
//! operating-system access:
//!
//! * [`dualsense`] — PlayStation 5 DualSense / DualSense Edge HID reports
//! * [`xinput`] — Xbox-compatible controllers (Windows XInput)
//! * [`model`] — the device-independent reading type shared by all backends

pub mod dualsense;
pub mod model;

#[cfg(windows)]
pub mod hid;
#[cfg(windows)]
pub mod xinput;

pub use model::{BatteryReading, ChargeState, Freshness, Precision, Transport};
