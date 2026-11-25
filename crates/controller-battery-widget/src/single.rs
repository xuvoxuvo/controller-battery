//! Single-instance guard.
//!
//! A second launch (double-clicking the exe again) must not create a second
//! card and a second tray icon. Instead it asks the running instance to show
//! its card and exits. The tray window's class name is unique to this app, so
//! it doubles as the "find the first instance" handle.

use std::time::Duration;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, LPARAM, WPARAM,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};

use crate::tray;

/// Keeps the named mutex alive for the lifetime of the process.
pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// `Some` when this is the only instance; `None` when another instance is
/// already running (it is asked to show its card).
pub fn acquire() -> Option<InstanceGuard> {
    unsafe {
        let handle =
            CreateMutexW(None, true, w!("Local\\ControllerBattery.SingleInstance")).ok()?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            wake_existing();
            return None;
        }
        Some(InstanceGuard(handle))
    }
}

/// Ask the running instance to bring its card back. Its tray window appears a
/// moment after startup, so retry briefly before giving up.
fn wake_existing() {
    for _ in 0..20 {
        unsafe {
            if let Ok(window) = FindWindowW(w!("ControllerBatteryTrayWindow"), PCWSTR::null()) {
                if !window.is_invalid() {
                    let _ = PostMessageW(window, tray::WM_SHOW_REQUEST, WPARAM(0), LPARAM(0));
                    return;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
