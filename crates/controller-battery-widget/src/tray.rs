//! Windows notification-area icon.
//!
//! The card is a borderless tool window without a taskbar button, so once it is
//! hidden the tray icon is the only way back. `Shell_NotifyIcon` needs a window
//! to receive its callback messages and subclassing eframe's window is fragile,
//! so this module owns a small hidden window plus its message loop on a
//! dedicated thread.
//!
//! Hiding and showing use plain `ShowWindow` on the eframe window instead of
//! `ViewportCommand::Visible`: egui then never believes the window is gone, so
//! the repaint that follows `ShowWindow` still has a live window to draw into.
//!
//! The icon is redrawn only when the battery text changes; the tooltip follows
//! the same value so hovering the icon answers "how much is left?" without
//! looking at the card.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use controller_battery_core::model::ChargeState;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HDC, HGDIOBJ,
};
use windows::Win32::Security::{GetTokenInformation, TokenIntegrityLevel, TOKEN_QUERY};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW, GetShellWindow,
    GetWindowThreadProcessId, IsWindowVisible, PostMessageW, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SetForegroundWindow, SetTimer, ShowWindow, TrackPopupMenuEx,
    TranslateMessage, HICON, ICONINFO, MENU_ITEM_FLAGS, MF_ENABLED, MF_GRAYED, MF_SEPARATOR,
    MF_STRING, MSG, SW_HIDE, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE,
    WM_APP, WM_CLOSE, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_TIMER,
    WNDCLASSW, WS_OVERLAPPED,
};

use crate::collector::{self, Shared};
use crate::icon_glyph;

/// Callback message the shell sends for mouse activity on the icon.
const WM_TRAY: u32 = WM_APP + 1;
const TIMER_ID: usize = 0xC8;
const ICON_ID: u32 = 1;

const MENU_SHOW: usize = 1;
const MENU_HIDE: usize = 2;
const MENU_QUIT: usize = 3;

/// Device collection state; shared with the collector threads.
static SHARED: OnceLock<Arc<Mutex<Shared>>> = OnceLock::new();
/// eframe's window, as `isize` so it can live in a `static`.
static MAIN_WINDOW: AtomicIsize = AtomicIsize::new(0);
/// The hidden window that receives the shell callbacks.
static TRAY_WINDOW: AtomicIsize = AtomicIsize::new(0);
/// Current `HICON`, so the previous one can be destroyed.
static ICON: AtomicIsize = AtomicIsize::new(0);
/// Last rendered (tip, percent, charging) so idle refreshes do not churn icons.
static LAST_STATE: Mutex<Option<(String, Option<u8>, bool)>> = Mutex::new(None);
/// True while the shell actually owns our icon. Hiding is only allowed then:
/// a hidden card with no icon would be unreachable.
static TRAY_READY: AtomicBool = AtomicBool::new(false);
/// `TaskbarCreated`: Explorer restarted and dropped every notification icon.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
/// The "hidden to tray" balloon is shown once per run, not on every hide.
static HINT_SHOWN: AtomicBool = AtomicBool::new(false);
/// Timer ticks, used to space out registration retries.
static TICKS: AtomicU32 = AtomicU32::new(0);
/// The `NIM_ADD` failure is worth one line on stderr, not one per retry.
static FAILURE_REPORTED: AtomicBool = AtomicBool::new(false);
/// Retry `NIM_ADD` every N timer ticks (the timer fires once a second).
const RETRY_EVERY_TICKS: u32 = 5;

/// Start the tray thread. Returns eframe's window handle for hide/show.
pub fn spawn(cc: &eframe::CreationContext<'_>, shared: Arc<Mutex<Shared>>) -> Option<isize> {
    let window = main_window(cc)?;
    MAIN_WINDOW.store(window, Ordering::Relaxed);
    let _ = SHARED.set(shared);

    std::thread::Builder::new()
        .name("tray-icon".to_owned())
        .spawn(|| {
            if let Err(error) = unsafe { run() } {
                eprintln!("tray icon failed: {error}");
            }
        })
        .ok()?;

    Some(window)
}

