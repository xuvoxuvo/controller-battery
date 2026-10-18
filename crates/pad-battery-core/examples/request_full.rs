//! Experiment: ask the DualSense to use the full Bluetooth report format.
//!
//! Sends one output report with every "valid flag" cleared (no LED or rumble
//! change) and then prints the report ids that arrive afterwards.

use pad_battery_core::dualsense::decode_input_report;
use pad_battery_core::hid::{enumerate_dualsense, HidDevice};

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn noop_report() -> [u8; 78] {
    let mut report = [0u8; 78];
    report[0] = 0x31; // output report id (Bluetooth)
    report[1] = 0x10; // seq_tag
    report[2] = 0x10; // tag
                      // bytes 3..: all valid flags zero -> no state change
    let mut crc_input = Vec::with_capacity(75);
    crc_input.push(0xA2); // output CRC seed
    crc_input.extend_from_slice(&report[..74]);
    let crc = crc32(&crc_input);
    report[74..78].copy_from_slice(&crc.to_le_bytes());
    report
}

fn main() {
    let controllers = enumerate_dualsense();
    let Some(controller) = controllers
        .iter()
        .find(|c| c.transport() == pad_battery_core::Transport::Bluetooth)
    else {
        println!("no Bluetooth DualSense found");
        return;
    };

    let Some(device) = HidDevice::open_with_access(controller, true) else {
        println!("failed to open with write access");
        return;
    };

    let report = noop_report();
    println!("sending no-op output report (crc=0x{:08X})", {
        let mut v = vec![0xA2u8];
        v.extend_from_slice(&report[..74]);
        crc32(&v)
    });
    println!("write ok = {}", device.write_report(&report));

    let mut buffer = [0u8; 128];
    for i in 0..12 {
        let Some(n) = device.read_report(&mut buffer) else {
            println!("read #{i} failed");
            break;
        };
        println!(
            "report #{i}: id=0x{:02X} len={} decoded={:?}",
            buffer[0],
            n,
            decode_input_report(&buffer[..n])
        );
    }
}
