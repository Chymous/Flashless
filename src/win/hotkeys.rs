//! System-wide hotkeys through low-level keyboard and mouse hooks. League ignores RegisterHotKey while
//! focused, so hooks are what make hotkeys work in game. Keys and buttons are only observed, never
//! swallowed or changed. Mouse buttons 3, 4 and 5 can be bound (alone or with Ctrl/Alt/Shift).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const MOD_ALT: u8 = 1;
pub const MOD_CTRL: u8 = 2;
pub const MOD_SHIFT: u8 = 4;
pub const MOD_WIN: u8 = 8;

struct Hook {
    bindings: Mutex<HashMap<(u8, u32), String>>,
    down: Mutex<HashSet<u32>>,
    tx: Mutex<Option<Sender<String>>>,
    wake: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Paused while Setup captures a new combination.
    paused: AtomicBool,
}

fn hook() -> &'static Hook {
    static H: OnceLock<Hook> = OnceLock::new();
    H.get_or_init(|| Hook { bindings: Mutex::new(HashMap::new()), down: Mutex::new(HashSet::new()), tx: Mutex::new(None), wake: Mutex::new(None), paused: AtomicBool::new(false) })
}

pub fn set_paused(p: bool) { hook().paused.store(p, Ordering::Relaxed); }

/// Installs the hook on its own thread (it needs a message loop) and sends action names on `tx`.
pub fn start(tx: Sender<String>, wake: Arc<dyn Fn() + Send + Sync>) {
    *hook().tx.lock().unwrap() = Some(tx);
    *hook().wake.lock().unwrap() = Some(wake);
    std::thread::Builder::new().name("keyboard-hook".into()).spawn(|| unsafe {
        let module = GetModuleHandleW(None).unwrap_or_default();
        let Ok(h) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc_), module, 0) else { return };
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0).ok();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(h);
        if let Some(m) = mouse { let _ = UnhookWindowsHookEx(m); }
    }).expect("spawn keyboard hook");
}

/// Replaces all bindings. Returns the combos that could not be parsed or are duplicates.
pub fn set_bindings(bindings: &std::collections::BTreeMap<String, String>) -> Vec<String> {
    let mut map = HashMap::new();
    let mut failed = Vec::new();
    for (action, combo) in bindings {
        if combo.trim().is_empty() { continue; }
        match parse_combo(combo) {
            Some(k) if !map.contains_key(&k) => { map.insert(k, action.clone()); }
            _ => failed.push(combo.clone()),
        }
    }
    *hook().bindings.lock().unwrap() = map;
    failed
}

fn held(vk: i32) -> bool { (unsafe { GetAsyncKeyState(vk) }) < 0 }

fn current_mods() -> u8 {
    (held(0x12) as u8 * MOD_ALT) | (held(0x11) as u8 * MOD_CTRL) | (held(0x10) as u8 * MOD_SHIFT) | ((held(0x5B) || held(0x5C)) as u8 * MOD_WIN)
}

/// Sends the action bound to (modifiers, key) if there is one.
fn fire(vk: u32) {
    let h = hook();
    if h.paused.load(Ordering::Relaxed) { return; }
    let action = h.bindings.lock().ok().and_then(|b| b.get(&(current_mods(), vk)).cloned());
    if let Some(a) = action {
        if let Some(tx) = h.tx.lock().ok().and_then(|t| t.clone()) { let _ = tx.send(a); }
        if let Some(w) = h.wake.lock().ok().and_then(|w| w.clone()) { w(); }
    }
}

