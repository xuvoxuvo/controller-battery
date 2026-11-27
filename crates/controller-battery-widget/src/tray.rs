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
use std::time::Instant;

use controller_battery_core::model::ChargeState;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, COLORREF, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
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
    AppendMenuW, ChangeWindowMessageFilterEx, CreateIconIndirect, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyIcon, DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW,
    GetShellWindow, GetWindowLongPtrW, GetWindowThreadProcessId, IsWindowVisible, KillTimer,
    PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow,
    SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TrackPopupMenuEx, TranslateMessage, GWL_EXSTYLE, HICON, ICONINFO, LWA_ALPHA, MENU_ITEM_FLAGS,
    MF_ENABLED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, MSGFLT_ALLOW, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNORMAL, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP,
    WM_NULL, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_OVERLAPPED,
};

use crate::autohide::{Action, AutoHide};
use crate::collector::{self, Shared};
use crate::i18n;
use crate::icon_glyph;

/// Callback message the shell sends for mouse activity on the icon.
const WM_TRAY: u32 = WM_APP + 1;
/// Posted by a second instance to bring the card back.
pub const WM_SHOW_REQUEST: u32 = WM_APP + 2;
/// Posted by the UI thread to start a non-blocking hide fade on the tray thread.
const WM_FADE_HIDE: u32 = WM_APP + 3;
const TIMER_ID: usize = 0xC8;
const FADE_TIMER_ID: usize = 0xC9;
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
/// Last rendered tray state so idle refreshes do not churn icons, while a
/// Windows light/dark theme switch triggers a redraw.
#[derive(PartialEq, Eq)]
struct TraySnapshot {
    tip: String,
    percent: Option<u8>,
    charging: bool,
    light_taskbar: bool,
}

static LAST_STATE: Mutex<Option<TraySnapshot>> = Mutex::new(None);
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
/// Auto-hide lives here, not in the UI thread: while the card is hidden egui
/// may not paint at all, so only the tray timer is guaranteed to keep running.
static AUTO_HIDE: Mutex<Option<AutoHide>> = Mutex::new(None);
/// Layered-window fade, initialised on the first hide/show and stepped via a
/// 50 ms Win32 timer on the tray window so neither the UI thread nor the tray
/// message loop ever blocks during a fade.
const FADE_STEPS_OUT: u32 = 40; // 40 × 50 ms = the 2 s fade-out
const FADE_STEPS_IN: u32 = 6; // 300 ms fade-in, snappy when a pad wakes up
const FADE_FRAME_MS: u32 = 50;
static FADE_INIT: AtomicBool = AtomicBool::new(false);
static FADE_READY: AtomicBool = AtomicBool::new(false);
/// A fade is in progress; ignore tray clicks until it finishes.
static FADING: AtomicBool = AtomicBool::new(false);
/// Set by the UI thread when the user touches the card; aborts a fade-out.
static ABORT_FADE: AtomicBool = AtomicBool::new(false);
/// A hide fade is running, so the card shows "hiding…".
static HIDING: AtomicBool = AtomicBool::new(false);

struct FadeState {
    hiding: bool,
    step: u32,
    total: u32,
    hint_after: bool,
    focus_after: bool,
}

static ACTIVE_FADE: Mutex<Option<FadeState>> = Mutex::new(None);

/// When the auto-hide fade is due. The UI reads this from its own clock, so a
/// late timer tick can never skip the 3-2-1 countdown.
static DEADLINE: Mutex<Option<Instant>> = Mutex::new(None);

/// A poisoned mutex must never disable show/hide entirely, so recover the
/// state instead of panicking or bailing out.
fn auto_hide_lock() -> std::sync::MutexGuard<'static, Option<AutoHide>> {
    AUTO_HIDE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn deadline_lock() -> std::sync::MutexGuard<'static, Option<Instant>> {
    DEADLINE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Start the tray thread. Returns eframe's window handle for hide/show.
