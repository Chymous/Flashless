//! Chat reading loop: about once a second while a match is live and League is in front, capture the chat
//! area, skip if unchanged, prepare the image, OCR it, and hand the rows to the UI.

use super::capture::{self, WindowCapture};
use super::ocr::{self, Ocr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct WatchConfig {
    pub region: [f64; 4],
    pub scale: usize,
    pub interval_ms: u64,
}

pub enum WatchEvent {
    Lines(Vec<String>),
    Stats { capture_ms: u64, ocr_ms: u64, scans: u32, ocr_runs: u32 },
    Problem(String),
}

pub struct Watcher {
    pub allowed: Arc<AtomicBool>,
    pub config: Arc<Mutex<WatchConfig>>,
}

impl Watcher {
    pub fn start(config: WatchConfig, tx: Sender<WatchEvent>, wake: Arc<dyn Fn() + Send + Sync>) -> Watcher {
        let allowed = Arc::new(AtomicBool::new(false));
        let config = Arc::new(Mutex::new(config));
        let (a, c) = (allowed.clone(), config.clone());
        std::thread::Builder::new().name("chat-watcher".into()).spawn(move || run(a, c, tx, wake)).expect("spawn watcher");
        Watcher { allowed, config }
    }
}

fn run(allowed: Arc<AtomicBool>, config: Arc<Mutex<WatchConfig>>, tx: Sender<WatchEvent>, wake: Arc<dyn Fn() + Send + Sync>) {
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_LOWEST};
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
        let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
    }
    let Some(engine) = Ocr::new() else {
        let _ = tx.send(WatchEvent::Problem("chat reading needs the Windows English OCR language".into()));
        wake();
        return;
    };
    let mut cap: Option<WindowCapture> = None;
    let mut last_print = 0u64;
    let mut failed = false;
    let (mut scans, mut ocr_runs, mut cap_total, mut ocr_total) = (0u32, 0u32, 0u64, 0u64);
    let mut next_stats = Instant::now() + Duration::from_secs(30);

    loop {
        let cfg = config.lock().unwrap().clone();
        std::thread::sleep(Duration::from_millis(cfg.interval_ms.clamp(250, 5000)));
        if !allowed.load(Ordering::Relaxed) || failed {
            cap = None; // no capture session between games
            continue;
        }
        let Some(hwnd) = super::game_window() else { cap = None; continue };
        if !super::game_is_foreground() || super::is_minimized(hwnd) { continue; }

        let t = Instant::now();
        if cap.as_ref().map(|c| c.hwnd) != Some(hwnd) {
            cap = None;
            if !capture::is_supported() {
                failed = true;
                let _ = tx.send(WatchEvent::Problem("window capture is not supported on this Windows version".into()));
                wake();
                continue;
            }
            match WindowCapture::new(hwnd) {
                Ok(c) => cap = Some(c),
                Err(e) => { let _ = tx.send(WatchEvent::Problem(format!("window capture failed: {e}"))); wake(); continue; }
            }
        }
        let frame = match cap.as_mut().unwrap().capture(cfg.region) {
            Ok(Some(f)) => f,
            Ok(None) => continue,
            Err(_) => { cap = None; continue; }
        };
        let cap_ms = t.elapsed().as_millis() as u64;
        scans += 1;
        cap_total += cap_ms;

        let print = ocr::text_fingerprint(&frame);
        if print != last_print {
            last_print = print;
            let t2 = Instant::now();
            let prepared = ocr::upscale(&ocr::whiten_red(&frame), cfg.scale.clamp(1, 4));
            if let Ok(rows) = engine.read_rows(&prepared) {
                ocr_runs += 1;
                ocr_total += t2.elapsed().as_millis() as u64;
                if !rows.is_empty() {
                    let _ = tx.send(WatchEvent::Lines(rows.into_iter().map(|r| r.text).collect()));
                    wake();
                }
            }
        }
        if Instant::now() >= next_stats {
            next_stats = Instant::now() + Duration::from_secs(30);
            let _ = tx.send(WatchEvent::Stats { capture_ms: cap_total / scans.max(1) as u64, ocr_ms: ocr_total / ocr_runs.max(1) as u64, scans, ocr_runs });
            (scans, ocr_runs, cap_total, ocr_total) = (0, 0, 0, 0);
        }
    }
}

/// One-off full-window capture + chat detection for Setup > Auto-detect. Blocking.
pub fn detect_chat_area() -> Result<([f64; 4], usize), String> {
    unsafe { let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED); }
    let hwnd = super::game_window().ok_or("League is not running.")?;
    let engine = Ocr::new().ok_or("Windows has no English OCR language installed.")?;
    let mut cap = WindowCapture::new(hwnd).map_err(|e| e.to_string())?;
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(Some(f)) = cap.capture([0.0, 0.0, 1.0, 1.0]) {
            return ocr::detect_chat_area(&engine, &f);
        }
    }
    Err("Could not capture the game window.".into())
}