pub const VK_MOUSE3: u32 = 0x04; // middle button
pub const VK_MOUSE4: u32 = 0x05; // XBUTTON1 ("back")
pub const VK_MOUSE5: u32 = 0x06; // XBUTTON2 ("forward")

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let ms = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        let injected = ms.flags & LLMHF_INJECTED != 0;
        let vk = match wparam.0 as u32 {
            WM_MBUTTONDOWN => Some(VK_MOUSE3),
            WM_XBUTTONDOWN => match (ms.mouseData >> 16) & 0xFFFF { 1 => Some(VK_MOUSE4), 2 => Some(VK_MOUSE5), _ => None },
            _ => None,
        };
        if let (Some(vk), false) = (vk, injected) { fire(vk); }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn proc_(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let vk = kb.vkCode;
        let msg = wparam.0 as u32;
        let injected = kb.flags.0 & LLKHF_INJECTED.0 != 0; // our own typing
        let h = hook();
        if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
            let first = h.down.lock().map(|mut d| d.insert(vk)).unwrap_or(false); // ignore auto-repeat
            let is_mod = matches!(vk, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5);
            if first && !injected && !is_mod { fire(vk); }
        } else if msg == WM_KEYUP || msg == WM_SYSKEYUP {
            if let Ok(mut d) = h.down.lock() { d.remove(&vk); }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

// ---------- combo names <-> virtual keys ----------

pub fn parse_combo(combo: &str) -> Option<(u8, u32)> {
    let mut mods = 0u8;
    let mut vk = None;
    for part in combo.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= MOD_CTRL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" => mods |= MOD_WIN,
            _ => vk = Some(key_to_vk(part)?),
        }
    }
    vk.map(|v| (mods, v))
}

pub fn key_to_vk(name: &str) -> Option<u32> {
    let n = name.trim();
    let up = n.to_ascii_uppercase();
    let up = up.strip_prefix('D').filter(|r| r.len() == 1 && r.chars().all(|c| c.is_ascii_digit())).map(str::to_string).unwrap_or(up);
    if up.len() == 1 {
        let c = up.as_bytes()[0];
        if c.is_ascii_uppercase() || c.is_ascii_digit() { return Some(c as u32); }
    }
    if let Some(f) = up.strip_prefix('F').and_then(|r| r.parse::<u32>().ok()) {
        if (1..=24).contains(&f) { return Some(0x6F + f); }
    }
    if let Some(d) = up.strip_prefix("NUMPAD").and_then(|r| r.parse::<u32>().ok()) {
        if d <= 9 { return Some(0x60 + d); }
    }
    Some(match up.as_str() {
        "SPACE" => 0x20, "ENTER" | "RETURN" => 0x0D, "TAB" => 0x09, "ESCAPE" | "ESC" => 0x1B,
        "INSERT" => 0x2D, "DELETE" => 0x2E, "HOME" => 0x24, "END" => 0x23, "PAGEUP" => 0x21, "PAGEDOWN" => 0x22,
        "LEFT" => 0x25, "UP" => 0x26, "RIGHT" => 0x27, "DOWN" => 0x28,
        "MOUSE3" | "MIDDLEMOUSE" | "MOUSEMIDDLE" => VK_MOUSE3,
        "MOUSE4" | "XBUTTON1" => VK_MOUSE4,
        "MOUSE5" | "XBUTTON2" => VK_MOUSE5,
        "MINUS" => 0xBD, "PLUS" | "EQUALS" => 0xBB, "COMMA" => 0xBC, "PERIOD" => 0xBE, "BACKTICK" => 0xC0,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_parse() {
        assert_eq!(parse_combo("Ctrl+Alt+C"), Some((MOD_CTRL | MOD_ALT, 'C' as u32)));
        assert_eq!(parse_combo("Ctrl+Alt+D1"), Some((MOD_CTRL | MOD_ALT, '1' as u32)));
        assert_eq!(parse_combo("Ctrl+Alt+1"), Some((MOD_CTRL | MOD_ALT, '1' as u32)));
        assert_eq!(parse_combo("Shift+F5"), Some((MOD_SHIFT, 0x74)));
        assert_eq!(parse_combo("Ctrl+Alt"), None);
        assert_eq!(parse_combo("Ctrl+Bogus"), None);
        assert_eq!(parse_combo("Mouse4"), Some((0, VK_MOUSE4)));
        assert_eq!(parse_combo("Ctrl+Mouse5"), Some((MOD_CTRL, VK_MOUSE5)));
        assert_eq!(parse_combo("Shift+Mouse3"), Some((MOD_SHIFT, VK_MOUSE3)));
    }
}