pub fn spawn(cc: &eframe::CreationContext<'_>, shared: Arc<Mutex<Shared>>) -> Option<isize> {
    let window = main_window(cc)?;
    MAIN_WINDOW.store(window, Ordering::Release);
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
    TRAY_READY.load(Ordering::Acquire)
}

/// Whether the card window is currently visible on screen.
pub fn is_card_visible() -> bool {
    main_window_handle().is_some_and(|w| unsafe { IsWindowVisible(w).as_bool() })
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
///
/// The fade itself runs asynchronously on the tray window's 50 ms timer so the
/// UI thread never freezes during the 2 s fade-out.
pub fn hide(window: isize) -> bool {
    if window == 0 || !available() || FADING.load(Ordering::Acquire) {
        return false;
    }
    let tray_window = TRAY_WINDOW.load(Ordering::Acquire);
    if tray_window == 0 {
        return false;
    }
    note_manual_hide();
    unsafe {
        PostMessageW(
            HWND(tray_window as *mut core::ffi::c_void),
            WM_FADE_HIDE,
            WPARAM(1),
            LPARAM(0),
        )
        .is_ok()
    }
}

/// Enable or disable auto-hide.
pub fn set_auto_hide(enabled: bool) {
    let mut state = auto_hide_lock();
    state
        .get_or_insert_with(|| AutoHide::new(enabled))
        .set_enabled(enabled);
    drop(state);
    *deadline_lock() = None;
}

/// The card was shown by the user; hold auto-hide for a few seconds.
fn note_manual_show() {
    let mut state = auto_hide_lock();
    state
        .get_or_insert_with(|| AutoHide::new(false))
        .note_manual_show(Instant::now());
    drop(state);
    *deadline_lock() = None;
}

/// The card was hidden by the user; never pop it back automatically.
fn note_manual_hide() {
    let mut state = auto_hide_lock();
    state
        .get_or_insert_with(|| AutoHide::new(false))
        .note_manual_hide();
    drop(state);
    *deadline_lock() = None;
}

/// The user clicked or dragged the card. Cancel the countdown immediately and
/// abort a fade-out that already started, instead of only checking focus at the
/// next one-second tick.
pub fn note_user_interaction() {
    ABORT_FADE.store(true, Ordering::Release);
    let mut state = auto_hide_lock();
    state
        .get_or_insert_with(|| AutoHide::new(false))
        .note_interaction(Instant::now());
    drop(state);
    *deadline_lock() = None;
}

/// At least one controller is reporting right now.
fn any_active() -> bool {
    let Some(shared) = SHARED.get() else {
        return false;
    };
    collector::snapshot(shared)
        .iter()
        .any(|(_, state)| !state.is_stale())
}

/// One auto-hide tick, driven by the tray timer.
fn auto_hide_tick(tray_hwnd: HWND) {
    let Some(window) = main_window_handle() else {
        return;
    };
    // A fade is running; do not fight it.
    if FADING.load(Ordering::Acquire) {
        return;
    }

    let visible = unsafe { IsWindowVisible(window).as_bool() };
    let active = any_active();
    let now = Instant::now();
    let (action, deadline) = {
        let mut guard = auto_hide_lock();
        let state = guard.get_or_insert_with(|| AutoHide::new(false));
        let action = state.tick(now, active, visible, available());
        (action, state.deadline())
    };
    *deadline_lock() = if action == Action::Hide {
        None
    } else {
        deadline
    };
    apply_auto_action(tray_hwnd, action);
}

/// Countdown for the card: `Some(n)` seconds, `Some(0)` while fading.
pub fn countdown() -> Option<u8> {
    if HIDING.load(Ordering::Acquire) {
        return Some(0);
    }
    let deadline = (*deadline_lock())?;
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        Some(0)
    } else {
        Some(left.as_millis().div_ceil(1000).min(9) as u8)
    }
}

