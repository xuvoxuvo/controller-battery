//! 8BitDo Ultimate 2 Wireless over its USB dongle in DInput mode.
//!
//! The dongle exposes two very different faces depending on how the pad was
//! powered on:
//!
//! * hold nothing (default) — XInput device, PID `0x310B`, no battery at all;
//! * hold `B` while powering on — plain HID gamepad, PID `0x6012`, and its
//!   34-byte input report carries a real percentage.
//!
//! The byte layout follows SDL's `SDL_hidapi_8bitdo.c`
//! (`HIDAPI_Driver8BitDo_HandleStatePacket`): byte 14 holds the charge flag in
//! bit 7 and the level in bits 0..=6, with 100 meaning "full".

use crate::hid::HidInterface;
use crate::model::{ChargeState, Transport};

pub const VENDOR_ID: u16 = 0x2DC8;

/// 8BitDo Ultimate 2 Wireless (DInput over dongle / Bluetooth HID).
pub const ULTIMATE2_WIRELESS_PID: u16 = 0x6012;

/// Battery state decoded from one input report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerState {
    pub percent: u8,
    pub state: ChargeState,
}

/// Whether this HID interface is the 8BitDo pad in its DInput mode.
///
/// Bluetooth paths are skipped on purpose: the same PID also shows up as a BLE
/// HID interface, where the percentage comes from the Windows battery property
/// in [`crate::bluetooth`] and would otherwise produce a second row.
pub fn is_supported(interface: &HidInterface) -> bool {
    interface.vendor_id == VENDOR_ID
        && interface.product_id == ULTIMATE2_WIRELESS_PID
        && interface.transport() != Transport::Bluetooth
}

/// Decode the charge level from a 34-byte DInput report.
///
/// Firmware 1.02 sends 12-byte reports without the battery field; anything
/// shorter than 34 bytes (including the dongle's early reports while the pad
/// is still waking up) is rejected instead of guessed at.
pub fn decode_power_state(report: &[u8]) -> Option<PowerState> {
    if report.len() < 34 {
        return None;
    }

    let raw = report[14];
    let level = raw & 0x7F;
    if level > 100 {
        return None;
    }

    let charging = raw & 0x80 != 0;
    let (percent, state) = if level == 100 {
        (100, ChargeState::Full)
    } else if charging {
        (level, ChargeState::Charging)
    } else {
        (level, ChargeState::Discharging)
    };

    Some(PowerState { percent, state })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from the dongle (PID 0x6012) with the pad idle on battery.
    const IDLE_REPORT: [u8; 34] = [
        0x01, 0x0F, 0x7F, 0x7F, 0x7F, 0x7F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x62,
        0x43, 0xFE, 0xF8, 0xFF, 0xC6, 0x0F, 0xFE, 0xFF, 0x0F, 0x00, 0xEB, 0xFF, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn decodes_discharging_percentage() {
        assert_eq!(
            decode_power_state(&IDLE_REPORT),
            Some(PowerState {
                percent: 98,
                state: ChargeState::Discharging,
            })
        );
    }

    #[test]
    fn decodes_charging_flag() {
        let mut report = IDLE_REPORT;
        report[14] = 0x80 | 55;
        assert_eq!(
            decode_power_state(&report),
            Some(PowerState {
                percent: 55,
                state: ChargeState::Charging,
            })
        );
    }

    #[test]
    fn full_battery_wins_over_charge_flag() {
        let mut report = IDLE_REPORT;
        report[14] = 0x80 | 100;
        assert_eq!(
            decode_power_state(&report),
            Some(PowerState {
                percent: 100,
                state: ChargeState::Full,
            })
        );
    }

    #[test]
    fn rejects_short_and_invalid_reports() {
        assert_eq!(decode_power_state(&IDLE_REPORT[..12]), None);

        let mut report = IDLE_REPORT;
        report[14] = 0x7F; // 127 is not a percentage
        assert_eq!(decode_power_state(&report), None);
    }

    #[test]
    fn only_matches_usb_dongle_path() {
        let usb = HidInterface {
            path: "\\\\?\\hid#vid_2dc8&pid_6012#7&1af83eef&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}"
                .to_string(),
            vendor_id: VENDOR_ID,
            product_id: ULTIMATE2_WIRELESS_PID,
        };
        assert!(is_supported(&usb));

        let ble = HidInterface {
            path: "\\\\?\\hid#{00001124-0000-1000-8000-00805f9b34fb}_vid&00022dc8_pid&6012#7&1af83eef&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}"
                .to_string(),
            ..usb.clone()
        };
        assert!(!is_supported(&ble));

        let other = HidInterface {
            product_id: 0x310B,
            ..usb
        };
        assert!(!is_supported(&other));
    }
}
