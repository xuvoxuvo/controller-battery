//! Bluetooth LE battery levels.
//!
//! Many wireless controllers expose the standard BLE Battery Service
//! (`0x180F`). Windows already reads it and publishes the value as the
//! `DEVPKEY_Bluetooth_Battery` device property, so instead of speaking GATT
//! ourselves we simply ask the PnP manager — this is the same number the
//! Windows "Bluetooth & devices" page shows.

use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDeviceInstanceIdW, SetupDiGetDevicePropertyW, DIGCF_PRESENT, SP_DEVINFO_DATA,
};
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_FriendlyName, DEVPROPKEY, DEVPROP_TYPE_BYTE, DEVPROP_TYPE_STRING,
};
use windows::Win32::Foundation::HWND;

use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothLEDevice};

/// `{e0cbf06c-cd8b-4647-bb8a-263b43f0f974}` — GUID_DEVCLASS_BLUETOOTH.
const GUID_DEVCLASS_BLUETOOTH: GUID = GUID::from_u128(0xe0cbf06c_cd8b_4647_bb8a_263b43f0f974);

/// `{104EA319-6EE2-4701-BD47-8DDBF425BBE5}, 2` — DEVPKEY_Bluetooth_Battery.
const DEVPKEY_BLUETOOTH_BATTERY: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
    pid: 2,
};

/// Ask the Bluetooth stack whether the device is actually connected.
///
/// A paired device keeps its PnP node (and its last battery value) while it is
/// switched off, so presence alone is not enough — this is what makes the row
/// disappear as soon as the controller disconnects.
fn is_connected(instance_id: &str) -> bool {
    let Some(address) = address_from_instance_id(instance_id) else {
        return false;
    };

    let operation = match BluetoothLEDevice::FromBluetoothAddressAsync(address) {
        Ok(operation) => operation,
        Err(_) => return false,
    };
    let device = match operation.get() {
        Ok(device) => device,
        Err(_) => return false,
    };

    matches!(
        device.ConnectionStatus(),
        Ok(BluetoothConnectionStatus::Connected)
    )
}

/// Diagnostic: connection status for an arbitrary address value.
pub fn probe_address(address: u64) -> bool {
    let Ok(operation) = BluetoothLEDevice::FromBluetoothAddressAsync(address) else {
        return false;
    };
    let Ok(device) = operation.get() else {
        return false;
    };
    matches!(
        device.ConnectionStatus(),
        Ok(BluetoothConnectionStatus::Connected)
    )
}

/// `BTHLE\DEV_E417D8BC7BA4\...` -> the 48 bit address.
fn address_from_instance_id(instance_id: &str) -> Option<u64> {
    let start = instance_id.find("DEV_")? + 4;
    let hex = instance_id.get(start..start + 12)?;

    let mut bytes = [0u8; 6];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
    }
    // `BluetoothLEDevice` expects the address as a plain 48 bit value.
    Some(u64::from_be_bytes([
        0, 0, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
    ]))
}

/// Diagnostic view: every battery-capable Bluetooth device and its link state.
#[derive(Debug, Clone)]
pub struct BleProbe {
    pub name: String,
    pub instance_id: String,
    pub address: Option<u64>,
    pub percent: u8,
    pub connected: bool,
}

/// Like [`enumerate`], but without dropping disconnected devices.
pub fn probe_all() -> Vec<BleProbe> {
    let mut found = Vec::new();

    unsafe {
        let set = match SetupDiGetClassDevsW(
            Some(&GUID_DEVCLASS_BLUETOOTH),
            PCWSTR::null(),
            HWND::default(),
            DIGCF_PRESENT,
        ) {
            Ok(handle) => handle,
            Err(_) => return found,
        };

        let mut index = 0u32;
        loop {
            let mut info = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInfo(set, index, &mut info).is_err() {
                break;
            }
            index += 1;

            let Some(percent) = battery_of(set, &info) else {
                continue;
            };
            let Some(name) = string_property(set, &info, &DEVPKEY_Device_FriendlyName) else {
                continue;
            };
            let instance_id = instance_id(set, &info).unwrap_or_default();
            found.push(BleProbe {
                name,
                address: address_from_instance_id(&instance_id),
                connected: is_connected(&instance_id),
                instance_id,
                percent,
            });
        }

        let _ = SetupDiDestroyDeviceInfoList(set);
    }

    found
}

/// A Bluetooth device that reports a battery percentage.
#[derive(Debug, Clone)]
pub struct BleBattery {
    pub instance_id: String,
    pub name: String,
    pub percent: u8,
}

/// Enumerate Bluetooth devices that publish a battery level.
pub fn enumerate() -> Vec<BleBattery> {
    let mut found = Vec::new();

    unsafe {
        let set = match SetupDiGetClassDevsW(
            Some(&GUID_DEVCLASS_BLUETOOTH),
            PCWSTR::null(),
            HWND::default(),
            DIGCF_PRESENT,
        ) {
            Ok(handle) => handle,
            Err(_) => return found,
        };

        let mut index = 0u32;
        loop {
            let mut info = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInfo(set, index, &mut info).is_err() {
                break;
            }
            index += 1;

            let Some(percent) = battery_of(set, &info) else {
                continue;
            };
            let Some(name) = string_property(set, &info, &DEVPKEY_Device_FriendlyName) else {
                continue;
            };
            let instance_id = instance_id(set, &info).unwrap_or_default();
            if !is_connected(&instance_id) {
                continue;
            }

            found.push(BleBattery {
                instance_id,
                name,
                percent,
            });
        }

        let _ = SetupDiDestroyDeviceInfoList(set);
    }

    found
}

fn battery_of(
    set: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
    info: &SP_DEVINFO_DATA,
) -> Option<u8> {
    let mut value = [0u8; 1];
    let mut kind = DEVPROP_TYPE_BYTE;

    unsafe {
        SetupDiGetDevicePropertyW(
            set,
            info,
            &DEVPKEY_BLUETOOTH_BATTERY,
            &mut kind,
            Some(&mut value),
            None,
            0,
        )
        .ok()?;
    }

    let percent = value[0];
    (kind == DEVPROP_TYPE_BYTE && percent <= 100).then_some(percent)
}

fn string_property(
    set: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
    info: &SP_DEVINFO_DATA,
    key: &DEVPROPKEY,
) -> Option<String> {
    let mut buffer = [0u16; 256];
    let mut kind = DEVPROP_TYPE_STRING;

    unsafe {
        let bytes =
            std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut u8, buffer.len() * 2);
        SetupDiGetDevicePropertyW(set, info, key, &mut kind, Some(bytes), None, 0).ok()?;
    }

    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

fn instance_id(
    set: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
    info: &SP_DEVINFO_DATA,
) -> Option<String> {
    let mut buffer = [0u16; 512];
    let mut required = 0u32;
    unsafe {
        SetupDiGetDeviceInstanceIdW(set, info, Some(&mut buffer), Some(&mut required)).ok()?;
    }
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

/// Heuristic: does the device name look like a game controller?
pub fn looks_like_controller(name: &str) -> bool {
    const KEYWORDS: [&str; 10] = [
        "controller",
        "gamepad",
        "8bitdo",
        "dualsense",
        "dualshock",
        "xbox",
        "switch",
        "joy-con",
        "手柄",
        "手柄",
    ];
    let lower = name.to_ascii_lowercase();
    KEYWORDS.iter().any(|k| lower.contains(k)) || name.contains("手柄")
}
