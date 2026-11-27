//! PlayStation 4 DualShock 4 input report decoding.
//!
//! ## Report layout
//!
//! Following Linux `drivers/hid/hid-playstation.c` (`dualshock4_parse_report`):
//!
//! * **USB** (`0x01`, 64 bytes): battery/cable status byte is at offset 30.
//! * **Bluetooth full** (`0x11`, 78+ bytes): battery/cable status byte is at
//!   offset 32 (two bytes later due to the Bluetooth header).
//!
//! Status byte bits:
//! * `bits 0..=3`: capacity bucket (`0..=10` when on battery; `0..=11` when
//!   plugged in, where `11` means full).
//! * `bit 4`: USB cable connected (`1` = charging or full).

use crate::dualsense::SONY_VENDOR_ID;
use crate::model::{bucket_to_percent, ChargeState, Precision, Transport};

/// DualShock 4 (CUH-ZCT1, original).
pub const DUALSHOCK4_V1_PID: u16 = 0x05C4;
/// DualShock 4 (CUH-ZCT2, Slim / Pro).
pub const DUALSHOCK4_V2_PID: u16 = 0x09CC;
/// Sony DualShock 4 USB Wireless Adaptor.
pub const DUALSHOCK4_DONGLE_PID: u16 = 0x0BA0;

/// Returns `true` if the product id belongs to a DualShock 4 controller.
pub fn is_supported_product(vendor_id: u16, product_id: u16) -> bool {
    vendor_id == SONY_VENDOR_ID
        && matches!(
            product_id,
            DUALSHOCK4_V1_PID | DUALSHOCK4_V2_PID | DUALSHOCK4_DONGLE_PID
        )
}

/// Result of decoding one DualShock 4 input report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedReport {
    pub percent: u8,
    pub state: ChargeState,
    pub transport: Transport,
    pub precision: Precision,
}

/// Decode one DualShock 4 HID input report (including the leading report id).
///
/// Returns `None` for simplified Bluetooth reports (`0x01`, 10 bytes) or any
/// other report that does not carry a valid battery byte.
pub fn decode_input_report(report: &[u8]) -> Option<DecodedReport> {
    let (status_offset, transport) = match (report.len(), report.first().copied()) {
        // USB input report: 64 bytes, report id 0x01.
        (64, Some(0x01)) => (30usize, Transport::Usb),
        // Bluetooth full input report: 78 bytes, report id 0x11.
        (78.., Some(0x11)) => (32usize, Transport::Bluetooth),
        _ => return None,
    };

    let status = *report.get(status_offset)?;
    let cable = (status & 0x10) != 0;
    let level = status & 0x0F;

    let (percent, state) = if cable {
        match level {
            0..=9 => (bucket_to_percent(level), ChargeState::Charging),
            10 => (100, ChargeState::Charging),
            11 => (100, ChargeState::Full),
            _ => return None,
        }
    } else {
        if level > 10 {
            return None;
        }
        (bucket_to_percent(level), ChargeState::Discharging)
    };

    Some(DecodedReport {
        percent,
        state,
        transport,
        precision: Precision::Buckets10,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_usb_discharging_charging_and_full() {
        let mut report = [0u8; 64];
        report[0] = 0x01;

        report[30] = 0x04; // cable=0, level=4 -> 45% discharging
        let d = decode_input_report(&report).unwrap();
        assert_eq!(d.transport, Transport::Usb);
        assert_eq!(d.percent, 45);
        assert_eq!(d.state, ChargeState::Discharging);

        report[30] = 0x16; // cable=1, level=6 -> 65% charging
        let d = decode_input_report(&report).unwrap();
        assert_eq!(d.percent, 65);
        assert_eq!(d.state, ChargeState::Charging);

        report[30] = 0x1A; // cable=1, level=10 -> 100% charging
        let d = decode_input_report(&report).unwrap();
        assert_eq!(d.percent, 100);
        assert_eq!(d.state, ChargeState::Charging);

        report[30] = 0x1B; // cable=1, level=11 -> 100% full
        let d = decode_input_report(&report).unwrap();
        assert_eq!(d.percent, 100);
        assert_eq!(d.state, ChargeState::Full);
    }

    #[test]
    fn decodes_bluetooth_full_report() {
        let mut report = [0u8; 78];
        report[0] = 0x11;
        report[32] = 0x08; // cable=0, level=8 -> 85% discharging

        let d = decode_input_report(&report).unwrap();
        assert_eq!(d.transport, Transport::Bluetooth);
        assert_eq!(d.percent, 85);
        assert_eq!(d.state, ChargeState::Discharging);
        assert_eq!(d.precision, Precision::Buckets10);
    }

    #[test]
    fn rejects_invalid_and_simplified_reports() {
        assert!(decode_input_report(&[]).is_none());
        // Simplified Bluetooth report (10 bytes, id 0x01) has no battery field.
        assert!(decode_input_report(&[0x01; 10]).is_none());
        // Bluetooth 0x01 78-byte report is not a DS4 full report.
        assert!(decode_input_report(&[0x01; 78]).is_none());

        let mut bad_level = [0u8; 64];
        bad_level[0] = 0x01;
        bad_level[30] = 0x0B; // level 11 without cable is invalid
        assert!(decode_input_report(&bad_level).is_none());

        bad_level[30] = 0x1C; // level 12 with cable is invalid
        assert!(decode_input_report(&bad_level).is_none());
    }
}
