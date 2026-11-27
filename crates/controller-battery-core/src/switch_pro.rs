//! Nintendo Switch Pro Controller & Joy-Con HID input report decoding.
//!
//! ## Report layout
//!
//! Following Linux `drivers/hid/hid-nintendo.c` and SDL `SDL_hidapi_switch.c`:
//! standard input reports (`0x21`, `0x30`, `0x31`) carry battery information in
//! the high nibble of byte 2 (`report[2] >> 4`):
//!
//! * `bit 0` (`report[2] & 0x10`): charging flag;
//! * `bits 1..=3` (`(report[2] >> 5) & 0x07`): coarse level `0..=4`
//!   (empty / low / medium / high / full).

use crate::model::{ChargeState, Precision};

/// Nintendo vendor id.
pub const NINTENDO_VENDOR_ID: u16 = 0x057E;
/// Joy-Con (L).
pub const JOYCON_L_PID: u16 = 0x2006;
/// Joy-Con (R).
pub const JOYCON_R_PID: u16 = 0x2007;
/// Nintendo Switch Pro Controller.
pub const PRO_CONTROLLER_PID: u16 = 0x2009;

/// Returns `true` if the vendor/product id is a supported Switch controller.
pub fn is_supported_product(vendor_id: u16, product_id: u16) -> bool {
    vendor_id == NINTENDO_VENDOR_ID
        && matches!(product_id, PRO_CONTROLLER_PID | JOYCON_L_PID | JOYCON_R_PID)
}

/// Battery state decoded from one Switch HID input report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerState {
    pub percent: u8,
    pub state: ChargeState,
    pub precision: Precision,
}

/// Decode the battery level and charging flag from a standard input report
/// (`0x21`, `0x30`, or `0x31`).
pub fn decode_input_report(report: &[u8]) -> Option<PowerState> {
    if report.len() < 13 {
        return None;
    }
    if !matches!(report[0], 0x21 | 0x30 | 0x31) {
        return None;
    }

    let nibble = report[2] >> 4;
    let charging = (nibble & 0x01) != 0;
    let level = nibble >> 1;

    let percent = match level {
        0 => 5,
        1 => 25,
        2 => 50,
        3 => 75,
        4 => 100,
        _ => return None,
    };

    let state = if charging {
        ChargeState::Charging
    } else {
        ChargeState::Discharging
    };

    Some(PowerState {
        percent,
        state,
        precision: Precision::Coarse,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report(id: u8, battery_nibble: u8) -> [u8; 49] {
        let mut report = [0u8; 49];
        report[0] = id;
        report[2] = (battery_nibble << 4) | 0x01; // lower nibble is connection info
        report
    }

    #[test]
    fn decodes_all_five_levels() {
        let expected = [(0, 5), (1, 25), (2, 50), (3, 75), (4, 100)];
        for (level, percent) in expected {
            let report = sample_report(0x30, level << 1);
            let decoded = decode_input_report(&report).unwrap();
            assert_eq!(decoded.percent, percent);
            assert_eq!(decoded.state, ChargeState::Discharging);
            assert_eq!(decoded.precision, Precision::Coarse);
        }
    }

    #[test]
    fn decodes_charging_flag_across_report_ids() {
        for id in [0x21, 0x30, 0x31] {
            let report = sample_report(id, (3 << 1) | 0x01); // 75%, charging
            let decoded = decode_input_report(&report).unwrap();
            assert_eq!(decoded.percent, 75);
            assert_eq!(decoded.state, ChargeState::Charging);
        }
    }

    #[test]
    fn rejects_short_or_unknown_reports() {
        assert!(decode_input_report(&[0x30; 12]).is_none());
        // Report 0x3F is the simple HID report without a battery field.
        assert!(decode_input_report(&[0x3F; 49]).is_none());
        // Level 5 is out of range.
        let invalid = sample_report(0x30, 5 << 1);
        assert!(decode_input_report(&invalid).is_none());
    }
}
