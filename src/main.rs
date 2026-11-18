#![cfg_attr(not(test), windows_subsystem = "windows")]
//! Flashless: League of Legends companion app that tracks enemy summoner spell cooldowns.

mod chat;
mod cooldown;
mod data;
mod live;
mod message;
mod settings;
mod tracker;
mod update;
mod ui;
mod win;

use eframe::egui;

/// Required by Riot's fan-content policy ("Legal Jibber Jabber") for free community projects.
pub const RIOT_NOTICE: &str = "Flashless isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or anyone officially involved in producing or managing Riot Games properties. Riot Games and all associated properties are trademarks or registered trademarks of Riot Games, Inc.";
use std::sync::Arc;

fn main() {
    // crashes go to error.log instead of vanishing
    std::panic::set_hook(Box::new(|info| {
        let dir = settings::data_dir();
        let _ = std::fs::create_dir_all(&dir);
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("error.log")) {
            let _ = writeln!(f, "{:?}  {info}", std::time::SystemTime::now());
        }
    }));

    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a.eq_ignore_ascii_case(flag));
    let silent = has("--silent");

    // diagnostic: Flashless.exe --ocr-test <image> [left top width height]  (prints to stdout)
    // diagnostic: Flashless.exe --capture-test "<window title>"  (GPU crop path on any window, prints to stdout)
    if args.first().is_some_and(|a| a == "--capture-test") {
        capture_test(args.get(1).map(String::as_str).unwrap_or("Flashless"));
        return;
    }
    // diagnostic: Flashless.exe --check-update [--download]  (latest release; --download fetches and verifies it, installs nothing)
    if args.first().is_some_and(|a| a == "--check-update") {
        match update::latest() {
            Some(r) => {
                println!("latest {} (this is {}), {} bytes, sha256 {}", r.version, env!("CARGO_PKG_VERSION"), r.size, r.sha256.as_deref().unwrap_or("not listed"));
                if has("--download") {
                    match update::download(&r) { Ok(p) => println!("downloaded and verified: {}", p.display()), Err(e) => println!("download: {e}") }
                }
            }
            None => println!("no release found"),
        }
        return;
    }
    if args.first().is_some_and(|a| a == "--ocr-test") {
        ocr_test(&args[1..]);
        return;
    }

    // uninstaller (Settings > Apps runs "Flashless.exe --uninstall")
    if has("--uninstall") {
        if silent || win::message_box("Remove Flashless from this PC?\n\nYour settings in %LOCALAPPDATA%\\Flashless are kept.", "Uninstall Flashless", true) {
            win::shell::uninstall();
            if !silent { win::message_box("Flashless was removed.", "Uninstall Flashless", false); }
        }
        return;
    }

    // installer (the shared file is named Flashless-Setup.exe)
    if win::shell::should_install(&args) {
        if silent {
            if let Err(e) = win::shell::install(!has("--no-desktop"), !has("--no-start-menu")) { eprintln!("{e}"); std::process::exit(1); }
            // in-app updates pass --relaunch so the new version comes back up by itself
            if has("--relaunch") { win::shell::launch_installed(); }
            return;
        }
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_title("Flashless Setup").with_inner_size([560.0, 620.0]).with_resizable(false)
                .with_maximize_button(false).with_icon(Arc::new(ui::theme::icon_data())),
            centered: true,
            ..Default::default()
        };
        let _ = eframe::run_native("Flashless Setup", options, Box::new(|cc| Ok(Box::new(ui::installer::Installer::new(cc)))));
        return;
    }

    // the overlay
    if !win::single_instance("Flashless.SingleInstance") {
        win::tray::show_existing(); // already running: show that overlay instead of a second copy
        return;
    }
    win::lower_priority();
    let s = settings::Settings::load();
    let mut vb = egui::ViewportBuilder::default().with_title("Flashless").with_decorations(false).with_transparent(true)
        .with_resizable(false).with_taskbar(true).with_inner_size([ui::app::CARD_W + 2.0 * ui::app::MARGIN, 300.0])
        .with_icon(Arc::new(ui::theme::icon_data()));
    if s.always_on_top { vb = vb.with_always_on_top(); }
    let (left, top) = win::clamp_to_screen(s.left.unwrap_or(40.0), s.top.unwrap_or(200.0), ui::app::CARD_W);
    vb = vb.with_position([left, top]);
    let options = eframe::NativeOptions { viewport: vb, ..Default::default() };
    let demo = has("--demo");
    let _ = eframe::run_native("Flashless", options, Box::new(move |cc| Ok(Box::new(ui::app::App::new(cc, demo)))));
}