/// Whether the shell actually owns our icon, i.e. whether hiding is safe.
pub fn available() -> bool {
    TRAY_READY.load(Ordering::Relaxed)
}

/// Integrity level of `process` (0x1000 low, 0x2000 medium, 0x3000 high).
fn integrity_level(process: HANDLE) -> Option<u32> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;

        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut needed);
        let mut buffer = vec![0u8; needed as usize];
        let read = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
        .is_ok();
        let _ = CloseHandle(token);

        if !read || buffer.len() < std::mem::size_of::<usize>() {
            return None;
        }
        // TOKEN_MANDATORY_LABEL { PSID Sid; DWORD Attributes; }
        let sid = buffer.as_ptr().cast::<*mut core::ffi::c_void>().read();
        if sid.is_null() {
            return None;
        }
        // SID: revision u8, sub-authority count u8, authority [u8; 6], then the
        // 32 bit sub-authorities; the first one is the integrity level.
        Some((sid.cast::<u8>().add(8) as *const u32).read())
    }
}

/// True when this process runs at low integrity (sandbox / AppContainer).
///
/// The shell lives at medium integrity and UIPI drops our messages, so
/// `Shell_NotifyIcon` keeps answering `ERROR_ACCESS_DENIED` no matter how often
/// it is retried — the user has to start the app from Explorer instead.
pub fn running_low_integrity() -> bool {
    let process = unsafe { GetCurrentProcess() };
    integrity_level(process).is_some_and(|level| level < 0x2000)
}

/// Integrity level of the process behind the desktop window (normally Explorer).
fn shell_integrity() -> Option<u32> {
    unsafe {
        let shell = GetShellWindow();
        if shell.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(shell, Some(&mut pid));

        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let level = integrity_level(process);
        let _ = CloseHandle(process);
        level
    }
}

/// Release builds have no console, so a failed registration leaves a small
/// report next to the settings file instead.
fn report_failure(details: &str) {
    let Some(dir) = crate::settings::data_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("tray.log"), details);
}

/// Hide the card (used by the in-app context menu).
///
/// Refuses when the shell does not own our icon: a hidden card without an icon
/// would be unreachable (this is what sandboxed/low-integrity processes hit —
/// `Shell_NotifyIconW` fails with `ERROR_ACCESS_DENIED`).
pub fn hide(window: isize) -> bool {
    if window == 0 || !available() {
        return false;
    }
    unsafe {
        let _ = ShowWindow(HWND(window as *mut core::ffi::c_void), SW_HIDE);
    }
    true
}

/// One-shot balloon that tells the user where the card went.
pub fn hint_hidden() {
    if !available() || HINT_SHOWN.swap(true, Ordering::Relaxed) {
        return;
    }
    let window = TRAY_WINDOW.load(Ordering::Relaxed);
    if window == 0 {
        return;
    }

    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: HWND(window as *mut core::ffi::c_void),
        uID: ICON_ID,
        uFlags: NIF_INFO,
        dwInfoFlags: NIIF_INFO,
        ..Default::default()
    };
    write_wide(&mut data.szInfoTitle, "已隐藏到托盘");
    write_wide(&mut data.szInfo, "左键单击托盘图标可恢复卡片");

    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

/// Bring the card back if it is hidden and the icon is gone.
fn restore_if_hidden() {
    let Some(window) = main_window_handle() else {
        return;
    };
    unsafe {
        if !IsWindowVisible(window).as_bool() {
            let _ = ShowWindow(window, SW_SHOWNORMAL);
        }
    }
}

