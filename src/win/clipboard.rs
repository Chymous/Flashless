//! Writes plain Unicode text to the clipboard (one CF_UNICODETEXT entry, like Notepad). Never reads it.

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;

pub fn set_text(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        let mut open = false;
        for _ in 0..10 {
            if OpenClipboard(None).is_ok() { open = true; break; }
            std::thread::sleep(std::time::Duration::from_millis(15)); // another process holds it
        }
        if !open {
            return false;
        }
        let ok = (|| -> Option<()> {
            EmptyClipboard().ok()?;
            let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ok()?;
            let ptr = GlobalLock(mem) as *mut u16;
            if ptr.is_null() { let _ = GlobalFree(mem); return None; }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(mem);
            if SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(mem.0)).is_err() {
                let _ = GlobalFree(mem); // ownership only passes to the system on success
                return None;
            }
            Some(())
        })().is_some();
        let _ = CloseClipboard();
        ok
    }
}

