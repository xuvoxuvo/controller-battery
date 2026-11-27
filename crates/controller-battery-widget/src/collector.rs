//! Background collection threads.
//!
//! Threads publish into the same snapshot map, one per data source:
//! * DualSense / DualSense Edge (HID, blocking reads, multi-device);
//! * DualShock 4 (HID, blocking reads, multi-device);
//! * Nintendo Switch Pro Controller / Joy-Con (HID, blocking reads, multi-device);
//! * 8BitDo Ultimate 2 Wireless in DInput mode (HID, blocking reads);
//! * XInput slots (polled);
//! * Bluetooth LE battery properties (polled).
//!
//! All HID consumers share a short-lived enumeration cache so the system HID
//! tree is not rescanned independently by every background thread.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use controller_battery_core::dualsense::{self, DecodedReport as DualSenseReport};
use controller_battery_core::dualshock4::{self, DecodedReport as DualShock4Report};
use controller_battery_core::eightbitdo;
use controller_battery_core::hid::{self, HidDevice, HidInterface};
use controller_battery_core::model::{
    report_fingerprint, BatteryReading, ChargeState, Freshness, Precision, Transport,
};
use controller_battery_core::switch_pro;
use controller_battery_core::xinput;

/// How long an entry stays visible after its last update.
const ENTRY_TTL: Duration = Duration::from_secs(10);
/// A reading older than this is shown as "idle". The HID streams report
/// continuously while in use and stop when idle, so one second is "as soon as
/// it stops" without flickering on a dropped frame.
pub const STALE_AFTER: Duration = Duration::from_secs(1);
/// XInput is polled every 800 ms and enumeration occasionally takes longer;
/// keep a little more slack so a connected pad never blinks to "idle".
const XINPUT_STALE_AFTER: Duration = Duration::from_secs(2);
/// A HID device missing from one enumeration pass is not necessarily gone;
/// wait this long before dropping its row so a hiccup does not make it blink.
const HID_GONE_GRACE: Duration = Duration::from_secs(1);
/// Bluetooth battery values are polled, so they may be older than that.
const BLE_STALE_AFTER: Duration = Duration::from_secs(20);
/// Heartbeat cadence for high-frequency HID streams (prevents 1 kHz lock churn).
const HID_HEARTBEAT: Duration = Duration::from_millis(500);
/// Shared HID enumeration cache lifetime.
const HID_CACHE_TTL: Duration = Duration::from_millis(450);

static HID_CACHE: Mutex<Option<(Instant, Vec<HidInterface>)>> = Mutex::new(None);

/// Fallback device name when a hardware HID product string is unavailable.
/// Kept as structured data so the UI and tray can localize it on language change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackName {
    Xbox(u32),
    EightBitDo,
    DualSense,
    DualShock4,
    SwitchPro,
    JoyConL,
    JoyConR,
}

#[derive(Debug, Clone)]
pub struct DeviceState {
    pub reading: BatteryReading,
    pub degraded: bool,
    pub last_update: Instant,
    /// How long a reading stays "live". Polled sources refresh less often than
    /// the HID stream, so each source passes its own value.
    pub stale_after: Duration,
    /// Localized fallback name used when no hardware product string was found.
    pub fallback_name: Option<FallbackName>,
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

/// Poison-safe lock helper: background collector threads must never panic.
pub fn lock_shared(state: &Arc<Mutex<Shared>>) -> MutexGuard<'_, Shared> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_mutex<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Cached view of all HID interfaces so multiple collector threads share one
/// SetupAPI scan instead of repeatedly opening every HID device on the system.
fn cached_hid_interfaces() -> Vec<HidInterface> {
    let mut cache = lock_mutex(&HID_CACHE);
    if let Some((at, ref list)) = *cache {
        if at.elapsed() < HID_CACHE_TTL {
            return list.clone();
        }
    }
    let fresh = hid::enumerate_all();
    *cache = Some((Instant::now(), fresh.clone()));
    fresh
}

/// Deduplicate HID interfaces belonging to the same physical controller (using
/// serial number when available), preferring USB over Bluetooth.
fn dedup_prefer_usb(mut interfaces: Vec<HidInterface>) -> Vec<(String, HidInterface)> {
    interfaces.sort_by_key(|i| i.transport() == Transport::Bluetooth);

    let mut seen_ids = HashSet::new();
    let mut out = Vec::with_capacity(interfaces.len());
    for interface in interfaces {
        let id = hid::serial_number(&interface.path)
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| interface.path.to_ascii_lowercase());
        if seen_ids.insert(id.clone()) {
            out.push((id, interface));
        }
    }
    out
}

