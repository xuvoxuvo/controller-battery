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
use battery_core::model::{BatteryReading, ChargeState, Freshness, Precision, Transport};
use battery_core::xinput;

/// How long an entry stays visible after its last update.
const ENTRY_TTL: Duration = Duration::from_secs(10);
/// A reading older than this is shown as "idle".
pub const STALE_AFTER: Duration = Duration::from_secs(5);
/// Bluetooth battery values are polled, so they may be older than that.
const BLE_STALE_AFTER: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub struct DeviceState {
    pub reading: BatteryReading,
    pub degraded: bool,
    pub last_update: Instant,
    /// How long a reading stays "live". Polled sources refresh less often than
    /// the HID stream, so each source passes its own value.
    pub stale_after: Duration,
}

impl DeviceState {
    pub fn is_stale(&self) -> bool {
        self.last_update.elapsed() > self.stale_after
    }
}

#[derive(Default)]
pub struct Shared {
    /// Ordered by device key so rows do not jump around.
    pub devices: BTreeMap<String, DeviceState>,
    pub error: Option<String>,
}

impl Shared {
    fn upsert(
        &mut self,
        key: &str,
        reading: BatteryReading,
        degraded: bool,
        stale_after: Duration,
    ) {
        self.devices.insert(
            key.to_string(),
            DeviceState {
                reading,
                degraded,
                last_update: Instant::now(),
                stale_after,
            },
        );
    }

    fn remove(&mut self, key: &str) {
        self.devices.remove(key);
    }

    /// Drop entries that have been silent for a while. The grace period keeps
    /// a controller that briefly stops reporting from making the card flicker.
    fn prune(&mut self) {
        let now = Instant::now();
        self.devices.retain(|_, state| {
            now.duration_since(state.last_update) < state.stale_after + ENTRY_TTL
        });
    }
}

/// Start the collector threads.
pub fn spawn(state: Arc<Mutex<Shared>>) {
    spawn_dualsense(state.clone());
    spawn_xinput(state.clone());
    spawn_bluetooth(state);
}

fn spawn_dualsense(state: Arc<Mutex<Shared>>) {
    // How long a controller may be missing from enumeration before its row is
    // retired. Enumeration is authoritative, so this only smooths over the
    // instant between a reconnect.
    const GONE_GRACE: Duration = Duration::from_secs(3);
    let mut missing_since: Option<Instant> = None;

    std::thread::spawn(move || loop {
        let controller = enumerate_dualsense().into_iter().next();
        let Some(controller) = controller else {
            match missing_since {
                Some(since) if since.elapsed() > GONE_GRACE => {
                    state.lock().unwrap().remove("dualsense");
                }
                Some(_) => {}
                None => missing_since = Some(Instant::now()),
            }
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };
        missing_since = None;

        let Some(device) = HidDevice::open(&controller) else {
            state.lock().unwrap().error =
                Some("DualSense 打开失败（可能被其他程序占用）".to_string());
            std::thread::sleep(Duration::from_millis(1500));
            continue;
        };

        let display_name = battery_core::hid::product_name(&controller.path)
            .unwrap_or_else(|| "DualSense".to_string());

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

            let reading = reading_from_dualsense(&decoded, &display_name);
            let mut shared = state.lock().unwrap();
            shared.upsert("dualsense", reading, decoded.degraded, STALE_AFTER);
            shared.error = None;
        }

        std::thread::sleep(Duration::from_millis(600));
    });
}

/// Product names of controller-like HID devices.
///
/// XInput itself does not expose device names, so we borrow the HID product
/// string (same name Device Manager shows) to label the slot.
fn controller_product_names() -> Vec<String> {
    const VENDORS: [u16; 8] = [
        0x045E, // Microsoft
        0x2DC8, // 8BitDo
        0x057E, // Nintendo
        0x0F0D, // Hori
        0x1532, // Razer
        0x28DE, // Valve
        0x0E6F, // PDP
        0x24C6, // PowerA
                // Sony is deliberately absent: DualSense has its own HID backend and
                // its name must not be borrowed for an unrelated XInput slot.
    ];

    let mut names: Vec<String> = Vec::new();
    for interface in battery_core::hid::enumerate_all() {
        if !VENDORS.contains(&interface.vendor_id) {
            continue;
        }
        if let Some(name) = battery_core::hid::product_name(&interface.path) {
            if battery_core::bluetooth::looks_like_controller(&name) && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn spawn_xinput(state: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || {
        let mut names: Vec<String> = Vec::new();
        let mut ticks = 0u32;
        loop {
            // Renaming only matters when devices come and go; refreshing it
            // every few seconds is enough and avoids enumerating at 1 Hz.
            if ticks.is_multiple_of(8) {
                names = controller_product_names();
            }
            ticks = ticks.wrapping_add(1);

            for slot in 0..xinput::MAX_SLOTS {
                let key = format!("xinput:{slot}");
                match xinput::poll_slot(slot) {
                    Some(mut reading) => {
                        if let Some(name) = names.first() {
                            reading.device = clean_device_name(name);
                        }
                        let mut shared = state.lock().unwrap();
                        shared.upsert(&key, reading, false, STALE_AFTER);
                    }
                    None => {
                        state.lock().unwrap().remove(&key);
                    }
                }
            }
            state.lock().unwrap().prune();
            std::thread::sleep(Duration::from_millis(800));
        }
    });
}

/// Bluetooth LE controllers expose their battery through the standard Battery
/// Service; Windows publishes it as a device property, which we poll.
fn spawn_bluetooth(state: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || loop {
        let mut seen: Vec<String> = Vec::new();
        for device in battery_core::bluetooth::enumerate() {
            if !battery_core::bluetooth::looks_like_controller(&device.name) {
                continue;
            }
            let key = format!("ble:{}", device.instance_id);
            seen.push(key.clone());

            let reading = BatteryReading {
                device: device.name.clone(),
                transport: Transport::Bluetooth,
                percent: Some(device.percent),
                state: ChargeState::LevelOnly,
                precision: Precision::Exact,
                freshness: Freshness::Live,
                note: None,
            };
            state
                .lock()
                .unwrap()
                .upsert(&key, reading, false, BLE_STALE_AFTER);
        }

        // A device that is no longer paired should disappear immediately: the
        // pairing list is authoritative, unlike a missing report.
        let mut shared = state.lock().unwrap();
        let gone: Vec<String> = shared
            .devices
            .keys()
            .filter(|k| k.starts_with("ble:") && !seen.contains(k))
            .cloned()
            .collect();
        for key in gone {
            shared.remove(&key);
        }
        drop(shared);

        std::thread::sleep(Duration::from_secs(10));
    });
}

/// Trim the boilerplate Windows puts around HID product strings, e.g.
/// `Controller (8BitDo Ultimate 2 Wireless Controller)`.
pub fn clean_device_name(name: &str) -> String {
    let trimmed = name.trim();
    if let Some(rest) = trimmed.strip_prefix("Controller (") {
        if let Some(inner) = rest.strip_suffix(')') {
            return inner.trim().to_string();
        }
    }
    trimmed.to_string()
}

fn reading_from_dualsense(decoded: &DecodedReport, name: &str) -> BatteryReading {
    BatteryReading {
        device: clean_device_name(name),
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
