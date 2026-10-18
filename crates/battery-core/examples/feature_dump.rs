//! Enumerate feature reports of a HID interface.
//!
//! Usage: cargo run -p battery-core --example feature_dump -- <vid_hex> <pid_hex> [max_id]

use battery_core::hid::{enumerate_all, HidDevice};

fn parse_hex(value: &str) -> Option<u16> {
    u16::from_str_radix(value.trim_start_matches("0x"), 16).ok()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let vid = args.next().and_then(|v| parse_hex(&v));
    let pid = args.next().and_then(|v| parse_hex(&v));
    let max_id: u8 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0x30);

    let interfaces: Vec<_> = enumerate_all()
        .into_iter()
        .filter(|i| vid.is_none_or(|v| i.vendor_id == v))
        .filter(|i| pid.is_none_or(|p| i.product_id == p))
        .collect();

    println!("{} interfaces match", interfaces.len());
    for interface in &interfaces {
        println!(
            "\n=== vid=0x{:04X} pid=0x{:04X}\n{}",
            interface.vendor_id, interface.product_id, interface.path
        );
        let Some(device) = HidDevice::open(interface) else {
            println!("  (open failed)");
            continue;
        };

        for id in 0..=max_id {
            let mut buffer = [0u8; 64];
            buffer[0] = id;
            if let Some(len) = device.get_feature(&mut buffer) {
                let hex: String = buffer[..len.min(32)]
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("  feature 0x{id:02X}: {hex}");
            }
        }
    }
}