fn apply_auto_action(tray_hwnd: HWND, action: Action) {
    match action {
        Action::None => {}
        // Without the one-shot balloon the card just vanishes and the tray icon
        // (often folded into the "^" overflow on Windows 11) is easy to miss.
        Action::Hide => start_fade_out(tray_hwnd, true),
        // Do not call `SetForegroundWindow` here: the card should come back on
        // its own without stealing focus from whatever the user is doing.
        Action::Show => start_fade_in(tray_hwnd, false),
    }
}

/// Make the card a layered window so the whole composition (squircle, text,
/// battery bar) can fade as one. Called once, lazily, from hide/show.
fn ensure_fade_ready(window: HWND) {
    if FADE_INIT.swap(true, Ordering::AcqRel) {
        return;
    }
    unsafe {
        let current = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
        if current & WS_EX_LAYERED.0 == 0 {
            SetWindowLongPtrW(window, GWL_EXSTYLE, (current | WS_EX_LAYERED.0) as isize);
            let _ = SetWindowPos(
                window,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
        let ready = SetLayeredWindowAttributes(window, COLORREF(0), 255, LWA_ALPHA).is_ok();
        FADE_READY.store(ready, Ordering::Release);
    }
}

fn set_alpha(window: HWND, alpha: u8) {
    unsafe {
        let _ = SetLayeredWindowAttributes(window, COLORREF(0), alpha, LWA_ALPHA);
    }
}

/// Begin a non-blocking fade-out stepped by `FADE_TIMER_ID` on the tray window.
fn start_fade_out(tray_hwnd: HWND, hint_after: bool) {
    let Some(window) = main_window_handle() else {
        return;
    };
    if FADING.swap(true, Ordering::AcqRel) {
        return;
    }
    HIDING.store(true, Ordering::Release);
    ABORT_FADE.store(false, Ordering::Release);
    ensure_fade_ready(window);

    if !FADE_READY.load(Ordering::Acquire) {
        unsafe {
            let _ = ShowWindow(window, SW_HIDE);
        }
        HIDING.store(false, Ordering::Release);
        FADING.store(false, Ordering::Release);
        if hint_after {
            hint_hidden();
        }
        return;
    }

    *ACTIVE_FADE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(FadeState {
        hiding: true,
        step: 0,
        total: FADE_STEPS_OUT,
        hint_after,
        focus_after: false,
    });
    unsafe {
        SetTimer(tray_hwnd, FADE_TIMER_ID, FADE_FRAME_MS, None);
    }
}

/// Begin a non-blocking fade-in stepped by `FADE_TIMER_ID` on the tray window.
fn start_fade_in(tray_hwnd: HWND, focus_after: bool) {
    let Some(window) = main_window_handle() else {
        return;
    };
    if FADING.swap(true, Ordering::AcqRel) {
        return;
    }
    HIDING.store(false, Ordering::Release);
    ABORT_FADE.store(false, Ordering::Release);
    ensure_fade_ready(window);

    if FADE_READY.load(Ordering::Acquire) {
        set_alpha(window, 0);
    }
    unsafe {
        let _ = ShowWindow(window, SW_SHOWNORMAL);
    }

    if !FADE_READY.load(Ordering::Acquire) {
        if focus_after {
            unsafe {
                let _ = SetForegroundWindow(window);
            }
        }
        FADING.store(false, Ordering::Release);
        return;
    }

    *ACTIVE_FADE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(FadeState {
        hiding: false,
        step: 0,
        total: FADE_STEPS_IN,
        hint_after: false,
        focus_after,
    });
    unsafe {
        SetTimer(tray_hwnd, FADE_TIMER_ID, FADE_FRAME_MS, None);
    }
}

/// Advance the active fade animation by one 50 ms frame.
fn step_fade(tray_hwnd: HWND) {
    let Some(window) = main_window_handle() else {
        unsafe {
            let _ = KillTimer(tray_hwnd, FADE_TIMER_ID);
        }
        *ACTIVE_FADE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        HIDING.store(false, Ordering::Release);
        FADING.store(false, Ordering::Release);
        return;
    };

    let mut guard = ACTIVE_FADE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(fade) = guard.as_mut() else {
        unsafe {
            let _ = KillTimer(tray_hwnd, FADE_TIMER_ID);
        }
        return;
    };

    if fade.hiding && ABORT_FADE.load(Ordering::Acquire) {
        *guard = None;
        drop(guard);
        unsafe {
            let _ = KillTimer(tray_hwnd, FADE_TIMER_ID);
        }
        set_alpha(window, 255);
        HIDING.store(false, Ordering::Release);
        FADING.store(false, Ordering::Release);
        return;
    }

    fade.step += 1;
    let hiding = fade.hiding;
    let done = fade.step >= fade.total;
    let hint_after = fade.hint_after;
    let focus_after = fade.focus_after;
    let alpha = if hiding {
        255 - (255 * fade.step / fade.total)
    } else {
        (255 * fade.step / fade.total).min(255)
    };
    if done {
        *guard = None;
    }
    drop(guard);

    set_alpha(window, alpha as u8);

    if done {
        unsafe {
            let _ = KillTimer(tray_hwnd, FADE_TIMER_ID);
        }
        if hiding {
            unsafe {
                let _ = ShowWindow(window, SW_HIDE);
            }
            set_alpha(window, 255);
            HIDING.store(false, Ordering::Release);
            FADING.store(false, Ordering::Release);
            if hint_after {
                hint_hidden();
            }
        } else {
            set_alpha(window, 255);
            FADING.store(false, Ordering::Release);
            if focus_after {
                unsafe {
                    let _ = SetForegroundWindow(window);
                }
            }
        }
    }
}

/// One-shot balloon that tells the user where the card went.
pub fn hint_hidden() {
    if !available() || HINT_SHOWN.swap(true, Ordering::AcqRel) {
        return;
    }
    let window = TRAY_WINDOW.load(Ordering::Acquire);
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
    write_wide(&mut data.szInfoTitle, i18n::current().hidden_title);
    write_wide(&mut data.szInfo, i18n::current().hidden_body);

    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

/// Bring the card back if it is hidden and the icon is gone.
fn restore_if_hidden(tray_hwnd: HWND) {
    let Some(window) = main_window_handle() else {
        return;
    };
    unsafe {
        if !IsWindowVisible(window).as_bool() {
            note_manual_show();
            start_fade_in(tray_hwnd, false);
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
/// Remove the icon synchronously: the tray thread's own `NIM_DELETE` can race
/// with process exit, which leaves a ghost icon in the notification area.
fn remove_icon() {
    let window = TRAY_WINDOW.load(Ordering::Acquire);
    if window == 0 {
        return;
    }
    let hwnd = HWND(window as *mut core::ffi::c_void);
    unsafe {
        let data = notify_data(hwnd, "", HICON::default());
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

pub fn shutdown() {
    let window = TRAY_WINDOW.load(Ordering::Acquire);
    if window == 0 {
        return;
    }
    remove_icon();
    unsafe {
        let _ = PostMessageW(
            HWND(window as *mut core::ffi::c_void),
            WM_CLOSE,
            WPARAM(0),
            LPARAM(0),
        );
    }
    // Give the thread a moment to drain its message loop.
    std::thread::sleep(std::time::Duration::from_millis(80));
}

type TrayResult = windows::core::Result<()>;

/// (Re-)register the icon with the shell, e.g. on startup and after Explorer
/// restarts. Records whether the shell really owns it.
unsafe fn add_icon(hwnd: HWND) {
    let (tip, percent, charging) = describe();
    let light_taskbar = uses_light_taskbar();
    let icon = battery_icon(percent, charging, light_taskbar);
    if icon.is_invalid() {
        eprintln!("tray icon: icon creation failed (percent={percent:?})");
    }

    let data = notify_data(hwnd, &tip, icon);
    // `Shell_NotifyIcon` does not always set the last error, so clear it first:
    // a value here is then really ours.
    windows::Win32::Foundation::SetLastError(windows::Win32::Foundation::WIN32_ERROR(0));
    let added = Shell_NotifyIconW(NIM_ADD, &data).as_bool();
    TRAY_READY.store(added, Ordering::Release);
    if !added {
        if !FAILURE_REPORTED.swap(true, Ordering::AcqRel) {
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

    store_state(tip, percent, charging, light_taskbar);
    let previous = ICON.swap(icon.0 as isize, Ordering::AcqRel);
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
    TRAY_WINDOW.store(hwnd.0 as isize, Ordering::Release);
    TASKBAR_CREATED.store(
        RegisterWindowMessageW(w!("TaskbarCreated")),
        Ordering::Release,
    );

    // Explorer lives at medium integrity. When this process runs elevated, UIPI
    // silently drops its tray callbacks and the `TaskbarCreated` broadcast, so
    // the icon would appear but no menu item (including Quit) would react.
    let _ = ChangeWindowMessageFilterEx(hwnd, WM_TRAY, MSGFLT_ALLOW, None);
    let _ = ChangeWindowMessageFilterEx(hwnd, WM_SHOW_REQUEST, MSGFLT_ALLOW, None);
    let taskbar_created = TASKBAR_CREATED.load(Ordering::Acquire);
    if taskbar_created != 0 {
        let _ = ChangeWindowMessageFilterEx(hwnd, taskbar_created, MSGFLT_ALLOW, None);
    }

    add_icon(hwnd);

    // Refresh the tooltip and icon once a second; the shell only needs a
    // `NIM_MODIFY` when the text or system theme actually changed.
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
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => toggle(hwnd),
                WM_RBUTTONUP => menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_SHOW_REQUEST => {
            // A second launch asked the running instance to come back.
            note_manual_show();
            start_fade_in(hwnd, true);
            LRESULT(0)
        }
        WM_FADE_HIDE => {
            start_fade_out(hwnd, wparam.0 != 0);
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            // System light/dark theme may have changed; redraw immediately.
            if TRAY_READY.load(Ordering::Acquire) {
                refresh(hwnd);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == FADE_TIMER_ID {
                step_fade(hwnd);
            } else if wparam.0 == TIMER_ID {
                let ticks = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
                // Registration can fail transiently (shell not ready yet), and a
                // low-integrity process can never register at all; retry on a slow
                // cadence so the menu item becomes usable as soon as it can.
                if !TRAY_READY.load(Ordering::Acquire) && ticks.is_multiple_of(RETRY_EVERY_TICKS) {
                    add_icon(hwnd);
                }
                refresh(hwnd);
                auto_hide_tick(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => {
            let taskbar_created = TASKBAR_CREATED.load(Ordering::Acquire);
            if taskbar_created != 0 && message == taskbar_created {
                // Explorer restarted and dropped every notification icon; put
                // ours back, and never leave a hidden card without one.
                add_icon(hwnd);
                if !TRAY_READY.load(Ordering::Acquire) {
                    restore_if_hidden(hwnd);
                }
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
    }
}

fn main_window_handle() -> Option<HWND> {
    let window = MAIN_WINDOW.load(Ordering::Acquire);
    (window != 0).then_some(HWND(window as *mut core::ffi::c_void))
}

fn toggle(tray_hwnd: HWND) {
    // A fade owns the window right now; clicking mid-animation used to flip
    // the intended state and leave the card invisible.
    if FADING.load(Ordering::Acquire) {
        return;
    }
    let Some(window) = main_window_handle() else {
        return;
    };
    unsafe {
        if IsWindowVisible(window).as_bool() {
            note_manual_hide();
            start_fade_out(tray_hwnd, false);
        } else {
            note_manual_show();
            start_fade_in(tray_hwnd, true);
        }
    }
}

unsafe fn menu(hwnd: HWND) {
    if FADING.load(Ordering::Acquire) {
        return;
    }
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };
    let visible = main_window_handle()
        .map(|window| IsWindowVisible(window).as_bool())
        .unwrap_or(false);

    let strings = i18n::current();
    let show_text = wide(strings.tray_show);
    let hide_text = wide(strings.tray_hide);
    let quit_text = wide(strings.quit);

    let show_flags = if visible { MF_GRAYED } else { MF_ENABLED };
    let hide_flags = if visible { MF_ENABLED } else { MF_GRAYED };
    let _ = AppendMenuW(
        menu,
        MENU_ITEM_FLAGS(MF_STRING.0 | show_flags.0),
        MENU_SHOW,
        PCWSTR(show_text.as_ptr()),
    );
    let _ = AppendMenuW(
        menu,
        MENU_ITEM_FLAGS(MF_STRING.0 | hide_flags.0),
        MENU_HIDE,
        PCWSTR(hide_text.as_ptr()),
    );
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, PCWSTR(quit_text.as_ptr()));

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
            note_manual_show();
            start_fade_in(hwnd, true);
        }
        MENU_HIDE => {
            note_manual_hide();
            start_fade_out(hwnd, false);
        }
        MENU_QUIT => {
            // A hidden eframe window may never confirm a WM_CLOSE (eframe only
            // closes after a repaint), so quit directly after removing the icon.
            remove_icon();
            std::process::exit(0);
        }
        _ => {}
    }
}

/// NUL-terminated UTF-16 copy of `text`, for the duration of a Win32 call.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `(tooltip, percent, charging)` for the first controller in the snapshot.
fn describe() -> (String, Option<u8>, bool) {
    let strings = i18n::current();
    let Some(shared) = SHARED.get() else {
        return (strings.app_name.to_owned(), None, false);
    };
    let snapshot = collector::snapshot(shared);
    let Some((_, first)) = snapshot.first() else {
        return (
            format!("{} · {}", strings.app_name, strings.tray_no_device),
            None,
            false,
        );
    };

    let reading = &first.reading;
    let name = strings.device_name(first);
    let state = strings.charge_state(reading.state);
    let mut text = match reading.percent {
        Some(percent) => format!("{name} {percent}% · {state}"),
        None => format!("{name} · {state}"),
    };
    if snapshot.len() > 1 {
        text.push_str(&strings.tray_count(snapshot.len()));
    }

    (
        text,
        reading.percent,
        reading.state == ChargeState::Charging,
    )
}

unsafe fn refresh(hwnd: HWND) {
    let (tip, percent, charging) = describe();
    let light_taskbar = uses_light_taskbar();
    let next = TraySnapshot {
        tip: tip.clone(),
        percent,
        charging,
        light_taskbar,
    };
    if LAST_STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        == Some(&next)
    {
        return;
    }
    store_state(tip.clone(), percent, charging, light_taskbar);

    let icon = battery_icon(percent, charging, light_taskbar);
    let updated = Shell_NotifyIconW(NIM_MODIFY, &notify_data(hwnd, &tip, icon)).as_bool();
    TRAY_READY.store(updated, Ordering::Release);
    let previous = ICON.swap(icon.0 as isize, Ordering::AcqRel);
    if previous != 0 {
        let _ = DestroyIcon(HICON(previous as *mut core::ffi::c_void));
    }

    // The shell forgot our icon (it does that when Explorer restarts). Try to
    // put it back; if that fails, never leave the card hidden without a way
    // back.
    if !updated {
        add_icon(hwnd);
        if !TRAY_READY.load(Ordering::Acquire) {
            restore_if_hidden(hwnd);
        }
    }
}

fn store_state(tip: String, percent: Option<u8>, charging: bool, light_taskbar: bool) {
    *LAST_STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(TraySnapshot {
        tip,
        percent,
        charging,
        light_taskbar,
    });
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
unsafe fn battery_icon(percent: Option<u8>, charging: bool, light_taskbar: bool) -> HICON {
    let size = icon_glyph::GRID;
    let pixels = icon_glyph::icon_pixels(size, percent, charging, outline_color(light_taskbar));

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
