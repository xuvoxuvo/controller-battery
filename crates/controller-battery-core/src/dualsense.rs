//! PlayStation 5 DualSense / DualSense Edge input report decoding.
//!
//! ## Report layout
//!
//! The battery lives in `status[0]`, at byte 53 of a **USB** report (`0x01`,
//! 64 bytes) and at byte 54 of a **Bluetooth** report (`0x31`, 78 bytes,
//! because the Bluetooth framing adds one byte after the report id).
//!
//! | bits   | meaning |
//! |--------|---------|
//! | 0..=3  | capacity bucket, 0..=10 (10% steps) |
//! | 4..=7  | charge state: 0 discharging, 1 charging, 2 full, 0xA/0xB error, 0xF error |
//!
//! See `docs/protocol.md` for the measurements behind this.
//!
//! ## The simplified Bluetooth report
//!
//! Some firmware sends a 78 byte report with id `0x01` instead of `0x31`
//! ("simplified" report). Its layout is the same but the battery/charge block
//! is **not refreshed** — it can keep reporting the state from the moment the
//! cable was plugged in. Such readings are marked [`degraded`](DecodedReport::degraded)
//! and the UI must not present them as live.

use crate::model::{bucket_to_percent, ChargeState, Precision, Transport};

/// Result of decoding one input report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedReport {
    pub percent: u8,
    pub state: ChargeState,
    pub transport: Transport,
    /// `true` for the 78 byte `0x01` report whose battery block is stale.
    pub degraded: bool,
    pub precision: Precision,
}

/// DualSense vendor id.
pub const SONY_VENDOR_ID: u16 = 0x054C;
/// DualSense.
pub const DUALSENSE_PRODUCT_ID: u16 = 0x0CE6;
/// DualSense Edge.
pub const DUALSENSE_EDGE_PRODUCT_ID: u16 = 0x0DF2;

/// Returns `true` if the USB product id is a supported PlayStation controller.
pub fn is_supported_product(product_id: u16) -> bool {
    matches!(product_id, DUALSENSE_PRODUCT_ID | DUALSENSE_EDGE_PRODUCT_ID)
}

/// Decode one HID input report (including the leading report id).
///
/// Returns `None` when the report is not a report we understand.
pub fn decode_input_report(report: &[u8]) -> Option<DecodedReport> {
    let (offset, transport, degraded) = match (report.len(), report.first().copied()) {
        // USB: 64 bytes, report id 0x01, payload starts right after the id.
        (64, Some(0x01)) => (1usize, Transport::Usb, false),
        // Bluetooth full report: 78 bytes, report id 0x31, one extra header byte.
        (78.., Some(0x31)) => (2usize, Transport::Bluetooth, false),
        // Bluetooth simplified report: same payload, but the battery block is stale.
        (78.., Some(0x01)) => (2usize, Transport::Bluetooth, true),
        _ => return None,
    };

    let status = *report.get(offset + 52)?;
    let bucket = status & 0x0F;
    let charge = (status >> 4) & 0x0F;

    if bucket > 10 {
        return None;
    }

    let (percent, state) = match charge {
        0x0 => (bucket_to_percent(bucket), ChargeState::Discharging),
        0x1 => (bucket_to_percent(bucket), ChargeState::Charging),
        0x2 => (100, ChargeState::Full),
        0xA | 0xB | 0xF => (0, ChargeState::BatteryError),
        _ => (bucket_to_percent(bucket), ChargeState::Unknown),
    };

    Some(DecodedReport {
        percent,
        state,
        transport,
        degraded,
        precision: Precision::Buckets10,
    })
}

/// PlayStation CRC32 variant used by the DualSense: standard reflected CRC-32
/// over `seed || data`, seeded with `0xFFFFFFFF` and inverted at the end.
pub fn playstation_crc32(seed: u8, data: &[u8]) -> u32 {
    fn update(mut crc: u32, data: &[u8]) -> u32 {
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        crc
    }
    let crc = update(0xFFFF_FFFF, &[seed]);
    !update(crc, data)
}

