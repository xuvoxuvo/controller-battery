//! "Start with Windows" integration through the per-user Run key.
//!
//! Only `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` is touched (no
//! elevation, no Task Scheduler). The registered command passes `--tray` so a
//! login start stays in the notification area instead of popping the card.

use std::path::{Path, PathBuf};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "ControllerBattery";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn command_line(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// Whether the per-user Run value currently exists.
pub fn is_enabled() -> bool {
    let run_key = wide(RUN_KEY);
    let value_name = wide(VALUE_NAME);
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(run_key.as_ptr()),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return false;
        }
        let mut kind = REG_SZ;
        let mut size = 0u32;
        let status = RegQueryValueExW(
            key,
            PCWSTR(value_name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        );
        let _ = RegCloseKey(key);
        status == ERROR_SUCCESS
    }
}

/// Enable or disable the login entry. Returns whether the registry now matches.
pub fn set_enabled(enabled: bool) -> bool {
    if enabled {
        enable()
    } else {
        disable()
    }
}

fn enable() -> bool {
    let Some(exe) = current_exe() else {
        eprintln!("autostart: cannot determine current executable path");
        return false;
    };
    let data: Vec<u8> = command_line(&exe)
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let run_key = wide(RUN_KEY);
    let value_name = wide(VALUE_NAME);

    unsafe {
        let mut key = HKEY::default();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(run_key.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        );
        if status != ERROR_SUCCESS {
            eprintln!("autostart: cannot open Run key: {status:?}");
            return false;
        }
        let status = RegSetValueExW(key, PCWSTR(value_name.as_ptr()), 0, REG_SZ, Some(&data));
        let _ = RegCloseKey(key);
        if status != ERROR_SUCCESS {
            eprintln!("autostart: cannot write Run value: {status:?}");
            return false;
        }
        true
    }
}

fn disable() -> bool {
    let run_key = wide(RUN_KEY);
    let value_name = wide(VALUE_NAME);

    unsafe {
        let mut key = HKEY::default();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(run_key.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut key,
        );
        if status == ERROR_FILE_NOT_FOUND {
            return true;
        }
        if status != ERROR_SUCCESS {
            eprintln!("autostart: cannot open Run key: {status:?}");
            return false;
        }
        let status = RegDeleteValueW(key, PCWSTR(value_name.as_ptr()));
        let _ = RegCloseKey(key);
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            return true;
        }
        eprintln!("autostart: cannot delete Run value: {status:?}");
        false
    }
}

fn current_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn command_line_quotes_the_exe() {
        let line = command_line(Path::new(
            r"C:\App\Controller Battery\controller-battery.exe",
        ));
        assert_eq!(
            line,
            r#""C:\App\Controller Battery\controller-battery.exe""#
        );
    }
}
