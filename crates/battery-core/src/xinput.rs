//! Xbox-compatible controllers via XInput (Windows 10+).
//!
//! XInput only exposes a coarse battery level (empty / low / medium / full)
//! plus a battery type (wired / alkaline / NiMH), which is what most
//! "Xbox-compatible" controllers — including 8BitDo pads in XInput mode —
//! report. The percentage is therefore an approximation.

use windows::Win32::UI::Input::XboxController::{
    XInputGetBatteryInformation, XInputGetState, BATTERY_DEVTYPE_GAMEPAD, BATTERY_LEVEL_EMPTY,
    BATTERY_LEVEL_FULL, BATTERY_LEVEL_LOW, BATTERY_LEVEL_MEDIUM, BATTERY_TYPE_ALKALINE,
    BATTERY_TYPE_DISCONNECTED, BATTERY_TYPE_NIMH, BATTERY_TYPE_WIRED, XINPUT_BATTERY_INFORMATION,
    XINPUT_STATE,
};

use crate::model::{BatteryReading, ChargeState, Freshness, Precision, Transport};

/// Maximum number of XInput slots.
pub const MAX_SLOTS: u32 = 4;

/// Poll every XInput slot and return the first connected controller.
pub fn poll() -> Option<BatteryReading> {
    for slot in 0..MAX_SLOTS {
        let mut state = XINPUT_STATE::default();
        let result = unsafe { XInputGetState(slot, &mut state) };
        if result != 0 {
            continue;
        }

        let mut battery = XINPUT_BATTERY_INFORMATION::default();
        let result =
            unsafe { XInputGetBatteryInformation(slot, BATTERY_DEVTYPE_GAMEPAD, &mut battery) };
        if result != 0 {
            continue;
        }

        return Some(reading_from_slot(slot, &battery));
    }
    None
}

fn reading_from_slot(slot: u32, battery: &XINPUT_BATTERY_INFORMATION) -> BatteryReading {
    // Third-party pads in XInput mode often report "wired" without exposing a
    // battery level at all, so do not pretend to know the percentage.
    let (percent, state, transport) = if battery.BatteryType == BATTERY_TYPE_WIRED {
        (None, ChargeState::Wired, Transport::Usb)
    } else if battery.BatteryType == BATTERY_TYPE_DISCONNECTED {
        (None, ChargeState::Unknown, Transport::Unknown)
    } else {
        let percent = if battery.BatteryLevel == BATTERY_LEVEL_EMPTY {
            5
        } else if battery.BatteryLevel == BATTERY_LEVEL_LOW {
            25
        } else if battery.BatteryLevel == BATTERY_LEVEL_MEDIUM {
            60
        } else if battery.BatteryLevel == BATTERY_LEVEL_FULL {
            100
        } else {
            50
        };
        (Some(percent), ChargeState::Discharging, Transport::Unknown)
    };

    let kind = if battery.BatteryType == BATTERY_TYPE_ALKALINE {
        Some("碱性电池".to_string())
    } else if battery.BatteryType == BATTERY_TYPE_NIMH {
        Some("镍氢电池".to_string())
    } else {
        None
    };

    BatteryReading {
        device: format!("Xbox 手柄 · 槽位 {}", slot + 1),
        transport,
        percent,
        state,
        precision: Precision::Coarse,
        freshness: Freshness::Live,
        note: kind,
    }
}

/// Number of connected XInput slots.
pub fn connected_slots() -> usize {
    (0..MAX_SLOTS)
        .filter(|slot| {
            let mut state = XINPUT_STATE::default();
            let result = unsafe { XInputGetState(*slot, &mut state) };
            result == 0
        })
        .count()
}

/// Debug helper: describe one slot, including raw battery fields.
pub fn debug_slot(slot: u32) -> Option<String> {
    let mut state = XINPUT_STATE::default();
    let connected = unsafe { XInputGetState(slot, &mut state) };
    if connected != 0 {
        return None;
    }

    let mut battery = XINPUT_BATTERY_INFORMATION::default();
    let result =
        unsafe { XInputGetBatteryInformation(slot, BATTERY_DEVTYPE_GAMEPAD, &mut battery) };

    Some(format!(
        "connected, batteryType=0x{:02X}, batteryLevel=0x{:02X} (call result {})",
        battery.BatteryType.0, battery.BatteryLevel.0, result
    ))
}