/// Build the "no state change" Bluetooth output report.
///
/// Every `valid flag` is zero, so LEDs, rumble and audio are untouched; the
/// message only tells the controller that a host is talking to it. Combined
/// with opening the device for writing this makes the DualSense switch to the
/// full `0x31` input report, which carries live battery data.
pub fn noop_output_report() -> [u8; 78] {
    let mut report = [0u8; 78];
    report[0] = 0x31; // output report id (Bluetooth)
    report[1] = 0x10; // seq_tag
    report[2] = 0x10; // tag
    let crc = playstation_crc32(0xA2, &report[..74]);
    report[74..78].copy_from_slice(&crc.to_le_bytes());
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real 78 byte Bluetooth report captured from a DualSense while the
    /// controller was plugged in (simplified format, battery byte 0x12).
    const BT_SIMPLIFIED: [u8; 78] = [
        0x01, 0x81, 0x7E, 0x80, 0x7F, 0x08, 0x00, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2D, 0x87,
        0x94, 0xE9, 0x00, 0x00, 0x1C, 0x00, 0xFE, 0xFF, 0x09, 0x00, 0x20, 0x14, 0xDF, 0xE6, 0x92,
        0x0A, 0x52, 0x67, 0x16, 0x80, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x09, 0x09,
        0x00, 0x00, 0x00, 0x00, 0x00, 0xA7, 0xAE, 0x52, 0x67, 0x12, 0x10, 0x00, 0x4F, 0xEC, 0xAB,
        0x81, 0x69, 0xF7, 0x22, 0x43, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07,
        0x06, 0x39, 0xBA,
    ];

    #[test]
    fn decodes_simplified_bluetooth_report_as_degraded() {
        let decoded = decode_input_report(&BT_SIMPLIFIED).expect("report should decode");
        assert_eq!(decoded.transport, Transport::Bluetooth);
        assert_eq!(decoded.percent, 25);
        assert_eq!(decoded.state, ChargeState::Charging);
        assert!(
            decoded.degraded,
            "0x01 over Bluetooth must be marked degraded"
        );
    }

    #[test]
    fn decodes_full_bluetooth_report() {
        let mut report = BT_SIMPLIFIED;
        report[0] = 0x31;
        // Same physical state, but the full report says "discharging".
        report[54] = 0x02;

        let decoded = decode_input_report(&report).expect("report should decode");
        assert_eq!(decoded.transport, Transport::Bluetooth);
        assert_eq!(decoded.percent, 25);
        assert_eq!(decoded.state, ChargeState::Discharging);
        assert!(!decoded.degraded);
    }

    #[test]
    fn decodes_usb_report() {
        let mut report = [0u8; 64];
        report[0] = 0x01;
        report[53] = 0x12; // 25%, charging

        let decoded = decode_input_report(&report).expect("report should decode");
        assert_eq!(decoded.transport, Transport::Usb);
        assert_eq!(decoded.percent, 25);
        assert_eq!(decoded.state, ChargeState::Charging);
        assert!(!decoded.degraded);
    }

    #[test]
    fn maps_full_and_error_states() {
        let mut report = [0u8; 64];
        report[0] = 0x01;

        report[53] = 0x2A; // full
        let d = decode_input_report(&report).unwrap();
        assert_eq!((d.percent, d.state), (100, ChargeState::Full));

        report[53] = 0xB2; // temperature error
        let d = decode_input_report(&report).unwrap();
        assert_eq!(d.state, ChargeState::BatteryError);

        report[53] = 0xF2; // charge error
        let d = decode_input_report(&report).unwrap();
        assert_eq!(d.state, ChargeState::BatteryError);
    }

    #[test]
    fn rejects_unknown_reports() {
        assert!(decode_input_report(&[]).is_none());
        assert!(decode_input_report(&[0x02; 64]).is_none());
        assert!(decode_input_report(&[0x01; 12]).is_none());
    }

    #[test]
    fn bucket_midpoints() {
        assert_eq!(bucket_to_percent(0), 5);
        assert_eq!(bucket_to_percent(2), 25);
        assert_eq!(bucket_to_percent(10), 100);
        assert_eq!(bucket_to_percent(99), 100);
    }

    #[test]
    fn crc32_matches_reference_vector() {
        // Standard CRC-32 check value.
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in b"123456789" {
            crc ^= byte as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        assert_eq!(!crc, 0xCBF4_3926);
    }

    #[test]
    fn noop_report_has_crc_and_zero_flags() {
        let report = noop_output_report();
        assert_eq!(report[0], 0x31);
        assert!(report[3..74].iter().all(|&b| b == 0));
        let expected = playstation_crc32(0xA2, &report[..74]);
        assert_eq!(&report[74..78], &expected.to_le_bytes());
    }
}
