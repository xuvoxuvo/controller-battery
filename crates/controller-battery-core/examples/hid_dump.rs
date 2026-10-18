//! Dump HID interfaces matching a vendor/product id.
//!
//! Usage: cargo run -p controller-battery-core --example hid_dump -- <vid_hex> <pid_hex>
//!
//! Reads use a watchdog thread so interfaces that never emit reports (for
//! example XInput-only collections) do not block the dump.

use std::sync::mpsc;
use std::time::Duration;

use controller_battery_core::hid::{enumerate_all, HidDevice};

fn parse_hex(value: &str) -> Option<u16> {
    u16::from_str_radix(value.trim_start_matches("0x"), 16).ok()
}

fn read_with_timeout(device: HidDevice, timeout: Duration) -> Option<(usize, Vec<u8>)> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 256];
        let result = device
            .read_report(&mut buffer)
            .map(|n| (n, buffer[..n].to_vec()));
        let _ = tx.send(result);
    });
    rx.recv_timeout(timeout).ok().flatten()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let vid = args.next().and_then(|v| parse_hex(&v));
    let pid = args.next().and_then(|v| parse_hex(&v));

    let interfaces: Vec<_> = enumerate_all()
        .into_iter()
        .filter(|i| vid.is_none_or(|v| i.vendor_id == v))
        .filter(|i| pid.is_none_or(|p| i.product_id == p))
        .collect();

    println!("{} interfaces match", interfaces.len());
    for interface in &interfaces {
        println!(
            "\n=== vid=0x{:04X} pid=0x{:04X} {:?}\n{}",
            interface.vendor_id,
            interface.product_id,
            interface.transport(),
            interface.path
        );

        for i in 0..2 {
            let Some(device) = HidDevice::open(interface) else {
                println!("  (open failed)");
                break;
            };
            if i == 0 {
                match device.caps() {
                    Some(caps) => println!(
                        "  caps: usagePage=0x{:04X} usage=0x{:04X} in={} out={} feature={}",
                        caps.UsagePage,
                        caps.Usage,
                        caps.InputReportByteLength,
                        caps.OutputReportByteLength,
                        caps.FeatureReportByteLength
                    ),
                    None => println!("  caps: unavailable"),
                }
            }
            match read_with_timeout(device, Duration::from_millis(700)) {
                Some((n, data)) => {
                    let hex: String = data[..n.min(40)]
                        .iter()
                        .map(|b| format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    println!("  report #{i}: id=0x{:02X} len={n} {hex}", data[0]);
                }
                None => {
                    println!("  report #{i}: (no data within 700 ms)");
                    break;
                }
            }
        }
    }
}
