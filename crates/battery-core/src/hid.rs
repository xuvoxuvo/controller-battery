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
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidD_GetAttributes, HidD_GetFeature, HIDD_ATTRIBUTES,
};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, HWND};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
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
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
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
    ///
    /// Write access is requested first: the DualSense only switches to the
    /// full (`0x31`) Bluetooth report format once a host opens it for writing.
    /// If write access is denied (another process owns the device), fall back
    /// to read-only.
    pub fn open(controller: &HidController) -> Option<Self> {
        Self::open_with_access(controller, true)
            .or_else(|| Self::open_with_access(controller, false))
    }

    /// Open a controller, optionally requesting write access (needed to send
    /// output reports such as "please use the full report format").
    pub fn open_with_access(controller: &HidController, write: bool) -> Option<Self> {
        unsafe {
            let access = if write {
                GENERIC_READ.0 | GENERIC_WRITE.0
            } else {
                GENERIC_READ.0
            };
            let handle = CreateFileW(
                PCWSTR(wide(&controller.path).as_ptr()),
                access,
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

    /// Send one output report (e.g. a no-op configuration report).
    pub fn write_report(&self, data: &[u8]) -> bool {
        let mut written = 0u32;
        unsafe {
            WriteFile(self.handle, Some(data), Some(&mut written), None).is_ok()
                && written as usize == data.len()
        }
    }

    /// Read a feature report. The first byte of `buffer` must contain the
    /// report id; returns the number of bytes the device filled in.
    pub fn get_feature(&self, buffer: &mut [u8]) -> Option<usize> {
        let ok = unsafe {
            HidD_GetFeature(
                self.handle,
                buffer.as_mut_ptr() as *mut _,
                buffer.len() as u32,
            )
        };
        ok.as_bool().then_some(buffer.len())
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

// The handle is owned by this value and may be moved to another thread; it is
// never shared for concurrent reads.
unsafe impl Send for HidDevice {}

impl Drop for HidDevice {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}
