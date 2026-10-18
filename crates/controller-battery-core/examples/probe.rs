//! Manual probe: enumerate controllers and print what the system reports.
//!
//! Run with: cargo run -p controller-battery-core --example probe

use controller_battery_core::dualsense::decode_input_report;
use controller_battery_core::hid::{enumerate_all, enumerate_dualsense, HidDevice};
use controller_battery_core::xinput;

fn main() {
    let all = enumerate_all();
    println!("HID interfaces total: {}", all.len());
    for i in &all {
        println!(
            "  · vid=0x{:04X} pid=0x{:04X} {:?}",
            i.vendor_id,
            i.product_id,
            i.transport()
        );
    }

    let controllers = enumerate_dualsense();
    println!("\nDualSense interfaces: {}", controllers.len());
    for c in &controllers {
        println!(
            "  - pid=0x{:04X} transport={:?} path={}",
            c.product_id,
            c.transport(),
            c.path
        );
    }

    for c in &controllers {
        let Some(device) = HidDevice::open(c) else {
            println!("  ! failed to open {:?}", c.transport());
            continue;
        };
        for i in 0..3 {
            let mut buffer = [0u8; 128];
            match device.read_report(&mut buffer) {
                Some(n) => {
                    let decoded = decode_input_report(&buffer[..n]);
                    println!(
                        "  [{:?}] report #{} len={} decoded={:?}",
                        c.transport(),
                        i,
                        n,
                        decoded
                    );
                }
                None => {
                    println!("  [{:?}] read failed", c.transport());
                    break;
                }
            }
        }
    }

    println!("\nXInput:");
    for slot in 0..xinput::MAX_SLOTS {
        let state = xinput::debug_slot(slot);
        if let Some(line) = state {
            println!("  slot {}: {}", slot + 1, line);
        }
    }
    println!("  connected slots: {}", xinput::connected_slots());
    if let Some(reading) = xinput::poll() {
        println!("  first reading: {:?}", reading);
    }
}
