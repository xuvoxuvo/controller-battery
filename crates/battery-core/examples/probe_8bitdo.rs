//! Probe the 8BitDo vendor channel (2.4G mode).
//!
//! Sends the `identify` request that 8BitDo's own software sends on connect
//! and prints whatever the pad answers. Read-only from the pad's perspective:
//! the command only asks for version information.
//!
//! Usage: cargo run -p battery-core --example 8bitdo_probe

use std::sync::mpsc;
use std::time::Duration;

use battery_core::hid::{enumerate_all, HidDevice};

fn main() {
    let candidates: Vec<_> = enumerate_all()
        .into_iter()
        .filter(|i| i.vendor_id == 0x2DC8)
        .collect();

    println!("{} 8BitDo HID interfaces", candidates.len());
    for interface in &candidates {
        let Some(device) = HidDevice::open(interface) else {
            continue;
        };
        let caps = device.caps();
        let vendor = caps.map(|c| c.UsagePage >= 0xFF00).unwrap_or(false);
        println!(
            "\n=== {:?} vendor={} \n{}",
            caps.map(|c| (c.UsagePage, c.Usage)),
            vendor,
            interface.path
        );
        if !vendor {
            continue;
        }

        // Identify request used by 8BitDo's configuration software.
        let request = [0x81u8, 0x05, 0x00, 0x21, 0x01];
        println!(
            "  send {:02X?} -> {}",
            request,
            device.write_report(&request)
        );

        // Reads block until the pad answers, so run them on a worker thread.
        let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 128];
            loop {
                match device.read_report(&mut buffer) {
                    Some(n) => {
                        if tx.send(Some(buffer[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    None => {
                        let _ = tx.send(None);
                        break;
                    }
                }
            }
        });

        for i in 0..4 {
            match rx.recv_timeout(Duration::from_millis(800)) {
                Ok(Some(data)) => {
                    let hex: String = data
                        .iter()
                        .map(|b| format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    println!("  response #{i}: len={} {hex}", data.len());
                }
                _ => {
                    println!("  response #{i}: (none)");
                    break;
                }
            }
        }
    }
}