impl Shared {
    fn upsert(
        &mut self,
        key: &str,
        reading: BatteryReading,
        degraded: bool,
        stale_after: Duration,
        fallback_name: Option<FallbackName>,
    ) {
        self.devices.insert(
            key.to_string(),
            DeviceState {
                reading,
                degraded,
                last_update: Instant::now(),
                stale_after,
                fallback_name,
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
    spawn_dualshock4(state.clone());
    spawn_switch_pro(state.clone());
    spawn_eightbitdo(state.clone());
    spawn_xinput(state.clone());
    spawn_bluetooth(state);
}

/// Remove keys with `prefix` that have been absent from enumeration for at
/// least `HID_GONE_GRACE`.
fn prune_missing_hid_keys(
    state: &Arc<Mutex<Shared>>,
    prefix: &str,
    present_keys: &HashSet<String>,
    missing_since: &mut HashMap<String, Instant>,
) {
    let existing: Vec<String> = {
        let shared = lock_shared(state);
        shared
            .devices
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    };

    for key in present_keys {
        missing_since.remove(key);
    }
    missing_since.retain(|k, _| existing.contains(k));

    let now = Instant::now();
    let mut to_remove = Vec::new();
    for key in existing {
        if !present_keys.contains(&key) {
            let since = *missing_since.entry(key.clone()).or_insert(now);
            if now.duration_since(since) >= HID_GONE_GRACE {
                to_remove.push(key);
            }
        }
    }

    if !to_remove.is_empty() {
        let mut shared = lock_shared(state);
        for key in to_remove {
            missing_since.remove(&key);
            shared.remove(&key);
        }
    }
}

fn spawn_dualsense(state: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || {
        let owners: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
        let running: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let mut missing_since: HashMap<String, Instant> = HashMap::new();

        loop {
            let interfaces: Vec<HidInterface> = cached_hid_interfaces()
                .into_iter()
                .filter(|i| {
                    i.vendor_id == dualsense::SONY_VENDOR_ID
                        && dualsense::is_supported_product(i.product_id)
                })
                .collect();
            let controllers = dedup_prefer_usb(interfaces);

            let mut present_keys = HashSet::new();
            for (id, interface) in &controllers {
                let key = format!("dualsense:{id}");
                present_keys.insert(key.clone());
                lock_mutex(&owners).insert(key, interface.path.clone());
            }

            if controllers.is_empty() {
                // Controller unplugged: clear any stale "open failed" error.
                let mut shared = lock_shared(&state);
                if shared.error == Some(ErrorKind::DualSenseOpenFailed) {
                    shared.error = None;
                }
            }

            prune_missing_hid_keys(&state, "dualsense:", &present_keys, &mut missing_since);

            for (id, interface) in controllers {
                let key = format!("dualsense:{id}");
                let path = interface.path.clone();
                if !lock_mutex(&running).insert(path.clone()) {
                    continue;
                }

                let state = state.clone();
                let owners = owners.clone();
                let running = running.clone();
                std::thread::spawn(move || {
                    run_dualsense_reader(state, owners, key, interface);
                    lock_mutex(&running).remove(&path);
                });
            }

            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

fn run_dualsense_reader(
    state: Arc<Mutex<Shared>>,
    owners: Arc<Mutex<HashMap<String, String>>>,
    key: String,
    controller: HidInterface,
) {
    let Some(device) = HidDevice::open(&controller) else {
        lock_shared(&state).error = Some(ErrorKind::DualSenseOpenFailed);
        std::thread::sleep(Duration::from_millis(1500));
        return;
    };

    let (display_name, fallback_name) = match hid::product_name(&controller.path) {
        Some(name) => (clean_device_name(&name), None),
        None => ("DualSense".to_string(), Some(FallbackName::DualSense)),
    };

    let mut buffer = [0u8; 128];
    let mut asked_for_full_report = false;
    let mut last_fingerprint: Option<u64> = None;
    let mut last_published: Option<(DualSenseReport, Instant)> = None;

    while let Some(read) = device.read_report(&mut buffer) {
        // If USB took over this controller's key, retire the Bluetooth reader.
        if lock_mutex(&owners).get(&key).map(String::as_str) != Some(controller.path.as_str()) {
            break;
        }

        let slice = &buffer[..read];
        let Some(decoded) = dualsense::decode_input_report(slice) else {
            continue;
        };

        // The controller only sends the full report (with live battery data)
        // once a host has written to it.
        if decoded.degraded && !asked_for_full_report {
            asked_for_full_report = device.write_report(&dualsense::noop_output_report());
        }

        // Windows repeats the last buffered report verbatim once the pad goes
        // idle; skip identical frames so `is_stale()` triggers accurately.
        let fingerprint = report_fingerprint(slice);
        if last_fingerprint == Some(fingerprint) {
            continue;
        }
        last_fingerprint = Some(fingerprint);

        let now = Instant::now();
        let unchanged = last_published
            .is_some_and(|(last, at)| last == decoded && now.duration_since(at) < HID_HEARTBEAT);
        if unchanged {
            continue;
        }
        last_published = Some((decoded, now));

        let reading = reading_from_dualsense(&decoded, &display_name);
        let mut shared = lock_shared(&state);
        shared.upsert(&key, reading, decoded.degraded, STALE_AFTER, fallback_name);
        if shared.error == Some(ErrorKind::DualSenseOpenFailed) {
            shared.error = None;
        }
    }

    std::thread::sleep(Duration::from_millis(600));
}

fn spawn_dualshock4(state: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || {
        let owners: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
        let running: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let mut missing_since: HashMap<String, Instant> = HashMap::new();

        loop {
            let interfaces: Vec<HidInterface> = cached_hid_interfaces()
                .into_iter()
                .filter(|i| dualshock4::is_supported_product(i.vendor_id, i.product_id))
                .collect();
            let controllers = dedup_prefer_usb(interfaces);

            let mut present_keys = HashSet::new();
            for (id, interface) in &controllers {
                let key = format!("dualshock4:{id}");
                present_keys.insert(key.clone());
                lock_mutex(&owners).insert(key, interface.path.clone());
            }

            prune_missing_hid_keys(&state, "dualshock4:", &present_keys, &mut missing_since);

            for (id, interface) in controllers {
                let key = format!("dualshock4:{id}");
                let path = interface.path.clone();
                if !lock_mutex(&running).insert(path.clone()) {
                    continue;
                }

                let state = state.clone();
                let owners = owners.clone();
                let running = running.clone();
                std::thread::spawn(move || {
                    run_dualshock4_reader(state, owners, key, interface);
                    lock_mutex(&running).remove(&path);
                });
            }

            std::thread::sleep(Duration::from_millis(700));
        }
    });
}

fn run_dualshock4_reader(
    state: Arc<Mutex<Shared>>,
    owners: Arc<Mutex<HashMap<String, String>>>,
    key: String,
    controller: HidInterface,
) {
    let Some(device) = HidDevice::open(&controller) else {
        std::thread::sleep(Duration::from_millis(1200));
        return;
    };

    // Reading feature report 0x02 switches a Bluetooth DualShock 4 from its
    // simplified 0x01 report into the full 0x11 report that carries battery.
    if device.transport == Transport::Bluetooth {
        let mut feature = [0u8; 37];
        feature[0] = 0x02;
        let _ = device.get_feature(&mut feature);
    }

    let (display_name, fallback_name) = match hid::product_name(&controller.path) {
        Some(name) => (clean_device_name(&name), None),
        None => ("DualShock 4".to_string(), Some(FallbackName::DualShock4)),
    };

    let mut buffer = [0u8; 128];
    let mut last_fingerprint: Option<u64> = None;
    let mut last_published: Option<(DualShock4Report, Instant)> = None;

    while let Some(read) = device.read_report(&mut buffer) {
        if lock_mutex(&owners).get(&key).map(String::as_str) != Some(controller.path.as_str()) {
            break;
        }

        let slice = &buffer[..read];
        let Some(decoded) = dualshock4::decode_input_report(slice) else {
            continue;
        };

        let fingerprint = report_fingerprint(slice);
        if last_fingerprint == Some(fingerprint) {
            continue;
        }
        last_fingerprint = Some(fingerprint);

        let now = Instant::now();
        let unchanged = last_published
            .is_some_and(|(last, at)| last == decoded && now.duration_since(at) < HID_HEARTBEAT);
        if unchanged {
            continue;
        }
        last_published = Some((decoded, now));

        let reading = BatteryReading {
            device: display_name.clone(),
            transport: decoded.transport,
            percent: Some(decoded.percent),
            state: decoded.state,
            precision: decoded.precision,
            freshness: Freshness::Live,
            note: None,
        };
        lock_shared(&state).upsert(&key, reading, false, STALE_AFTER, fallback_name);
    }

    std::thread::sleep(Duration::from_millis(600));
}

fn spawn_switch_pro(state: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || {
        let owners: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
        let running: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let mut missing_since: HashMap<String, Instant> = HashMap::new();

        loop {
            let interfaces: Vec<HidInterface> = cached_hid_interfaces()
                .into_iter()
                .filter(|i| switch_pro::is_supported_product(i.vendor_id, i.product_id))
                .collect();
            let controllers = dedup_prefer_usb(interfaces);

            let mut present_keys = HashSet::new();
            for (id, interface) in &controllers {
                let key = format!("switch:{id}");
                present_keys.insert(key.clone());
                lock_mutex(&owners).insert(key, interface.path.clone());
            }

            prune_missing_hid_keys(&state, "switch:", &present_keys, &mut missing_since);

            for (id, interface) in controllers {
                let key = format!("switch:{id}");
                let path = interface.path.clone();
                if !lock_mutex(&running).insert(path.clone()) {
                    continue;
                }

                let state = state.clone();
                let owners = owners.clone();
                let running = running.clone();
                std::thread::spawn(move || {
                    run_switch_pro_reader(state, owners, key, interface);
                    lock_mutex(&running).remove(&path);
                });
            }

            std::thread::sleep(Duration::from_millis(700));
        }
    });
}

fn run_switch_pro_reader(
    state: Arc<Mutex<Shared>>,
    owners: Arc<Mutex<HashMap<String, String>>>,
    key: String,
    interface: HidInterface,
) {
    let Some(device) = HidDevice::open_with_access(&interface, false) else {
        std::thread::sleep(Duration::from_millis(1000));
        return;
    };

    let default_fallback = match interface.product_id {
        switch_pro::JOYCON_L_PID => FallbackName::JoyConL,
        switch_pro::JOYCON_R_PID => FallbackName::JoyConR,
        _ => FallbackName::SwitchPro,
    };
    let (display_name, fallback_name) = match hid::product_name(&interface.path) {
        Some(name) => (clean_device_name(&name), None),
        None => ("Switch Pro Controller".to_string(), Some(default_fallback)),
    };

    let mut buffer = [0u8; 128];
    let mut last_fingerprint: Option<u64> = None;
    let mut last_published: Option<(switch_pro::PowerState, Instant)> = None;

    while let Some(read) = device.read_report(&mut buffer) {
        if lock_mutex(&owners).get(&key).map(String::as_str) != Some(interface.path.as_str()) {
            break;
        }

        let slice = &buffer[..read];
        let Some(power) = switch_pro::decode_input_report(slice) else {
            continue;
        };

        let fingerprint = report_fingerprint(slice);
        if last_fingerprint == Some(fingerprint) {
            continue;
        }
        last_fingerprint = Some(fingerprint);

        let now = Instant::now();
        let unchanged = last_published
            .is_some_and(|(last, at)| last == power && now.duration_since(at) < HID_HEARTBEAT);
        if unchanged {
            continue;
        }
        last_published = Some((power, now));

        let reading = BatteryReading {
            device: display_name.clone(),
            transport: device.transport,
            percent: Some(power.percent),
            state: power.state,
            precision: power.precision,
            freshness: Freshness::Live,
            note: None,
        };
        lock_shared(&state).upsert(&key, reading, false, STALE_AFTER, fallback_name);
    }

    std::thread::sleep(Duration::from_millis(600));
}

/// 8BitDo Ultimate 2 Wireless on its USB dongle in DInput mode.
///
/// In XInput mode (the pad's default when powered on normally) the same dongle
/// uses PID `0x310B` and reports no battery at all; in DInput mode (hold `B`
/// while powering on) it becomes this HID gamepad and the input report carries
/// an exact percentage.
fn spawn_eightbitdo(state: Arc<Mutex<Shared>>) {
    let mut missing_since: Option<Instant> = None;
    std::thread::spawn(move || loop {
        let interface = cached_hid_interfaces()
            .into_iter()
            .find(eightbitdo::is_supported);

        let Some(interface) = interface else {
            // Same as the DualSense thread: keep a short grace against a
            // single failed enumeration, then drop the row.
            match missing_since {
                Some(since) if since.elapsed() >= HID_GONE_GRACE => {
                    lock_shared(&state).remove("8bitdo");
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

        let (name, fallback_name) = match hid::product_name(&interface.path) {
            Some(name) => (clean_device_name(&name), None),
            None => (
                "8BitDo Controller".to_string(),
                Some(FallbackName::EightBitDo),
            ),
        };

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
            let unchanged = last_published
                .is_some_and(|(last, at)| last == power && now.duration_since(at) < HID_HEARTBEAT);
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
            lock_shared(&state).upsert("8bitdo", reading, false, STALE_AFTER, fallback_name);
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
                // Sony is deliberately absent: DualSense/DualShock have their
                // own HID backends and must not be borrowed for an XInput slot.
    ];

    let mut names: Vec<String> = Vec::new();
    for interface in cached_hid_interfaces() {
        if !VENDORS.contains(&interface.vendor_id) {
            continue;
        }
        if let Some(name) = hid::product_name(&interface.path) {
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

            let mut active_index = 0usize;
            for slot in 0..xinput::MAX_SLOTS {
                let key = format!("xinput:{slot}");
                match xinput::poll_slot(slot) {
                    Some(mut reading) => {
                        // Assign distinct discovered HID names per connected
                        // slot; fall back to a localized "Xbox Controller N"
                        // name instead of cloning the first name across pads.
                        let fallback = if let Some(name) = names.get(active_index) {
                            reading.device = clean_device_name(name);
                            None
                        } else {
                            Some(FallbackName::Xbox(slot + 1))
                        };
                        active_index += 1;

                        let mut shared = lock_shared(&state);
                        shared.upsert(&key, reading, false, XINPUT_STALE_AFTER, fallback);
                    }
                    None => {
                        lock_shared(&state).remove(&key);
                    }
                }
            }
            lock_shared(&state).prune();
            std::thread::sleep(Duration::from_millis(800));
        }
    });
}

/// Bluetooth LE controllers expose their battery through the standard Battery
/// Service; Windows publishes it as a device property, which we poll.
fn spawn_bluetooth(state: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || loop {
        let mut seen: Vec<String> = Vec::new();
        for device in controller_battery_core::bluetooth::enumerate_filtered(
            controller_battery_core::bluetooth::looks_like_controller,
        ) {
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
            lock_shared(&state).upsert(&key, reading, false, BLE_STALE_AFTER, None);
        }

        // A device that is no longer paired/connected should disappear promptly.
        let mut shared = lock_shared(&state);
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

        std::thread::sleep(Duration::from_secs(2));
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

fn reading_from_dualsense(decoded: &DualSenseReport, name: &str) -> BatteryReading {
    BatteryReading {
        device: clean_device_name(name),
        transport: decoded.transport,
        percent: Some(decoded.percent),
        state: decoded.state,
        precision: Precision::Buckets10,
        freshness: Freshness::Live,
        note: None,
    }
}

/// A stable, ordered snapshot for the UI.
///
/// Filtering happens here, on every frame, so a pad that switches from
/// Bluetooth to its 2.4G dongle or USB loses its stale Bluetooth row
/// immediately instead of waiting for the next Bluetooth poll.
pub fn snapshot(state: &Arc<Mutex<Shared>>) -> Vec<(String, DeviceState)> {
    let mut shared = lock_shared(state);
    shared.prune();

    let primary_names: Vec<String> = shared
        .devices
        .iter()
        .filter(|(key, _)| !key.starts_with("ble:"))
        .map(|(_, value)| value.reading.device.to_ascii_lowercase())
        .collect();

    shared
        .devices
        .iter()
        .filter(|(key, value)| {
            if !key.starts_with("ble:") {
                return true;
            }
            let name = value.reading.device.to_ascii_lowercase();
            !primary_names
                .iter()
                .any(|other| !other.is_empty() && (other.contains(&name) || name.contains(other)))
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
                "{key} -> name={:?} fallback={:?} transport={:?} percent={:?} state={:?} stale={}",
                state.reading.device,
                state.fallback_name,
                state.reading.transport,
                state.reading.percent,
                state.reading.state,
                state.is_stale()
            );
        }
        assert!(!snapshot.is_empty(), "no device detected");
    }
}
