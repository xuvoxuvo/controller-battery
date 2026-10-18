//! List Bluetooth devices that expose a battery level.
//!
//! Usage: cargo run -p battery-core --example bt_battery

use battery_core::bluetooth::{enumerate, looks_like_controller};

fn main() {
    let devices = enumerate();
    println!(
        "{} Bluetooth devices with a battery property",
        devices.len()
    );
    for device in &devices {
        println!(
            "  {:>3}%  controller={}  {}  [{}]",
            device.percent,
            looks_like_controller(&device.name),
            device.name,
            device.instance_id
        );
    }
}