fn ocr_test(args: &[String]) {
    use win::ocr;
    let Some(path) = args.first() else { println!("usage: --ocr-test <image> [l t w h]"); return };
    let full = match ocr::load_image(std::path::Path::new(path)) { Ok(f) => f, Err(e) => { println!("load failed: {e}"); return } };
    let Some(engine) = ocr::Ocr::new() else { println!("no OCR language"); return };
    let region: Vec<f64> = args[1..].iter().filter_map(|a| a.parse().ok()).collect();
    let region = if region.len() == 4 { [region[0], region[1], region[2], region[3]] } else { settings::DEFAULT_CHAT_REGION };
    let t = std::time::Instant::now();
    let frame = ocr::upscale(&ocr::whiten_red(&ocr::crop_region(&full, region)), 2);
    match engine.read_rows(&frame) {
        Ok(rows) => {
            println!("image {}x{}, region {:?}, OCR {} ms", full.w, full.h, region, t.elapsed().as_millis());
            let data = data::GameData::embedded();
            let mut enemies = Vec::new();
            for name in args.iter().skip(5) {
                if let Some(c) = data.find_champion(name) {
                    let mut e = tracker::Enemy { champion_id: c.id.clone(), name: c.name.clone(), spell1_id: Some("SummonerFlash".into()), ..Default::default() };
                    e.spell2_id = Some("SummonerBarrier".into());
                    enemies.push(e);
                }
            }
            for r in rows {
                let det = chat::parse(&r.text, &enemies, &data).map(|d| format!("  -> {} {:?} {:?}:{:?}", enemies[d.enemy].name, d.slot, d.minutes, d.seconds)).unwrap_or_default();
                println!("  {}{det}", r.text);
            }
        }
        Err(e) => println!("OCR failed: {e}"),
    }
    let t = std::time::Instant::now();
    match ocr::detect_chat_area(&engine, &full) {
        Ok((r, n)) => println!("auto-detect: {n} chat lines, region {:?} ({} ms)", r, t.elapsed().as_millis()),
        Err(e) => println!("auto-detect: {e}"),
    }
}

fn capture_test(title: &str) {
    let Some(hwnd) = win::find_window(title) else { println!("window '{title}' not found"); return };
    let mut cap = match win::capture::WindowCapture::new(hwnd) { Ok(c) => c, Err(e) => { println!("capture init failed: {e}"); return } };
    for region in [[0.0, 0.0, 1.0, 1.0], [0.0, 0.40, 0.30, 0.57], [0.25, 0.25, 0.5, 0.5]] {
        let mut got = None;
        let t = std::time::Instant::now();
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if let Ok(Some(f)) = cap.capture(region) { got = Some(f); break; }
        }
        match got {
            Some(f) => {
                let n = (f.px.len() / 4) as f64;
                let mean: f64 = f.px.chunks_exact(4).map(|p| (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0).sum::<f64>() / n;
                println!("region {:?}: {}x{} px, mean brightness {:.0}, text fingerprint {:016x}, {} ms", region, f.w, f.h, mean, win::ocr::text_fingerprint(&f), t.elapsed().as_millis());
            }
            None => println!("region {:?}: no frame", region),
        }
    }
}
