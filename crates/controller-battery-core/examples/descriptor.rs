//! Dump the HID report descriptor field layout of matching interfaces.
//!
//! Usage: cargo run -p controller-battery-core --example descriptor -- <vid_hex> [pid_hex]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use controller_battery_core::hid::enumerate_all;
use windows::core::PCWSTR;
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetPreparsedData, HidP_GetButtonCaps, HidP_GetCaps,
    HidP_GetValueCaps, HidP_Input, HidP_Output, HIDP_BUTTON_CAPS, HIDP_STATUS_SUCCESS,
    HIDP_VALUE_CAPS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn parse_hex(value: &str) -> Option<u16> {
    u16::from_str_radix(value.trim_start_matches("0x"), 16).ok()
}

fn dump_caps(path: &str) {
    unsafe {
        let Ok(handle) = CreateFileW(
            PCWSTR(wide(path).as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        ) else {
            println!("  open failed");
            return;
        };

        let mut preparsed = PHIDP_PREPARSED_DATA::default();
        if !HidD_GetPreparsedData(handle, &mut preparsed).as_bool() {
            println!("  preparsed data failed");
            let _ = CloseHandle(handle);
            return;
        }

        let mut caps = windows::Win32::Devices::HumanInterfaceDevice::HIDP_CAPS::default();
        if HidP_GetCaps(preparsed, &mut caps) == HIDP_STATUS_SUCCESS {
            println!(
                "  caps: usagePage=0x{:04X} usage=0x{:04X} in={} out={} feature={}",
                caps.UsagePage,
                caps.Usage,
                caps.InputReportByteLength,
                caps.OutputReportByteLength,
                caps.FeatureReportByteLength
            );
        }

        for (label, report_type) in [("input", HidP_Input), ("output", HidP_Output)] {
            let mut count = 0u16;
            let _ = HidP_GetValueCaps(report_type, std::ptr::null_mut(), &mut count, preparsed);
            if count > 0 {
                let mut values = vec![HIDP_VALUE_CAPS::default(); count as usize];
                let status =
                    HidP_GetValueCaps(report_type, values.as_mut_ptr(), &mut count, preparsed);
                if status == HIDP_STATUS_SUCCESS {
                    println!("  {label} values ({count}):");
                    for value in values.iter().take(count as usize) {
                        let usage = if value.IsRange.as_bool() {
                            format!(
                                "0x{:02X}..0x{:02X}",
                                value.Anonymous.Range.UsageMin, value.Anonymous.Range.UsageMax
                            )
                        } else {
                            format!("0x{:02X}", value.Anonymous.NotRange.Usage)
                        };
                        println!(
                            "    id=0x{:02X} page=0x{:04X} usage={usage} bits={} count={} logical={}..{}",
                            value.ReportID,
                            value.UsagePage,
                            value.BitSize,
                            value.ReportCount,
                            value.LogicalMin,
                            value.LogicalMax
                        );
                    }
                }
            }

            let mut buttons = 0u16;
            let status =
                HidP_GetButtonCaps(report_type, std::ptr::null_mut(), &mut buttons, preparsed);
            if status == HIDP_STATUS_SUCCESS && buttons > 0 {
                let mut caps = vec![HIDP_BUTTON_CAPS::default(); buttons as usize];
                let status =
                    HidP_GetButtonCaps(report_type, caps.as_mut_ptr(), &mut buttons, preparsed);
                if status == HIDP_STATUS_SUCCESS {
                    println!("  {label} buttons ({buttons}):");
                    for cap in caps.iter().take(buttons as usize) {
                        let usage = if cap.IsRange.as_bool() {
                            format!(
                                "0x{:02X}..0x{:02X}",
                                cap.Anonymous.Range.UsageMin, cap.Anonymous.Range.UsageMax
                            )
                        } else {
                            format!("0x{:02X}", cap.Anonymous.NotRange.Usage)
                        };
                        println!(
                            "    id=0x{:02X} page=0x{:04X} usage={usage} count={}",
                            cap.ReportID, cap.UsagePage, cap.ReportCount
                        );
                    }
                }
            }
        }

        let _ = HidD_FreePreparsedData(preparsed);
        let _ = CloseHandle(handle);
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let vid = args.next().and_then(|v| parse_hex(&v));
    let pid = args.next().and_then(|v| parse_hex(&v));

    for interface in enumerate_all()
        .into_iter()
        .filter(|i| vid.is_none_or(|v| i.vendor_id == v))
        .filter(|i| pid.is_none_or(|p| i.product_id == p))
    {
        println!(
            "\n=== vid=0x{:04X} pid=0x{:04X}\n{}",
            interface.vendor_id, interface.product_id, interface.path
        );
        dump_caps(&interface.path);
    }
}
