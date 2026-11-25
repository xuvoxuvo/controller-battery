//! Background collection threads.
//!
//! Threads publish into the same snapshot map, one per data source:
//! * DualSense (HID, blocking reads);
//! * 8BitDo Ultimate 2 Wireless in DInput mode (HID, blocking reads);
//! * XInput slots (polled);
//! * Bluetooth LE battery properties (polled).
//!
//! The UI shows every device that has reported recently.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use controller_battery_core::dualsense::{decode_input_report, DecodedReport};
use controller_battery_core::eightbitdo;
use controller_battery_core::hid::{enumerate_dualsense, HidDevice};
use controller_battery_core::model::{
    BatteryReading, ChargeState, Freshness, Precision, Transport,
};
use controller_battery_core::xinput;

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
    /// Last error, as a value the UI can translate.
    pub error: Option<ErrorKind>,
}

/// Failures the UI has to explain. Keep the message out of the collector: the
/// text belongs to the current language, not to the data source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// `HidDevice::open` failed; usually another program owns the pad.
    DualSenseOpenFailed,
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
    spawn_eightbitdo(state.clone());
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
            state.lock().unwrap().error = Some(ErrorKind::DualSenseOpenFailed);
            std::thread::sleep(Duration::from_millis(1500));
            continue;
        };

        let display_name = controller_battery_core::hid::product_name(&controller.path)
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
                    device.write_report(&controller_battery_core::dualsense::noop_output_report());
            }

            let reading = reading_from_dualsense(&decoded, &display_name);
            let mut shared = state.lock().unwrap();
            shared.upsert("dualsense", reading, decoded.degraded, STALE_AFTER);
            shared.error = None;
        }

        std::thread::sleep(Duration::from_millis(600));
    });
}

/// 8BitDo Ultimate 2 Wireless on its USB dongle in DInput mode.
///
/// In XInput mode (the pad's default when powered on normally) the same dongle
/// uses PID `0x310B` and reports no battery at all; in DInput mode (hold `B`
/// while powering on) it becomes this HID gamepad and the input report carries
/// an exact percentage.
fn spawn_eightbitdo(state: Arc<Mutex<Shared>>) {
    const GONE_GRACE: Duration = Duration::from_secs(3);
    let mut missing_since: Option<Instant> = None;

    std::thread::spawn(move || loop {
        let interface = controller_battery_core::hid::enumerate_all()
            .into_iter()
            .find(eightbitdo::is_supported);

        let Some(interface) = interface else {
            match missing_since {
                Some(since) if since.elapsed() > GONE_GRACE => {
                    state.lock().unwrap().remove("8bitdo");
                }
                Some(_) => {}
                None => missing_since = Some(Instant::now()),
            }
            std::thread::sleep(Duration::from_millis(700));
            continue;
        };
        missing_since = None;

        // Read-only: the pad (and Steam Input) may already own the device for
        // rumble writes, but HID input reports are delivered to every handle.
        let Some(device) = HidDevice::open_with_access(&interface, false) else {
            std::thread::sleep(Duration::from_millis(1000));
            continue;
        };

        let name = controller_battery_core::hid::product_name(&interface.path)
            .map(|name| clean_device_name(&name))
            .unwrap_or_else(|| "8BitDo 手柄".to_string());

        let mut buffer = [0u8; 128];
        // The dongle streams at up to 1 kHz; publishing every report would cost
        // 1000 mutex round-trips and string clones per second for a value that
        // changes a few times an hour. Publish on change, plus a heartbeat so
        // the row never looks idle.
        let mut last_published: Option<(eightbitdo::PowerState, Instant)> = None;
        while let Some(read) = device.read_report(&mut buffer) {
            let Some(power) = eightbitdo::decode_power_state(&buffer[..read]) else {
                continue;
            };

            let now = Instant::now();
            let unchanged = last_published.is_some_and(|(last, at)| {
                last == power && now.duration_since(at) < Duration::from_millis(500)
            });
            if unchanged {
                continue;
            }
            last_published = Some((power, now));

            let reading = BatteryReading {
                device: name.clone(),
                transport: Transport::Usb,
                percent: Some(power.percent),
                state: power.state,
                precision: Precision::Exact,
                freshness: Freshness::Live,
                note: None,
            };
            state
                .lock()
                .unwrap()
                .upsert("8bitdo", reading, false, STALE_AFTER);
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
    for interface in controller_battery_core::hid::enumerate_all() {
        if !VENDORS.contains(&interface.vendor_id) {
            continue;
        }
        if let Some(name) = controller_battery_core::hid::product_name(&interface.path) {
            if controller_battery_core::bluetooth::looks_like_controller(&name)
                && !names.contains(&name)
            {
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
        for device in controller_battery_core::bluetooth::enumerate() {
            if !controller_battery_core::bluetooth::looks_like_controller(&device.name) {
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

        std::thread::sleep(Duration::from_secs(1));
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
///
/// Filtering happens here, on every frame, so a pad that switches from
/// Bluetooth to its 2.4G dongle loses its stale Bluetooth row immediately
/// instead of waiting for the next Bluetooth poll.
pub fn snapshot(state: &Arc<Mutex<Shared>>) -> Vec<(String, DeviceState)> {
    let mut shared = state.lock().unwrap();
    shared.prune();

    let xinput_names: Vec<String> = shared
        .devices
        .iter()
        .filter(|(key, _)| key.starts_with("xinput:"))
        .map(|(_, value)| value.reading.device.clone())
        .collect();

    shared
        .devices
        .iter()
        .filter(|(key, value)| {
            if !key.starts_with("ble:") {
                return true;
            }
            let name = &value.reading.device;
            !xinput_names
                .iter()
                .any(|other| other.contains(name) || name.contains(other.as_str()))
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Manual end-to-end check against a real controller:
    /// `cargo test -p controller-battery-widget -- --ignored --nocapture dump_live_devices`
    #[test]
    #[ignore = "needs a real controller attached"]
    fn dump_live_devices() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        spawn(shared.clone());
        std::thread::sleep(Duration::from_secs(3));
        let snapshot = snapshot(&shared);
        for (key, state) in &snapshot {
            println!(
                "{key} -> name={:?} transport={:?} percent={:?} state={:?} stale={}",
                state.reading.device,
                state.reading.transport,
                state.reading.percent,
                state.reading.state,
                state.is_stale()
            );
        }
        assert!(!snapshot.is_empty(), "no device detected");
    }
}
