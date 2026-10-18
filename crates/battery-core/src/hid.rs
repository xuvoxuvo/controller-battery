//! Windows HID access (SetupAPI + hid.dll).
//!
//! Only the pieces needed by the battery widget are implemented: enumerate HID
//! interfaces, keep a handle open and read input reports. Reads are blocking;
//! the UI derives "stale" from the timestamp of the last report.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows::Win32::Devices::HumanInterfaceDevice::{HidD_GetAttributes, HIDD_ATTRIBUTES};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, HWND};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};

use crate::dualsense::{is_supported_product, SONY_VENDOR_ID};
use crate::model::Transport;

/// `{4D1E55B2-F16F-11CF-88CB-001111000030}` — GUID_DEVINTERFACE_HID.
const GUID_DEVINTERFACE_HID: GUID = GUID::from_u128(0x4d1e55b2_f16f_11cf_88cb_001111000030);

/// Any HID interface present on the system.
#[derive(Debug, Clone)]
pub struct HidInterface {
    pub path: String,
    pub vendor_id: u16,
    pub product_id: u16,
}

impl HidInterface {
    pub fn transport(&self) -> Transport {
        classify_transport(&self.path)
    }
}

/// A supported PlayStation controller.
pub type HidController = HidInterface;

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// Enumerate every HID interface, with vendor/product ids resolved.
pub fn enumerate_all() -> Vec<HidInterface> {
    let mut found = Vec::new();

    unsafe {
        let dev_info = match SetupDiGetClassDevsW(
            Some(&GUID_DEVINTERFACE_HID),
            PCWSTR::null(),
            HWND::default(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        ) {
            Ok(handle) => handle,
            Err(_) => return found,
        };

        let mut index = 0u32;
        loop {
            let mut interface_data = SP_DEVICE_INTERFACE_DATA {
                cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };

            if SetupDiEnumDeviceInterfaces(
                dev_info,
                None,
                &GUID_DEVINTERFACE_HID,
                index,
                &mut interface_data,
            )
            .is_err()
            {
                break;
            }
            index += 1;

            // Ask for the required buffer size, then fetch the interface path.
            let mut required = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(
                dev_info,
                &interface_data,
                None,
                0,
                Some(&mut required),
                None,
            );
            if required == 0 {
                continue;
            }

            let mut buffer = vec![0u8; required as usize];
            let detail = buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;

            if SetupDiGetDeviceInterfaceDetailW(
                dev_info,
                &interface_data,
                Some(detail),
                required,
                None,
                None,
            )
            .is_err()
            {
                continue;
            }

            // `DevicePath` is a flexible array at offset 4 of the detail struct.
            let path_units = std::slice::from_raw_parts(
                buffer[4..].as_ptr() as *const u16,
                (buffer.len() - 4) / 2,
            );
            let end = path_units.iter().position(|&c| c == 0).unwrap_or(0);
            let path = String::from_utf16_lossy(&path_units[..end]);

            if let Some(interface) = probe(&path) {
                found.push(interface);
            }
        }

        let _ = SetupDiDestroyDeviceInfoList(dev_info);
    }

    found
}

fn probe(path: &str) -> Option<HidInterface> {
    unsafe {
        let handle = CreateFileW(
            PCWSTR(wide(path).as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .ok()?;

        let mut attributes = HIDD_ATTRIBUTES {
            Size: std::mem::size_of::<HIDD_ATTRIBUTES>() as u32,
            ..Default::default()
        };
        let ok = HidD_GetAttributes(handle, &mut attributes).as_bool();
        let _ = CloseHandle(handle);

        if !ok {
            return None;
        }

        Some(HidInterface {
            path: path.to_string(),
            vendor_id: attributes.VendorID,
            product_id: attributes.ProductID,
        })
    }
}

/// Enumerate all connected DualSense / DualSense Edge interfaces.
///
/// When the controller is connected over both USB and Bluetooth, both entries
/// are returned; callers should prefer the USB one.
pub fn enumerate_dualsense() -> Vec<HidController> {
    enumerate_all()
        .into_iter()
        .filter(|i| i.vendor_id == SONY_VENDOR_ID && is_supported_product(i.product_id))
        .collect()
}

/// Bluetooth HID interfaces carry the Bluetooth HID service GUID in their path.
fn classify_transport(path: &str) -> Transport {
    if path.to_ascii_lowercase().contains("00001124") {
        Transport::Bluetooth
    } else {
        Transport::Usb
    }
}

/// An open controller handle.
pub struct HidDevice {
    handle: HANDLE,
    pub transport: Transport,
}

impl HidDevice {
    /// Open a controller for reading input reports.
    pub fn open(controller: &HidController) -> Option<Self> {
        unsafe {
            let handle = CreateFileW(
                PCWSTR(wide(&controller.path).as_ptr()),
                GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
            .ok()?;
            Some(Self {
                handle,
                transport: controller.transport(),
            })
        }
    }

    /// Blocking read of one input report. Returns the number of bytes read.
    pub fn read_report(&self, buffer: &mut [u8]) -> Option<usize> {
        let mut read = 0u32;
        unsafe {
            ReadFile(self.handle, Some(buffer), Some(&mut read), None).ok()?;
        }
        Some(read as usize)
    }
}

impl Drop for HidDevice {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}
