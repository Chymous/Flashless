//! Notification-area (system tray) icon. Left click shows/hides the overlay; right click opens a menu.
//! Runs on its own thread with a hidden window that receives the tray messages; actions are sent to the
//! UI through the same channel as hotkeys.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 1;
/// Posted by a second Flashless launch: bring this instance's overlay back.
pub const WM_SHOW_EXISTING: u32 = WM_APP + 2;
const ITEMS: [(usize, &str); 5] = [(1, "ToggleOverlay"), (2, "OpenSettings"), (3, "CopySummary"), (4, "Exit"), (5, "ToggleCompact")];

static HWND_: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

struct Sink {
    tx: Sender<String>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

fn sink() -> &'static Mutex<Option<Sink>> {
    static S: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

fn send(action: &str) {
    if let Some(s) = sink().lock().unwrap().as_ref() {
        let _ = s.tx.send(action.to_string());
        (s.wake)();
    }
}

pub fn start(tx: Sender<String>, wake: Arc<dyn Fn() + Send + Sync>) {
    *sink().lock().unwrap() = Some(Sink { tx, wake });
    std::thread::Builder::new().name("tray".into()).spawn(|| unsafe {
        let log = |m: String| {
            let _ = std::fs::create_dir_all(crate::settings::data_dir());
            let _ = std::fs::write(crate::settings::data_dir().join("tray.log"), m);
        };
        let Ok(module) = GetModuleHandleW(None) else { log("GetModuleHandleW failed".into()); return };
        let hinst = HINSTANCE(module.0);
        let class = w!("FlashlessTrayWindow");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst, lpszClassName: class, ..Default::default() };
        if RegisterClassW(&wc) == 0 { log(format!("RegisterClassW failed: {:?}", windows::core::Error::from_win32())); }
        let _ = TASKBAR_CREATED.set(RegisterWindowMessageW(w!("TaskbarCreated")));
        let hwnd = match CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("Flashless tray"), WINDOW_STYLE(0), 0, 0, 0, 0, None, None, hinst, None) {
            Ok(h) => h,
            Err(e) => { log(format!("CreateWindowExW failed: {e}")); return; }
        };
        HWND_.store(hwnd.0 as isize, Ordering::SeqCst);
        add_icon(hwnd);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }).expect("spawn tray thread");
}

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() }
}

unsafe fn add_icon(hwnd: HWND) {
    let module = GetModuleHandleW(None).unwrap_or_default();
    // icon resource 1 from assets/app.rc, at the small-icon size for this display
    let icon = LoadImageW(HINSTANCE(module.0), PCWSTR(1 as *const u16), IMAGE_ICON, GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON), LR_DEFAULTCOLOR)
        .map(|h| HICON(h.0)).unwrap_or_default();
    let mut nid = data(hwnd);
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = icon;
    for (i, c) in "Flashless".encode_utf16().enumerate() { nid.szTip[i] = c; }
    if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
        let _ = std::fs::write(crate::settings::data_dir().join("tray.log"), format!("Shell_NotifyIconW failed, icon null: {}", icon.is_invalid()));
    }
}

/// Removes the icon (call on exit, or it lingers until the mouse passes over it).
pub fn remove() {
    let h = HWND_.load(Ordering::SeqCst);
    if h != 0 {
        unsafe { let _ = Shell_NotifyIconW(NIM_DELETE, &data(HWND(h as _))); }
    }
}

unsafe fn show_menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let _ = AppendMenuW(menu, MF_STRING, 1, w!("Show / hide overlay"));
    let _ = AppendMenuW(menu, MF_STRING, 5, w!("Compact mode on / off"));
    let _ = AppendMenuW(menu, MF_STRING, 2, w!("Settings"));
    let _ = AppendMenuW(menu, MF_STRING, 3, w!("Copy cooldown message"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, 4, w!("Exit Flashless"));
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd); // so the menu closes when clicking elsewhere
    let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, None);
    let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_SHOW_EXISTING {
        send("ShowOverlay");
        return LRESULT(0);
    }
    if msg == WM_TRAY {
        match (lparam.0 as u32) & 0xFFFF {
            WM_LBUTTONUP => send("ToggleOverlay"),
            WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
            _ => {}
        }
        return LRESULT(0);
    }
    if msg == WM_COMMAND {
        let id = wparam.0 & 0xFFFF;
        if let Some((_, action)) = ITEMS.iter().find(|(i, _)| *i == id) { send(action); }
        return LRESULT(0);
    }
    if TASKBAR_CREATED.get() == Some(&msg) {
        add_icon(hwnd); // Explorer restarted: put the icon back
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Called by a second launch: asks the running instance to show its overlay. True if one was found.
pub fn show_existing() -> bool {
    unsafe {
        match FindWindowW(w!("FlashlessTrayWindow"), PCWSTR::null()) {
            Ok(h) if !h.is_invalid() => PostMessageW(h, WM_SHOW_EXISTING, WPARAM(0), LPARAM(0)).is_ok(),
            _ => false,
        }
    }
}
