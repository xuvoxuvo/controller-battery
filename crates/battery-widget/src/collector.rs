//! Background collection threads.
//!
//! Two threads publish into the same snapshot map:
//! * one for DualSense (HID, blocking reads);
//! * one for XInput slots (polled).
//!
//! The UI shows every device that has reported recently.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use battery_core::dualsense::{decode_input_report, DecodedReport};
use battery_core::hid::{enumerate_dualsense, HidDevice};
use battery_core::model::{BatteryReading, Freshness, Precision};
use battery_core::xinput;

/// How long an entry stays visible after its last update.
const ENTRY_TTL: Duration = Duration::from_secs(8);
/// A reading older than this is shown as "idle".
pub const STALE_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct DeviceState {
    pub reading: BatteryReading,
    pub degraded: bool,
    pub last_update: Instant,
}

impl DeviceState {
    pub fn is_stale(&self) -> bool {
        self.last_update.elapsed() > STALE_AFTER
    }
}

#[derive(Default)]
pub struct Shared {
    /// Ordered by device key so rows do not jump around.
    pub devices: BTreeMap<String, DeviceState>,
    pub error: Option<String>,
}

impl Shared {
    fn upsert(&mut self, key: &str, reading: BatteryReading, degraded: bool) {
        self.devices.insert(
            key.to_string(),
            DeviceState {
                reading,
                degraded,
                last_update: Instant::now(),
            },
        );
    }

    fn remove(&mut self, key: &str) {
        self.devices.remove(key);
    }

    fn prune(&mut self) {
        let now = Instant::now();
        self.devices
            .retain(|_, state| now.duration_since(state.last_update) < ENTRY_TTL);
    }
}

/// Start the collector threads.
pub fn spawn(state: Arc<Mutex<Shared>>) {
    spawn_dualsense(state.clone());
    spawn_xinput(state);
}

fn spawn_dualsense(state: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || loop {
        let controller = enumerate_dualsense().into_iter().next();
        let Some(controller) = controller else {
            state.lock().unwrap().remove("dualsense");
            std::thread::sleep(Duration::from_millis(1200));
            continue;
        };

        let Some(device) = HidDevice::open(&controller) else {
            state.lock().unwrap().error =
                Some("DualSense 打开失败（可能被其他程序占用）".to_string());
            std::thread::sleep(Duration::from_millis(1500));
            continue;
        };

        let mut buffer = [0u8; 128];
        let mut asked_for_full_report = false;
        while let Some(read) = device.read_report(&mut buffer) {
            let Some(decoded) = decode_input_report(&buffer[..read]) else {
                continue;
            };

            // The controller only sends the full report (with live battery
            // data) once a host has written to it.
            if decoded.degraded && !asked_for_full_report {
                asked_for_full_report =
                    device.write_report(&battery_core::dualsense::noop_output_report());
            }

            let reading = reading_from_dualsense(&decoded);
            let mut shared = state.lock().unwrap();
            shared.upsert("dualsense", reading, decoded.degraded);
            shared.error = None;
        }

        std::thread::sleep(Duration::from_millis(600));
    });
}

fn spawn_xinput(state: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || loop {
        for slot in 0..xinput::MAX_SLOTS {
            let key = format!("xinput:{slot}");
            match xinput::poll_slot(slot) {
                Some(reading) => {
                    let mut shared = state.lock().unwrap();
                    shared.upsert(&key, reading, false);
                }
                None => {
                    state.lock().unwrap().remove(&key);
                }
            }
        }
        state.lock().unwrap().prune();
        std::thread::sleep(Duration::from_millis(800));
    });
}

fn reading_from_dualsense(decoded: &DecodedReport) -> BatteryReading {
    BatteryReading {
        device: "DualSense".to_string(),
        transport: decoded.transport,
        percent: Some(decoded.percent),
        state: decoded.state,
        precision: Precision::Buckets10,
        freshness: Freshness::Live,
        note: if decoded.degraded {
            Some("简化报文，电量不刷新".to_string())
        } else {
            None
        },
    }
}

/// A stable, ordered snapshot for the UI.
pub fn snapshot(state: &Arc<Mutex<Shared>>) -> Vec<(String, DeviceState)> {
    let mut shared = state.lock().unwrap();
    shared.prune();
    shared
        .devices
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
