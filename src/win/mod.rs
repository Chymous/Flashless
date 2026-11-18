//! Windows integration. Everything here is read-only towards the game except `typing`, which sends
//! ordinary keyboard input only when the user presses the Type In Chat hotkey.

pub mod capture;
pub mod clipboard;
pub mod hotkeys;
pub mod ocr;
pub mod shell;
pub mod tray;
pub mod typing;
pub mod watcher;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HWND, RECT};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, SetPriorityClass, BELOW_NORMAL_PRIORITY_CLASS};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const GAME_TITLE: PCWSTR = w!("League of Legends (TM) Client");

pub fn game_window() -> Option<HWND> {
    let h = unsafe { FindWindowW(None, GAME_TITLE) }.ok()?;
    (!h.is_invalid()).then_some(h)
}

pub fn game_is_foreground() -> bool {
    game_window().is_some_and(|g| unsafe { GetForegroundWindow() } == g)
}

/// Window rectangle in physical pixels.
pub fn window_rect(h: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(h, &mut r) }.ok()?;
    (r.right > r.left && r.bottom > r.top).then_some(r)
}

pub fn is_minimized(h: HWND) -> bool { unsafe { IsIconic(h) }.as_bool() }

/// Clicking the overlay must never take keyboard focus away from the game.
pub fn make_no_activate(h: HWND) {
    unsafe {
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        SetWindowLongPtrW(h, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize | WS_EX_TOOLWINDOW.0 as isize);
    }
}

pub fn find_window(title: &str) -> Option<HWND> {
    let t: Vec<u16> = title.encode_utf16().chain([0]).collect();
    let h = unsafe { FindWindowW(None, PCWSTR(t.as_ptr())) }.ok()?;
    (!h.is_invalid()).then_some(h)
}

/// Title bar colours for a standard window: dark-mode flag, caption, caption text, border (Windows 11).
pub fn set_title_bar(h: HWND, dark: bool, caption: [u8; 3], text: [u8; 3], border: [u8; 3]) {
    let rgb = |c: [u8; 3]| c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16;
    unsafe {
        let d: i32 = dark as i32;
        let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(20), &d as *const _ as _, 4);
        for (attr, c) in [(35, caption), (36, text), (34, border)] {
            let v = rgb(c);
            let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(attr), &v as *const _ as _, 4);
        }
    }
}

/// Everything here (OCR included) yields to the game when the CPU is busy.
pub fn lower_priority() {
    unsafe { let _ = SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS); }
}

/// False if another instance holds the named mutex. The handle is kept for the process lifetime.
pub fn single_instance(name: &str) -> bool {
    let n: Vec<u16> = name.encode_utf16().chain([0]).collect();
    unsafe {
        match CreateMutexW(None, true, PCWSTR(n.as_ptr())) {
            Ok(h) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    let _ = CloseHandle(h);
                    return false;
                }
                // the handle stays open for the life of the process; that is what holds the lock
                let _ = h;
                true
            }
            Err(_) => true,
        }
    }
}

pub fn message_box(text: &str, title: &str, yes_no: bool) -> bool {
    let t: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let c: Vec<u16> = title.encode_utf16().chain([0]).collect();
    let style = if yes_no { MB_YESNO | MB_ICONQUESTION } else { MB_OK | MB_ICONINFORMATION };
    unsafe { MessageBoxW(None, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), style) == IDYES }
}

/// Windows "Apps use light theme" preference.
pub fn system_prefers_dark() -> bool {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    key.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
        .map(|v| v == 0)
        .unwrap_or(true)
}

/// Keeps a saved overlay position on screen (a monitor may have been unplugged or the resolution changed).
/// Positions are in logical points; the virtual-screen metrics are in pixels, so this is approximate at
/// high DPI but always lands somewhere visible.
pub fn clamp_to_screen(left: f32, top: f32, width: f32) -> (f32, f32) {
    let (x, y, w, h) = unsafe {
        (GetSystemMetrics(SM_XVIRTUALSCREEN) as f32, GetSystemMetrics(SM_YVIRTUALSCREEN) as f32,
         GetSystemMetrics(SM_CXVIRTUALSCREEN) as f32, GetSystemMetrics(SM_CYVIRTUALSCREEN) as f32)
    };
    if w <= 0.0 || h <= 0.0 { return (left, top); }
    let visible_x = left + width * 0.5 >= x && left + 60.0 <= x + w;
    let visible_y = top >= y - 10.0 && top + 40.0 <= y + h;
    if visible_x && visible_y { (left, top) } else { (x + 40.0, y + 200.0) }
}

/// The Windows "notification" sound (respects the user's sound scheme and volume).
pub fn notify_sound() {
    use windows::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK;
    unsafe { let _ = MessageBeep(MB_ICONASTERISK); }
}

/// Opens an https link in the default browser.
pub fn open_url(url: &str) {
    if !url.starts_with("https://") { return; }
    use windows::core::{w, HSTRING};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url), None, None, SW_SHOWNORMAL); }
}
