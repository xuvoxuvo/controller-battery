//! Background collection thread.
//!
//! The thread prefers a DualSense over USB, then over Bluetooth, and finally
//! falls back to XInput. It publishes an immutable snapshot that the UI reads.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use battery_core::dualsense::{decode_input_report, DecodedReport};
use battery_core::hid::{enumerate_dualsense, HidDevice};
use battery_core::model::{BatteryReading, Freshness, Precision, Transport};
use battery_core::xinput;

/// Snapshot shared between the collector thread and the UI.
#[derive(Default)]
pub struct Shared {
    pub reading: Option<BatteryReading>,
    /// The last report was the stale "simplified" Bluetooth report.
    pub degraded: bool,
    pub error: Option<String>,
    pub last_update: Option<Instant>,
}

impl Shared {
    /// `true` when no report arrived for `timeout`.
    pub fn is_stale(&self, timeout: Duration) -> bool {
        match self.last_update {
            Some(at) => at.elapsed() > timeout,
            None => true,
        }
    }
}

/// Start the collector thread.
pub fn spawn(state: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || loop {
        if let Some(controller) = pick_dualsense() {
            if let Some(device) = HidDevice::open(&controller) {
                read_dualsense(&device, &state);
                // Device went away (or read failed): re-enumerate shortly.
                std::thread::sleep(Duration::from_millis(600));
                continue;
            }
        }

        if let Some(reading) = xinput::poll() {
            publish_xinput(&state, reading);
            std::thread::sleep(Duration::from_millis(800));
            continue;
        }

        {
            let mut shared = state.lock().unwrap();
            shared.reading = None;
            shared.error = None;
            shared.last_update = None;
            shared.degraded = false;
        }
        std::thread::sleep(Duration::from_millis(1200));
    });
}

fn pick_dualsense() -> Option<battery_core::hid::HidController> {
    let controllers = enumerate_dualsense();
    controllers
        .iter()
        .find(|c| c.transport() == Transport::Usb)
        .or_else(|| controllers.first())
        .cloned()
}

fn read_dualsense(device: &HidDevice, state: &Arc<Mutex<Shared>>) {
    let mut buffer = [0u8; 128];
    loop {
        let Some(read) = device.read_report(&mut buffer) else {
            return;
        };
        let Some(decoded) = decode_input_report(&buffer[..read]) else {
            continue;
        };
        publish_dualsense(state, &decoded);
    }
}

fn publish_dualsense(state: &Arc<Mutex<Shared>>, decoded: &DecodedReport) {
    let reading = BatteryReading {
        device: "DualSense".to_string(),
        transport: decoded.transport,
        percent: Some(decoded.percent),
        state: decoded.state,
        precision: Precision::Buckets10,
        freshness: Freshness::Live,
        note: if decoded.degraded {
            Some("蓝牙简化报文，电量不刷新".to_string())
        } else {
            None
        },
    };

    let mut shared = state.lock().unwrap();
    shared.reading = Some(reading);
    shared.degraded = decoded.degraded;
    shared.last_update = Some(Instant::now());
    shared.error = None;
}

fn publish_xinput(state: &Arc<Mutex<Shared>>, reading: BatteryReading) {
    let mut shared = state.lock().unwrap();
    shared.reading = Some(reading);
    shared.degraded = false;
    shared.last_update = Some(Instant::now());
    shared.error = None;
}