/// eframe's own window handle, via `raw-window-handle`.
fn main_window(cc: &eframe::CreationContext<'_>) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

/// Remove the icon and stop the tray thread (called when the app exits).
pub fn shutdown() {
    let window = TRAY_WINDOW.load(Ordering::Relaxed);
    if window == 0 {
        return;
    }
    unsafe {
        let _ = PostMessageW(
            HWND(window as *mut core::ffi::c_void),
            WM_CLOSE,
            WPARAM(0),
            LPARAM(0),
        );
    }
    // Give the thread a moment to send `NIM_DELETE`, otherwise the shell keeps
    // a ghost icon until the mouse happens to pass over it.
    std::thread::sleep(std::time::Duration::from_millis(80));
}

type TrayResult = windows::core::Result<()>;

/// (Re-)register the icon with the shell, e.g. on startup and after Explorer
/// restarts. Records whether the shell really owns it.
unsafe fn add_icon(hwnd: HWND) {
    let (tip, percent, charging) = describe();
    let icon = battery_icon(percent, charging);
    if icon.is_invalid() {
        eprintln!("tray icon: icon creation failed (percent={percent:?})");
    }

    let data = notify_data(hwnd, &tip, icon);
    // `Shell_NotifyIcon` does not always set the last error, so clear it first:
    // a value here is then really ours.
    windows::Win32::Foundation::SetLastError(windows::Win32::Foundation::WIN32_ERROR(0));
    let added = Shell_NotifyIconW(NIM_ADD, &data).as_bool();
    TRAY_READY.store(added, Ordering::Relaxed);
    if !added {
        if !FAILURE_REPORTED.swap(true, Ordering::Relaxed) {
            let level = integrity_level(GetCurrentProcess());
            let shell = shell_integrity();
            let mut session = 0u32;
            let _ = ProcessIdToSessionId(GetCurrentProcessId(), &mut session);
            let details = format!(
                "controller-battery: tray icon registration failed\n\
                 error: {:?}\n\
                 cbSize: {}\n\
                 process integrity: {}\n\
                 shell integrity: {}\n\
                 session: {}\n\
                 hint: {}\n",
                windows::Win32::Foundation::GetLastError(),
                data.cbSize,
                level.map_or("unknown".to_owned(), |l| format!("0x{l:X}")),
                shell.map_or("unknown".to_owned(), |l| format!("0x{l:X}")),
                session,
                if running_low_integrity() {
                    "the process is sandboxed/low integrity; start it from Explorer"
                } else {
                    "the shell refused the icon; see the error code"
                }
            );
            eprintln!("{details}");
            report_failure(&details);
        }
        let _ = DestroyIcon(icon);
        return;
    }

    store_state(tip, percent, charging);
    let previous = ICON.swap(icon.0 as isize, Ordering::Relaxed);
    if previous != 0 {
        let _ = DestroyIcon(HICON(previous as *mut core::ffi::c_void));
    }
}

unsafe fn run() -> TrayResult {
    let hinstance = HINSTANCE(GetModuleHandleW(None)?.0);
    let class = w!("ControllerBatteryTrayWindow");

    let window_class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: hinstance,
        lpszClassName: class,
        ..Default::default()
    };
    RegisterClassW(&window_class);

    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        class,
        w!("Controller Battery"),
        WS_OVERLAPPED,
        0,
        0,
        0,
        0,
        None,
        None,
        hinstance,
        None,
    )?;
    TRAY_WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
    TASKBAR_CREATED.store(
        RegisterWindowMessageW(w!("TaskbarCreated")),
        Ordering::Relaxed,
    );

    add_icon(hwnd);

    // Refresh the tooltip and icon once a second; the shell only needs a
    // `NIM_MODIFY` when the text actually changed.
    SetTimer(hwnd, TIMER_ID, 1_000, None);

    let mut message = MSG::default();
    while GetMessageW(&mut message, None, 0, 0).0 > 0 {
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }

    let _ = Shell_NotifyIconW(NIM_DELETE, &notify_data(hwnd, "", HICON::default()));
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TRAY => {
            match (lparam.0 & 0xFFFF) as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => toggle(),
                WM_RBUTTONUP => menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_TIMER => {
            let ticks = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
            // Registration can fail transiently (shell not ready yet), and a
            // low-integrity process can never register at all; retry on a slow
            // cadence so the menu item becomes usable as soon as it can.
            if !TRAY_READY.load(Ordering::Relaxed) && ticks.is_multiple_of(RETRY_EVERY_TICKS) {
                add_icon(hwnd);
            }
            refresh(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => {
            let taskbar_created = TASKBAR_CREATED.load(Ordering::Relaxed);
            if taskbar_created != 0 && message == taskbar_created {
                // Explorer restarted and dropped every notification icon; put
                // ours back, and never leave a hidden card without one.
                add_icon(hwnd);
                if !TRAY_READY.load(Ordering::Relaxed) {
                    restore_if_hidden();
                }
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
    }
}

fn main_window_handle() -> Option<HWND> {
    let window = MAIN_WINDOW.load(Ordering::Relaxed);
    (window != 0).then_some(HWND(window as *mut core::ffi::c_void))
}

fn toggle() {
    let Some(window) = main_window_handle() else {
        return;
    };
    unsafe {
        if IsWindowVisible(window).as_bool() {
            let _ = ShowWindow(window, SW_HIDE);
        } else {
            let _ = ShowWindow(window, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(window);
        }
    }
}

unsafe fn menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };
    let visible = main_window_handle()
        .map(|window| IsWindowVisible(window).as_bool())
        .unwrap_or(false);

    let show_flags = if visible { MF_GRAYED } else { MF_ENABLED };
    let hide_flags = if visible { MF_ENABLED } else { MF_GRAYED };
    let _ = AppendMenuW(
        menu,
        MENU_ITEM_FLAGS(MF_STRING.0 | show_flags.0),
        MENU_SHOW,
        w!("显示卡片"),
    );
    let _ = AppendMenuW(
        menu,
        MENU_ITEM_FLAGS(MF_STRING.0 | hide_flags.0),
        MENU_HIDE,
        w!("隐藏卡片"),
    );
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("退出"));

    let mut point = POINT::default();
    let _ = GetCursorPos(&mut point);
    let _ = SetForegroundWindow(hwnd);
    let command = TrackPopupMenuEx(
        menu,
        TPM_RIGHTBUTTON.0 | TPM_RETURNCMD.0,
        point.x,
        point.y,
        hwnd,
        None,
    )
    .0 as usize;
    let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);

    match command {
        MENU_SHOW => {
            if let Some(window) = main_window_handle() {
                let _ = ShowWindow(window, SW_SHOWNORMAL);
                let _ = SetForegroundWindow(window);
            }
        }
        MENU_HIDE => {
            if let Some(window) = main_window_handle() {
                let _ = ShowWindow(window, SW_HIDE);
            }
        }
        MENU_QUIT => {
            if let Some(window) = main_window_handle() {
                let _ = PostMessageW(window, WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        _ => {}
    }
}

/// `(tooltip, percent, charging)` for the first controller in the snapshot.
fn describe() -> (String, Option<u8>, bool) {
    let Some(shared) = SHARED.get() else {
        return ("Controller Battery".to_owned(), None, false);
    };
    let snapshot = collector::snapshot(shared);
    let Some((_, first)) = snapshot.first() else {
        return ("Controller Battery · 未检测到手柄".to_owned(), None, false);
    };

    let reading = &first.reading;
    let mut text = match reading.percent {
        Some(percent) => format!(
            "{} {}% · {}",
            reading.device,
            percent,
            reading.state.label()
        ),
        None => format!("{} · {}", reading.device, reading.state.label()),
    };
    if snapshot.len() > 1 {
        text.push_str(&format!("（共 {} 个）", snapshot.len()));
    }

    (
        text,
        reading.percent,
        reading.state == ChargeState::Charging,
    )
}

unsafe fn refresh(hwnd: HWND) {
    let (tip, percent, charging) = describe();
    if LAST_STATE
        .lock()
        .map(|last| {
            last.as_ref()
                .is_some_and(|(t, p, c)| *t == tip && *p == percent && *c == charging)
        })
        .unwrap_or(false)
    {
        return;
    }
    store_state(tip.clone(), percent, charging);

    let icon = battery_icon(percent, charging);
    let updated = Shell_NotifyIconW(NIM_MODIFY, &notify_data(hwnd, &tip, icon)).as_bool();
    TRAY_READY.store(updated, Ordering::Relaxed);
    let previous = ICON.swap(icon.0 as isize, Ordering::Relaxed);
    if previous != 0 {
        let _ = DestroyIcon(HICON(previous as *mut core::ffi::c_void));
    }

    // The shell forgot our icon (it does that when Explorer restarts). Try to
    // put it back; if that fails, never leave the card hidden without a way
    // back.
    if !updated {
        add_icon(hwnd);
        if !TRAY_READY.load(Ordering::Relaxed) {
            restore_if_hidden();
        }
    }
}

fn store_state(tip: String, percent: Option<u8>, charging: bool) {
    if let Ok(mut last) = LAST_STATE.lock() {
        *last = Some((tip, percent, charging));
    }
}

unsafe fn notify_data(hwnd: HWND, tip: &str, icon: HICON) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };

    let slots = data.szTip.len();
    for (slot, unit) in data
        .szTip
        .iter_mut()
        .zip(tip.encode_utf16().take(slots.saturating_sub(1)))
    {
        *slot = unit;
    }
    data
}

/// Copy `text` into a fixed-size UTF-16 field, always leaving a terminator.
fn write_wide(slots: &mut [u16], text: &str) {
    let room = slots.len().saturating_sub(1);
    for (slot, unit) in slots.iter_mut().zip(text.encode_utf16().take(room)) {
        *slot = unit;
    }
}
// ---------------------------------------------------------------------------
// Icon drawing

/// Outline colour that stays visible on both light and dark taskbars.
fn outline_color(light_taskbar: bool) -> [u8; 4] {
    if light_taskbar {
        icon_glyph::OUTLINE_ON_LIGHT
    } else {
        icon_glyph::OUTLINE_ON_DARK
    }
}

/// Windows' own light/dark choice, so the glyph does not vanish into the
/// taskbar. Unknown values fall back to the dark-taskbar (light) outline.
fn uses_light_taskbar() -> bool {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    status.is_ok() && value == 1
}

/// A small battery glyph, filled to `percent`, coloured like the card's accent.
unsafe fn battery_icon(percent: Option<u8>, charging: bool) -> HICON {
    let size = icon_glyph::GRID;
    let pixels =
        icon_glyph::icon_pixels(size, percent, charging, outline_color(uses_light_taskbar()));

    // 1 bpp mask: bit set means "transparent" (alpha == 0).
    let mask_stride = size.div_ceil(16) * 2;
    let mut mask = vec![0u8; mask_stride * size];
    for y in 0..size {
        for x in 0..size {
            if pixels[(y * size + x) * 4 + 3] == 0 {
                mask[y * mask_stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }

    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            // Negative height: top-down rows, matching the pixel buffer.
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let Ok(color) = CreateDIBSection(HDC::default(), &info, DIB_RGB_COLORS, &mut bits, None, 0)
    else {
        return HICON::default();
    };
    if !bits.is_null() {
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
    }

    let mask_bitmap = CreateBitmap(size as i32, size as i32, 1, 1, Some(mask.as_ptr().cast()));

    let icon_info = ICONINFO {
        fIcon: windows::Win32::Foundation::BOOL(1),
        hbmMask: mask_bitmap,
        hbmColor: color,
        ..Default::default()
    };
    let icon = CreateIconIndirect(&icon_info).unwrap_or_default();

    // `CreateIconIndirect` copies the bitmaps, so ours can go now.
    let _ = DeleteObject(HGDIOBJ(color.0));
    let _ = DeleteObject(HGDIOBJ(mask_bitmap.0));

    icon
}
