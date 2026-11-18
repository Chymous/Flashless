//! Type In Chat: opens League chat (Enter), types the message, selects and copies it in the chat box so
//! League's own clipboard holds it, and stops. It never presses the final Enter: the player sends it.
//! Ordinary SendInput key events, only while the game window is focused, only when the hotkey is pressed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::Duration;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

static BUSY: AtomicBool = AtomicBool::new(false);

const VK_RETURN_: u16 = 0x0D;
const VK_SHIFT_: u16 = 0x10;
const VK_CONTROL_: u16 = 0x11;

fn send(inputs: &[INPUT]) {
    unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32); }
}

fn key(vk: u16, up: bool) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: scan, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } },
    }
}

fn unicode(ch: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: ch, dwFlags: if up { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE }, time: 0, dwExtraInfo: 0 } },
    }
}

fn tap(vk: u16, delay: u64) {
    send(&[key(vk, false)]);
    sleep(Duration::from_millis(delay));
    send(&[key(vk, true)]);
    sleep(Duration::from_millis(delay));
}

fn chord(vk: u16, delay: u64) {
    send(&[key(VK_CONTROL_, false)]);
    sleep(Duration::from_millis(delay));
    tap(vk, delay + 10);
    send(&[key(VK_CONTROL_, true)]);
    sleep(Duration::from_millis(delay + 20));
}

fn type_char(c: u16, delay: u64) {
    let scan = unsafe { VkKeyScanW(c) };
    let mods = ((scan >> 8) & 0xFF) as u8;
    if scan == -1 || mods & !1 != 0 {
        // not a plain or shifted key on this layout
        send(&[unicode(c, false), unicode(c, true)]);
        sleep(Duration::from_millis(delay));
        return;
    }
    let shift = mods & 1 != 0;
    if shift { send(&[key(VK_SHIFT_, false)]); }
    tap((scan & 0xFF) as u16, delay);
    if shift { send(&[key(VK_SHIFT_, true)]); }
}

fn modifier_held() -> bool {
    [0x10, 0x11, 0x12, 0x5B, 0x5C].into_iter().any(|vk| (unsafe { GetAsyncKeyState(vk) }) < 0)
}

/// Blocking; call from a worker thread. Returns a short status for the overlay.
pub fn type_in_chat(text: &str, key_delay_ms: u64, copy_in_chat: bool) -> String {
    if text.is_empty() { return "nothing to type".into(); }
    if !super::game_is_foreground() { return "game not focused".into(); }
    if BUSY.swap(true, Ordering::SeqCst) { return "already typing".into(); }
    let result = (|| {
        // a hotkey's own Ctrl/Alt would turn every letter into a shortcut: wait for release
        for _ in 0..100 {
            if !modifier_held() { break; }
            sleep(Duration::from_millis(15));
        }
        if modifier_held() { return "release the hotkey".to_string(); }
        // measured in game: 12 ms presses get dropped by the chat box, 20 ms do not
        let delay = key_delay_ms.clamp(20, 100);
        tap(VK_RETURN_, 35);
        sleep(Duration::from_millis(200));
        for c in text.encode_utf16() {
            if !super::game_is_foreground() { return "stopped: game lost focus".into(); }
            type_char(c, delay);
        }
        sleep(Duration::from_millis(60));
        if copy_in_chat {
            chord(0x41, delay); // Ctrl+A
            chord(0x43, delay); // Ctrl+C
        }
        "typed in chat - press Enter to send".into()
    })();
    BUSY.store(false, Ordering::SeqCst);
    result
}

