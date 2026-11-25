//! Native context menu for the card.
//!
//! egui draws its popups inside the card's own 230×68 window, so the menu gets
//! clipped at the window edge — and the language submenu needs more room than
//! the card has. A real Win32 menu is drawn by the shell, flips itself at
//! screen edges and follows the system light/dark theme, so it is used here
//! (the tray icon already works the same way).

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, PostMessageW, SetForegroundWindow,
    TrackPopupMenuEx, MENU_ITEM_FLAGS, MF_CHECKED, MF_ENABLED, MF_GRAYED, MF_POPUP, MF_SEPARATOR,
    MF_STRING, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_NULL,
};

use crate::i18n::{self, Language};
use crate::settings::Settings;

const MENU_AUTO_HIDE: usize = 1;
const MENU_HIDE: usize = 2;
const MENU_QUIT: usize = 3;
/// Language entries occupy `MENU_LANGUAGE + index into Language::ALL`.
const MENU_LANGUAGE: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    ToggleAutoHide,
    SetLanguage(Language),
    HideToTray,
    Quit,
}

/// Show the menu at the cursor and return the chosen item.
///
/// `can_hide` is false while the shell does not own the tray icon: without it a
/// hidden card could not be brought back.
pub fn show(window: isize, settings: &Settings, can_hide: bool) -> Option<Command> {
    let strings = i18n::current();
    let card = HWND(window as *mut core::ffi::c_void);
    if card.is_invalid() {
        return None;
    }

    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let language_menu = CreatePopupMenu().ok()?;

        for (index, language) in Language::ALL.iter().enumerate() {
            let text = wide(strings.language_name(*language));
            let flags = MF_STRING
                | if settings.language == *language {
                    MF_CHECKED
                } else {
                    MENU_ITEM_FLAGS(0)
                };
            let _ = AppendMenuW(
                language_menu,
                flags,
                MENU_LANGUAGE + index,
                PCWSTR(text.as_ptr()),
            );
        }

        let auto_hide = wide(strings.auto_hide);
        let _ = AppendMenuW(
            menu,
            MF_STRING
                | if settings.auto_hide {
                    MF_CHECKED
                } else {
                    MENU_ITEM_FLAGS(0)
                },
            MENU_AUTO_HIDE,
            PCWSTR(auto_hide.as_ptr()),
        );

        let language = wide(strings.language);
        let _ = AppendMenuW(
            menu,
            MF_STRING | MF_POPUP,
            language_menu.0 as usize,
            PCWSTR(language.as_ptr()),
        );

        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

        let hide = wide(strings.hide_to_tray);
        let _ = AppendMenuW(
            menu,
            MF_STRING | if can_hide { MF_ENABLED } else { MF_GRAYED },
            MENU_HIDE,
            PCWSTR(hide.as_ptr()),
        );
        let quit = wide(strings.quit);
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, PCWSTR(quit.as_ptr()));

        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        // The menu is modal and needs our window in front, or it will not
        // dismiss when the user clicks elsewhere.
        let _ = SetForegroundWindow(card);
        let picked = TrackPopupMenuEx(
            menu,
            TPM_RIGHTBUTTON.0 | TPM_RETURNCMD.0,
            point.x,
            point.y,
            card,
            None,
        )
        .0 as usize;
        let _ = PostMessageW(card, WM_NULL, WPARAM(0), LPARAM(0));
        // Destroys the attached language submenu as well.
        let _ = DestroyMenu(menu);

        match picked {
            MENU_AUTO_HIDE => Some(Command::ToggleAutoHide),
            MENU_HIDE => Some(Command::HideToTray),
            MENU_QUIT => Some(Command::Quit),
            id if id >= MENU_LANGUAGE => Language::ALL
                .get(id - MENU_LANGUAGE)
                .copied()
                .map(Command::SetLanguage),
            _ => None,
        }
    }
}

/// NUL-terminated UTF-16 copy of `text`, for the duration of a Win32 call.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
