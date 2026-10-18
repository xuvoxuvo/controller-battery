//! Capture HID reports for a vendor/product id and report which bytes change.
//!
//! Usage: cargo run -p battery-core --example capture -- <vid_hex> <pid_hex> [seconds]

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use battery_core::hid::{enumerate_all, HidDevice};

fn parse_hex(value: &str) -> Option<u16> {
    u16::from_str_radix(value.trim_start_matches("0x"), 16).ok()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let vid = args.next().and_then(|v| parse_hex(&v));
    let pid = args.next().filter(|v| v != "-").and_then(|v| parse_hex(&v));
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(20);

    let interfaces: Vec<_> = enumerate_all()
        .into_iter()
        .filter(|i| vid.is_none_or(|v| i.vendor_id == v))
        .filter(|i| pid.is_none_or(|p| i.product_id == p))
        .collect();

    println!(
        "{} interfaces match, capturing {}s",
        interfaces.len(),
        seconds
    );
    let deadline = Instant::now() + Duration::from_secs(seconds);

    for (index, interface) in interfaces.iter().enumerate() {
        println!(
            "\n=== #{} vid=0x{:04X} pid=0x{:04X} {:?}",
            index,
            interface.vendor_id,
            interface.product_id,
            interface.transport()
        );
        println!("{}", interface.path);

        let Some(device) = HidDevice::open(interface) else {
            println!("  (open failed)");
            continue;
        };

        let (tx, rx) = mpsc::channel::<Option<(Instant, Vec<u8>)>>();
        let owner = interface.clone();
        std::thread::spawn(move || {
            let device = device;
            let mut buffer = [0u8; 256];
            loop {
                match device.read_report(&mut buffer) {
                    Some(n) => {
                        if tx
                            .send(Some((Instant::now(), buffer[..n].to_vec())))
                            .is_err()
                        {
                            break;
                        }
                    }
                    None => {
                        let _ = tx.send(None);
                        break;
                    }
                }
            }
            drop(owner);
        });

        let mut samples: Vec<(Instant, Vec<u8>)> = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining.min(Duration::from_millis(200))) {
                Ok(Some(sample)) => samples.push(sample),
                Ok(None) => break,
                Err(_) => {}
            }
        }

        if samples.is_empty() {
            println!("  no reports captured");
            continue;
        }

        let len = samples.iter().map(|s| s.1.len()).max().unwrap_or(0);
        println!("  {} reports, max len {}", samples.len(), len);

        let mut changes: BTreeMap<usize, usize> = BTreeMap::new();
        for offset in 0..len {
            let mut previous = None;
            let mut count = 0;
            for (_, data) in &samples {
                let value = data.get(offset).copied();
                if previous.is_some() && value != previous {
                    count += 1;
                }
                previous = value;
            }
            if count > 0 {
                changes.insert(offset, count);
            }
        }

        let stable: Vec<String> = (0..len)
            .filter(|o| !changes.contains_key(o))
            .map(|o| format!("{o:02}"))
            .collect();
        println!("  changing offsets: {:?}", changes);
        println!("  stable offsets:   {}", stable.join(" "));

        let first = &samples[0].1;
        let hex: String = first
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        println!("  first report: {hex}");
        let last = &samples[samples.len() - 1].1;
        let hex: String = last
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        println!("  last report:  {hex}");
    }
}
