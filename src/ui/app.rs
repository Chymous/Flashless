//! The overlay: enemy rows with summoner spell tiles, the chat message preview, and all background wiring
//! (live game API, hotkeys, chat reading, clipboard, typing, data updates).

use super::setup::SetupState;
use super::theme::{self, Pal};
use crate::chat;
use crate::cooldown::CooldownRules;
use crate::data::GameData;
use crate::live::{self, LiveSnapshot};
use crate::message::{self, TimeFormat};
use crate::settings::{data_dir, Settings};
use crate::tracker::{Enemy, GameClock, Slot, Tracker, SLOTS};
use crate::win::{self, clipboard, hotkeys, typing, watcher};
use eframe::egui::{self, Align2, Color32, Rect, Rounding, Sense, Stroke, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const CARD_W: f32 = 372.0;
pub const MARGIN: f32 = 14.0;
const TILE: Vec2 = Vec2::new(104.0, 42.0);
const TILE_GAP: f32 = 4.0;
const MINI_ICON: f32 = 30.0;
const MINI_GAP: f32 = 4.0;
const MINI_ROLE_W: f32 = 34.0;

pub struct App {
    pub settings: Settings,
    pub data: GameData,
    pub tracker: Tracker,
    pub clock: GameClock,
    pub pal: Pal,
    filter: chat::Filter,
    live_rx: Receiver<Option<LiveSnapshot>>,
    hotkey_rx: Receiver<String>,
    watch_rx: Receiver<watcher::WatchEvent>,
    data_rx: Receiver<GameData>,
    status_tx: Sender<String>,
    status_rx: Receiver<String>,
    watcher: watcher::Watcher,
    flash: Option<(String, Instant)>,
    copy_due: Option<Instant>,
    seen_revision: u64,
    expanded: Option<usize>,
    pub setup: Option<SetupState>,
    hidden: bool,
    hwnd_done: bool,
    last_lines: HashSet<String>,
    last_detections: HashMap<(String, Slot), f64>,
    last_size: Vec2,
    last_pos: Option<egui::Pos2>,
    theme_mode: String,
    next_theme_check: Instant,
    demo: bool,
    /// Unreadable-time chat lines waiting to be timed from when they first appeared.
    sightings: HashMap<(String, Slot), Sighting>,
    last_scan: Option<f64>,
    /// Flash tiles pulsing because the spell just came back: (champion, slot) -> until.
    alerts: HashMap<(String, Slot), Instant>,
    passthrough: bool,
    was_live: bool,
    /// Shown anyway this lobby, after the user asked to see it while "hide outside game" is on.
    force_show: bool,
    update: Option<crate::update::Release>,
    update_rx: Receiver<crate::update::Release>,
    /// In-app update in progress: the downloaded setup (or why it failed) arrives here.
    updating: Option<Receiver<Result<std::path::PathBuf, String>>>,
    /// Your Riot ID from the live game.
    me: String,
    #[cfg(feature = "personal")]
    riot: Option<Receiver<Result<Vec<crate::riot::RunePage>, String>>>,
    /// Next rune lookup attempt for this game (personal builds), and attempts so far.
    #[cfg(feature = "personal")]
    riot_retry: Option<(Instant, u32)>,
    #[cfg(feature = "personal")]
    riot_retry_count: u32,
    /// Asked for from Settings.
    #[cfg(feature = "personal")]
    pub riot_lookup_now: bool,
}

/// A chat detection whose time could not be read yet.
struct Sighting {
    first_seen: f64,
    /// Settled: registered, or judged old / repeated / already on cooldown. No fallback then.
    settled: bool,
    /// Appeared while chat was being watched and few new lines came at once (not a history pop-up).
    trustworthy: bool,
    enemy: usize,
    line: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, demo: bool) -> App {
        let ctx = cc.egui_ctx.clone();
        theme::install_fonts(&ctx);
        let settings = Settings::load();
        let pal = theme::resolve(&settings.theme);
        theme::apply(&ctx, &pal);

        let wake: Arc<dyn Fn() + Send + Sync> = { let c = ctx.clone(); Arc::new(move || c.request_repaint()) };
        let (live_tx, live_rx) = channel();
        live::start_poller(live_tx, wake.clone());
        let (hk_tx, hotkey_rx) = channel();
        win::tray::start(hk_tx.clone(), wake.clone()); // tray menu actions share the hotkey channel
        hotkeys::start(hk_tx, wake.clone());
        let (w_tx, watch_rx) = channel();
        let watcher = watcher::Watcher::start(watch_config(&settings), w_tx, wake.clone());
        let (status_tx, status_rx) = channel();

        let cache = data_dir().join("gamedata.json");
        let data = GameData::load(&cache);
        let (d_tx, data_rx) = channel();
        let version = data.version.clone();
        let w2 = wake.clone();
        std::thread::spawn(move || {
            if let Ok(Some(d)) = crate::data::update(&version, &cache) { let _ = d_tx.send(d); w2(); }
        });

        crate::update::cleanup();
        let (u_tx, update_rx) = channel();
        if settings.check_updates && !crate::update::REPO.is_empty() {
            let w3 = wake.clone();
            std::thread::spawn(move || { if let Some(r) = crate::update::check() { let _ = u_tx.send(r); w3(); } });
        }

        let mut app = App {
            theme_mode: settings.theme.clone(),
            settings, data, tracker: Tracker::default(), clock: GameClock::default(), pal,
            filter: chat::Filter::default(), live_rx, hotkey_rx, watch_rx, data_rx, status_tx, status_rx, watcher,
            flash: None, copy_due: None, seen_revision: 0, expanded: None, setup: None, hidden: false, hwnd_done: false,
            last_lines: HashSet::new(), last_detections: HashMap::new(), last_size: Vec2::ZERO, last_pos: None, next_theme_check: Instant::now(),
            demo, sightings: HashMap::new(), last_scan: None, alerts: HashMap::new(), passthrough: false, was_live: false,
            force_show: false, update: None, update_rx, updating: None, me: String::new(),
            #[cfg(feature = "personal")] riot_lookup_now: false,
            #[cfg(feature = "personal")] riot: None,
            #[cfg(feature = "personal")] riot_retry: None,
            #[cfg(feature = "personal")] riot_retry_count: 0,
        };
        let failed = hotkeys::set_bindings(&app.settings.hotkeys);
        if !failed.is_empty() { app.flash(format!("bad or duplicate hotkey: {}", failed.join(", "))); }
        if demo { app.load_demo(); }
        app.seen_revision = app.tracker.cooldown_revision;
        app
    }

    fn rules(&self) -> &CooldownRules { &self.settings.rules }

    /// Personal builds: looks up the enemies' rune pages once per game, retrying while the game is not listed yet.
    #[cfg(feature = "personal")]
    fn pump_riot(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.riot_lookup_now) { self.riot_retry = Some((Instant::now(), 0)); }
        if self.riot.is_none() && crate::riot::has_key() && self.clock.is_live && !self.me.is_empty() {
            if let Some((at, n)) = self.riot_retry {
                if Instant::now() >= at {
                    self.riot_retry = None;
                    let (tx, rx) = channel();
                    let (region, me, c) = (self.settings.riot_region.clone(), self.me.clone(), ctx.clone());
                    std::thread::spawn(move || { let _ = tx.send(crate::riot::enemy_runes(&region, &me)); c.request_repaint(); });
                    self.riot = Some(rx);
                    self.riot_retry_count = n;
                }
            }
        }
        let Some(rx) = &self.riot else { return };
        let Ok(res) = rx.try_recv() else { return };
        self.riot = None;
        match res {
            Ok(pages) => {
                let now = self.clock.now();
                let (data, rules) = (self.data.clone(), self.rules().clone());
                let mut with_ci = Vec::new();
                let mut matched = 0;
                for i in 0..self.tracker.enemies.len() {
                    let id = self.tracker.enemies[i].riot_id.to_lowercase();
                    let Some(page) = pages.iter().find(|p| !id.is_empty() && p.riot_id.to_lowercase() == id) else { continue };
                    matched += 1;
                    let e = &mut self.tracker.enemies[i];
                    e.runes_known = true;
                    e.has_inspiration = page.inspiration;
                    let changed = e.cosmic_insight != page.cosmic_insight;
                    e.cosmic_insight = page.cosmic_insight;
                    if page.cosmic_insight { with_ci.push(e.name.clone()); }
                    if changed { self.tracker.recalculate(i, now, true, &data, &rules); }
                }
                let msg = if with_ci.is_empty() { format!("runes read for {matched} enemies: no Cosmic Insight") } else { format!("Cosmic Insight: {}", with_ci.join(", ")) };
                self.chat_log(&format!("RIOT API {msg}"));
                self.flash(msg);
            }
            Err(e) => {
                self.chat_log(&format!("RIOT API {e}"));
                // the game can take a minute to be listed; retry a few times, then give up for this game
                let n = self.riot_retry_count + 1;
                if e.starts_with("no live game") && n < 4 { self.riot_retry = Some((Instant::now() + Duration::from_secs(45), n)); }
                else { self.flash(format!("Riot API: {e}")); }
            }
        }
    }

    /// Shows (and logs) what the tracker worked out by itself, such as a detected Cosmic Insight.
    fn show_tracker_note(&mut self) {
        if let Some(n) = self.tracker.note.take() { self.chat_log(&format!("INFERRED {n}")); self.flash(n); }
    }

    fn save_window_state(&mut self) {
        if let Some(p) = self.last_pos { self.settings.left = Some(p.x); self.settings.top = Some(p.y); }
        self.settings.save();
    }

    /// Downloads and installs the newer release. Not during a game: the restart would drop your timers.
    fn start_update(&mut self, ctx: &egui::Context) {
        if self.updating.is_some() { return; }
        let Some(r) = self.update.clone() else { return };
        if self.clock.is_live { self.flash("update after the game"); return; }
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || { let _ = tx.send(crate::update::download(&r)); c.request_repaint(); });
        self.updating = Some(rx);
        self.flash(format!("downloading {}...", self.update.as_ref().map(|u| u.version.clone()).unwrap_or_default()));
    }

    /// Hidden by the "hide outside game" option right now.
    fn auto_hidden(&self) -> bool {
        self.settings.hide_outside_game && !self.clock.is_live && !self.demo && self.setup.is_none() && !self.force_show
    }

    fn is_hidden(&self) -> bool { self.hidden || self.auto_hidden() }

    pub fn flash(&mut self, text: impl Into<String>) { self.flash = Some((text.into(), Instant::now() + Duration::from_millis(1800))); }

    fn load_demo(&mut self) {
        let mk = |id: &str, s2: &str, lvl: i32, items: &[i64], role: &str, data: &GameData| Enemy {
            champion_id: id.into(), name: data.find_champion(id).map(|c| c.name.clone()).unwrap_or_else(|| id.into()),
            level: lvl, item_ids: items.to_vec(), spell1_id: Some("SummonerFlash".into()), spell2_id: Some(s2.into()), role: Some(role.into()), ..Default::default()
        };
        let d = &self.data;
        let mut roster = vec![
            mk("Lucian", "SummonerBarrier", 11, &[3158], "ADC", d), mk("Nautilus", "SummonerDot", 9, &[], "Sup", d),
            mk("Syndra", "SummonerTeleport", 12, &[], "Mid", d), mk("LeeSin", "SummonerSmite", 10, &[], "Jg", d), mk("Shen", "SummonerTeleport", 11, &[], "Top", d),
        ];
        roster[2].has_inspiration = true;
        self.tracker.set_roster(roster);
        let now = self.clock.now();
        let (data, rules) = (self.data.clone(), self.rules().clone());
        for (i, slot, ago) in [(0, Slot::Spell1, 0.0), (0, Slot::Spell2, 40.0), (1, Slot::Spell1, 120.0), (4, Slot::Spell2, 200.0)] {
            self.tracker.register(i, slot, ago, now, false, &data, &rules);
        }
    }

    /// Left and right tile for enemy `i`, per the spell order setting.
    fn display_slots(&self, i: usize) -> [Slot; 2] {
        let e = &self.tracker.enemies[i];
        let is_flash = |slot| self.data.find_spell(e.spell_id(slot)).is_some_and(|s| s.name == "Flash");
        self.settings.spell_order.slots(is_flash(Slot::Spell1), is_flash(Slot::Spell2))
    }

    pub fn message(&self) -> String {
        message::build(&self.tracker.enemies, &self.data, self.clock.now(), self.clock.is_live || self.clock.seen_live, &self.settings.message)
    }

    pub fn copy_now(&mut self) {
        let m = self.message();
        if m.is_empty() { return; }
        let ok = clipboard::set_text(&m);
        self.flash(if ok { "copied" } else { "clipboard busy" });
    }

    fn register(&mut self, i: usize, slot: Slot) {
        let (data, rules) = (self.data.clone(), self.rules().clone());
        if !self.tracker.register(i, slot, self.settings.reaction_offset_seconds, self.clock.now(), self.clock.is_live, &data, &rules) {
            self.flash("no cooldown data for that");
        }
        self.show_tracker_note();
    }

    fn type_in_chat(&mut self) {
        let msg = self.message();
        if msg.is_empty() { self.flash("nothing to type"); return; }
        let (tx, delay, copy) = (self.status_tx.clone(), self.settings.chat_key_delay_ms, self.settings.copy_in_chat_after_typing);
        std::thread::spawn(move || { let _ = tx.send(typing::type_in_chat(&msg, delay, copy)); });
    }

    fn on_hotkey(&mut self, action: &str, ctx: &egui::Context) {
        match action {
            "CopySummary" => self.copy_now(),
            "TypeInChat" => self.type_in_chat(),
            "Undo" => { self.tracker.undo(); }
            "ResetAll" => self.tracker.reset_cooldowns(),
            "OpenSettings" => {
                self.hidden = false;
                if self.setup.is_none() { self.setup = Some(SetupState::new(self)); }
            }
            "Exit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            "ToggleCompact" => {
                self.settings.compact = !self.settings.compact;
                self.expanded = None;
                self.settings.save();
            }
            "ShowOverlay" => {
                self.hidden = false;
                if self.auto_hidden() { self.force_show = true; }
                self.flash("already running");
            }
            "ToggleOverlay" => {
                if self.auto_hidden() { self.force_show = true; self.hidden = false; } else { self.hidden = !self.hidden; }
            }
            a if a.starts_with("Enemy") && a.len() > 6 => {
                let Some(i) = a[5..6].parse::<usize>().ok().and_then(|n| n.checked_sub(1)) else { return };
                let Some(e) = self.tracker.enemies.get(i) else { return };
                let is_flash = |slot| self.data.find_spell(e.spell_id(slot)).is_some_and(|s| s.name == "Flash");
                let flash_second = !is_flash(Slot::Spell1) && is_flash(Slot::Spell2);
                let (flash, other) = if flash_second { (Slot::Spell2, Slot::Spell1) } else { (Slot::Spell1, Slot::Spell2) };
                match &a[6..] {
                    "Flash" => self.register(i, flash),
                    "OtherSpell" => self.register(i, other),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// New match: chat.log -> chat.1.log -> chat.2.log, so the logs hold the last three games.
    fn rotate_chat_log(&self) {
        let dir = data_dir();
        let _ = std::fs::remove_file(dir.join("chat.2.log"));
        let _ = std::fs::rename(dir.join("chat.1.log"), dir.join("chat.2.log"));
        let _ = std::fs::rename(dir.join("chat.log"), dir.join("chat.1.log"));
        let _ = std::fs::remove_file(dir.join("chat.log.old"));
    }

    fn chat_log(&self, text: &str) {
        if !self.settings.chat_log { return; }
        use std::io::Write;
        let path = data_dir().join("chat.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1_000_000) { self.rotate_chat_log(); }
        let _ = std::fs::create_dir_all(data_dir());
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let t = self.clock.now() as i64;
            let _ = writeln!(f, "game {}:{:02}  {}", t / 60, t % 60, text);
        }
    }

    /// Registers "Enemy used Spell" lines read from the game chat.
    fn on_chat_lines(&mut self, lines: Vec<String>) {
        if !self.settings.chat_detection || !self.clock.is_live { return; }
        let now = self.clock.now();
        // a history pop-up (chat opened) shows many old lines at once; only trust lines that came in ones and twos
        // while chat was being watched
        let watched = self.last_scan.is_some_and(|t| now - t <= 3.0);
        self.last_scan = Some(now);
        let new_used = lines.iter().filter(|l| !self.last_lines.contains(*l) && l.to_lowercase().contains("used")).count();
        let trustworthy = watched && new_used <= 2;
        let mut detections = HashMap::new();
        for line in &lines {
            let is_new = !self.last_lines.contains(line);
            let Some(d) = chat::parse(line, &self.tracker.enemies, &self.data) else {
                if is_new && line.to_lowercase().contains("used") { self.chat_log(&format!("UNMATCHED  {line}")); }
                continue;
            };
            let champ = self.tracker.enemies[d.enemy].champion_id.clone();
            let id = (champ.clone(), d.slot);
            // first seen within the last few seconds: a line can take a scan or two before its time reads cleanly
            let first_seen = self.last_detections.get(&id).copied().unwrap_or(self.clock.now());
            let just_appeared = self.clock.now() - first_seen <= 4.0;
            detections.insert(id.clone(), first_seen);
            let mut remaining = self.tracker.remaining(d.enemy, d.slot, self.clock.now());
            // still "on cooldown" here, but Cosmic Insight would have it back by now: let the use through
            // (register() then marks the enemy as having it)
            if remaining > 15.0 && self.tracker.cosmic_insight_explains(d.enemy, d.slot, self.clock.now(), &self.settings.rules) { remaining = 0.0; }
            let res = self.filter.accept(&d, &champ, self.clock.now(), remaining, just_appeared);
            let sighting = self.sightings.entry(id.clone()).or_insert(Sighting { first_seen, settled: false, trustworthy, enemy: d.enemy, line: line.clone() });
            sighting.enemy = d.enemy;
            match &res {
                Ok(_) => sighting.settled = true,
                // the time could not be judged yet: the first-sighting fallback below may still time it
                Err(e) if e.starts_with("no readable timestamp") || e.starts_with("only seconds readable") => {}
                Err(_) => sighting.settled = true, // old, repeated, or already on cooldown
            }
            let stamp = format!("{}:{}", d.minutes.map(|m| format!("{m:02}")).unwrap_or("??".into()), d.seconds.map(|s| format!("{s:02}")).unwrap_or("??".into()));
            let reason = match &res { Ok(ago) => format!("registered, used {ago:.0}s ago"), Err(e) => e.clone() };
            if is_new || res.is_ok() { self.chat_log(&format!("{champ} {:?} {stamp} -> {reason}  | {line}", d.slot)); }
            if let Ok(ago) = res {
                let (data, rules) = (self.data.clone(), self.rules().clone());
                // time since the chat line, plus the time it took someone to see the spell and call it
                let ago = ago + self.settings.reaction_offset_seconds;
                if self.tracker.register(d.enemy, d.slot, ago, self.clock.now(), true, &data, &rules) {
                    let what = data.find_spell(self.tracker.enemies[d.enemy].spell_id(d.slot)).map(|x| x.name.clone()).unwrap_or("spell".into());
                    let who = self.tracker.enemies[d.enemy].name.clone();
                    self.flash(format!("chat: {who} {what}"));
                    self.show_tracker_note();
                }
            }
        }
        // lines whose time never read cleanly: after 3 s of tries, time them from when they first appeared
        let due: Vec<(String, Slot)> = self.sightings.iter()
            .filter(|(id, s)| detections.contains_key(*id) && !s.settled && s.trustworthy && now - s.first_seen >= 3.0)
            .map(|(id, _)| id.clone()).collect();
        for id in due {
            let s = self.sightings.get_mut(&id).unwrap();
            s.settled = true;
            let (i, ago, line) = (s.enemy, now - s.first_seen, s.line.clone());
            if self.tracker.enemies.get(i).is_none_or(|e| e.champion_id != id.0) { continue; }
            if self.tracker.remaining(i, id.1, now) > 15.0 && !self.tracker.cosmic_insight_explains(i, id.1, now - ago, &self.settings.rules) { continue; }
            let (data, rules) = (self.data.clone(), self.rules().clone());
            if self.tracker.register(i, id.1, ago + self.settings.reaction_offset_seconds, now, true, &data, &rules) {
                self.chat_log(&format!("{} {:?} -> registered from first sighting, {ago:.0}s ago  | {line}", id.0, id.1));
                let what = data.find_spell(self.tracker.enemies[i].spell_id(id.1)).map(|x| x.name.clone()).unwrap_or("spell".into());
                let who = self.tracker.enemies[i].name.clone();
                self.flash(format!("chat: {who} {what}"));
                self.show_tracker_note();
            }
        }
        // a line that scrolled away ends its sighting
        self.sightings.retain(|id, _| detections.contains_key(id));
        self.last_lines = lines.into_iter().collect();
        self.last_detections = detections;
    }

    pub fn apply_settings(&mut self, ctx: &egui::Context) {
        if self.settings.theme != self.theme_mode {
            self.theme_mode = self.settings.theme.clone();
        }
        self.pal = theme::resolve(&self.settings.theme);
        theme::apply(ctx, &self.pal);
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if self.settings.always_on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal }));
        *self.watcher.config.lock().unwrap() = watch_config(&self.settings);
        let failed = hotkeys::set_bindings(&self.settings.hotkeys);
        if !failed.is_empty() { self.flash(format!("bad or duplicate hotkey: {}", failed.join(", "))); }
        self.settings.save();
    }

    /// Restores the saved theme after Setup previewed another one and was cancelled.
    pub fn apply_settings_theme_only(&mut self, ctx: &egui::Context) {
        self.pal = theme::resolve(&self.settings.theme);
        theme::apply(ctx, &self.pal);
    }

    fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(snap) = self.live_rx.try_recv() {
            match snap {
                Some(s) => {
                    self.clock.sync_live(s.game_time);
                    if !s.me.is_empty() { self.me = s.me.clone(); }
                    let before: Vec<String> = self.tracker.enemies.iter().map(|e| e.champion_id.clone()).collect();
                    if self.tracker.apply_live(&s, &self.data, &self.settings.rules) {
                        self.filter.reset();
                        if !self.tracker.enemies.iter().any(|e| before.contains(&e.champion_id)) {
                            self.rotate_chat_log();
                            #[cfg(feature = "personal")]
                            { self.riot_retry = Some((Instant::now(), 0)); }
                        }
                        let names: Vec<&str> = self.tracker.enemies.iter().map(|e| e.champion_id.as_str()).collect();
                        self.chat_log(&format!("ROSTER {}", names.join(", ")));
                    }
                }
                None => if self.clock.is_live { self.clock.go_offline(); },
            }
        }
        let actions: Vec<String> = self.hotkey_rx.try_iter().collect();
        for a in actions { self.on_hotkey(&a, ctx); }
        while let Ok(ev) = self.watch_rx.try_recv() {
            match ev {
                watcher::WatchEvent::Lines(l) => self.on_chat_lines(l),
                watcher::WatchEvent::Stats { capture_ms, ocr_ms, scans, ocr_runs } =>
                    self.chat_log(&format!("STATS wgc: {scans} scans, avg capture {capture_ms} ms, {ocr_runs} OCR runs, avg OCR {ocr_ms} ms")),
                watcher::WatchEvent::Problem(p) => { self.chat_log(&format!("PROBLEM {p}")); self.flash(p); }
            }
        }
        if let Ok(d) = self.data_rx.try_recv() { self.flash(format!("data updated to {}", d.version)); self.data = d; }
        if let Ok(s) = self.status_rx.try_recv() { self.flash(s); }

        #[cfg(feature = "personal")]
        self.pump_riot(ctx);
        if let Ok(r) = self.update_rx.try_recv() { self.flash(format!("Flashless {} is available", r.version)); self.update = Some(r); }
        if let Some(rx) = &self.updating {
            if let Ok(res) = rx.try_recv() {
                self.updating = None;
                match res.and_then(|setup| { self.save_window_state(); crate::update::run_installer(&setup) }) {
                    // the installer closes this copy, replaces it and starts the new one
                    Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                    Err(e) => self.flash(format!("update failed: {e}")),
                }
            }
        }
        let now = self.clock.now();
        if self.settings.flash_alert {
            let mut fresh = Vec::new();
            for e in &self.tracker.enemies {
                for (&slot, c) in &e.cooldowns {
                    // back up within the last 2 s (not old timers expiring after a restart)
                    if c.ready_at <= now && now - c.ready_at < 2.0 && self.data.find_spell(e.spell_id(slot)).is_some_and(|s| s.name == "Flash") {
                        fresh.push((e.champion_id.clone(), slot));
                    }
                }
            }
            if !fresh.is_empty() && self.settings.flash_alert_sound { win::notify_sound(); }
            for k in fresh { self.alerts.insert(k, Instant::now() + Duration::from_millis(3500)); }
        }
        self.alerts.retain(|_, until| Instant::now() < *until);
        self.tracker.prune(now);
        // lobby <-> game
        if self.clock.is_live != self.was_live {
            self.was_live = self.clock.is_live;
            self.force_show = false;
            if self.settings.auto_compact && self.settings.compact != self.clock.is_live { self.settings.compact = self.clock.is_live; self.expanded = None; }
        }
        let pass = self.is_hidden() || (self.settings.click_through_in_game && self.setup.is_none() && win::game_is_foreground());
        if pass != self.passthrough { self.passthrough = pass; ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(pass)); }
        if self.tracker.cooldown_revision != self.seen_revision {
            self.seen_revision = self.tracker.cooldown_revision;
            if self.settings.auto_copy { self.copy_due = Some(Instant::now() + Duration::from_millis(self.settings.copy_delay_ms.clamp(1, 5000))); }
        }
        if self.copy_due.is_some_and(|t| Instant::now() >= t) { self.copy_due = None; self.copy_now(); }
        self.watcher.allowed.store(self.settings.chat_detection && self.clock.is_live && !self.tracker.enemies.is_empty(), std::sync::atomic::Ordering::Relaxed);
        if self.settings.theme == "System" && Instant::now() >= self.next_theme_check {
            self.next_theme_check = Instant::now() + Duration::from_secs(3);
            let p = theme::resolve("System");
            if p.dark != self.pal.dark { self.pal = p; theme::apply(ctx, &p); }
        }
    }

    fn status_text(&self) -> String {
        if let Some((t, until)) = &self.flash { if Instant::now() < *until { return t.clone(); } }
        let t = self.clock.now() as i64;
        if self.clock.is_live { format!("live {}:{:02}", t / 60, t % 60) } else { "no game".into() }
    }
}

pub fn watch_config(s: &Settings) -> watcher::WatchConfig {
    watcher::WatchConfig { region: s.chat_region, scale: s.chat_ocr_scale as usize, interval_ms: s.chat_scan_interval_ms }
}

impl eframe::App for App {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] { [0.0; 4] }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !self.hwnd_done {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = frame.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    win::make_no_activate(windows::Win32::Foundation::HWND(w.hwnd.get() as _));
                    self.hwnd_done = true;
                }
            }
        }
        self.pump(ctx);
        // timers show whole seconds: redraw when the next second ticks over (events wake it sooner)
        let mut next = Duration::from_secs_f64((1.0 - self.clock.now().fract()).max(0.02) + 0.005);
        if let Some(t) = self.copy_due { next = next.min(t.saturating_duration_since(Instant::now())); }
        if !self.alerts.is_empty() { next = next.min(Duration::from_millis(33)); }
        // League in front with click-through: the overlay cannot be hovered, so check focus again soon
        if self.settings.click_through_in_game { next = next.min(Duration::from_millis(500)); }
        ctx.request_repaint_after(next);
        if let Some(r) = ctx.input(|i| i.viewport().outer_rect) { self.last_pos = Some(r.min); }

        let p = self.pal;
        let panel = egui::Frame::none().inner_margin(egui::Margin::same(MARGIN));
        egui::CentralPanel::default().frame(panel).show(ctx, |ui| {
            if self.is_hidden() { return; }
            ui.set_opacity(self.settings.opacity.clamp(0.3, 1.0));
            let compact = self.settings.compact;
            let card_w = if compact { mini_width() } else { CARD_W };
            let card = egui::Frame::none().fill(p.surface).stroke(Stroke::new(1.0_f32, p.line_strong)).rounding(Rounding::same(if compact { 10.0 } else { 12.0 }))
                .shadow(egui::epaint::Shadow { offset: egui::vec2(0.0, 3.0), blur: 16.0, spread: 0.0, color: p.shadow });
            let resp = card.show(ui, |ui| {
                ui.set_width(card_w);
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                if compact {
                    self.mini_header(ui, ctx, card_w);
                    egui::Frame::none().inner_margin(egui::Margin { left: 8.0, right: 8.0, top: 2.0, bottom: 8.0 }).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, MINI_GAP);
                        self.mini_rows(ui);
                    });
                    return;
                }
                self.header(ui, ctx);
                let (r, _) = ui.allocate_exact_size(egui::vec2(CARD_W, 1.0), Sense::hover());
                ui.painter().rect_filled(r, 0.0_f32, p.line);
                egui::Frame::none().inner_margin(egui::Margin { left: 12.0, right: 12.0, top: 10.0, bottom: 12.0 }).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                    self.rows(ui);
                    if self.tracker.enemies.is_empty() {
                        ui.add(egui::Label::new(egui::RichText::new("Waiting for a match. Enemies appear automatically once the game has loaded. To track without a game, add them under Settings.").color(p.muted).size(11.5)).wrap());
                    }
                    if self.settings.show_preview { self.preview(ui); }
                    self.footer(ui);
                });
            });
            // fit the window to the card
            let want = resp.response.rect.size() + egui::vec2(MARGIN * 2.0, MARGIN * 2.0);
            if (want - self.last_size).length() > 1.0 {
                self.last_size = want;
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(want));
            }
        });

        if self.setup.is_some() { super::setup::show(self, ctx); }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        win::tray::remove();
        self.save_window_state();
    }
}

// ---------- drawing ----------

impl App {
    fn header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.pal;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(CARD_W, 42.0), Sense::click_and_drag());
        if resp.drag_started() { ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); }
        let painter = ui.painter_at(rect);
        let cy = rect.center().y;
        theme::brand(&painter, egui::pos2(rect.left() + 22.0, cy), 9.0, &p);
        let name = painter.text(egui::pos2(rect.left() + 38.0, cy), Align2::LEFT_CENTER, "Flashless", theme::semi(13.5), p.text);
        let title = painter.text(egui::pos2(name.right() + 5.0, cy + 1.0), Align2::LEFT_CENTER, if cfg!(feature = "personal") { concat!("v", env!("CARGO_PKG_VERSION"), "-p") } else { concat!("v", env!("CARGO_PKG_VERSION")) }, theme::regular(10.0), p.muted);
        // status pill
        let status = self.status_text();
        let galley = painter.layout_no_wrap(status, theme::regular(10.5), p.muted);
        let pill = Rect::from_min_size(egui::pos2(title.right() + 10.0, cy - 10.0), egui::vec2(galley.size().x + 26.0, 20.0));
        painter.rect(pill, Rounding::same(10.0), p.bg, Stroke::new(1.0_f32, p.line));
        painter.circle_filled(egui::pos2(pill.left() + 11.0, cy), 3.0, if self.clock.is_live { p.ready } else { p.muted });
        painter.galley(egui::pos2(pill.left() + 18.0, cy - galley.size().y / 2.0), galley, p.muted);
        // icon buttons, right aligned
        let icons = [("\u{E7A7}", "Undo the last change"), ("\u{E72C}", "Clear all tracked cooldowns"), ("\u{E73F}", "Compact mode"), ("\u{E713}", "Settings: roster, chat reading, appearance, hotkeys"), ("\u{E8BB}", "Exit")];
        let mut x = rect.right() - 10.0 - 26.0 * icons.len() as f32;
        for (i, (glyph, tip)) in icons.iter().enumerate() {
            let r = Rect::from_min_size(egui::pos2(x, cy - 12.0), egui::vec2(26.0, 24.0));
            x += 26.0;
            let b = ui.interact(r, ui.id().with(("hdr", i)), Sense::click()).on_hover_text(*tip);
            if b.hovered() { ui.painter().rect_filled(r, 6.0_f32, p.surface_hover); }
            ui.painter().text(r.center(), Align2::CENTER_CENTER, *glyph, theme::icon(if i == 4 { 10.0 } else { 12.0 }), if b.hovered() { p.accent_deep } else { p.muted });
            if b.clicked() {
                match i {
                    0 => { self.tracker.undo(); }
                    1 => self.tracker.reset_cooldowns(),
                    2 => { self.settings.compact = true; self.expanded = None; self.settings.save(); }
                    3 => if self.setup.is_none() { self.setup = Some(SetupState::new(self)); hotkeys::set_paused(false); },
                    _ => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                }
            }
        }
    }

    fn rows(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let now = self.clock.now();
        let gctx = self.tracker.context(now, self.clock.is_live);
        let name_w = CARD_W - 24.0 - 2.0 * (TILE.x + TILE_GAP);
        for i in 0..self.tracker.enemies.len() {
            let row = ui.allocate_exact_size(egui::vec2(CARD_W - 24.0, TILE.y), Sense::hover()).0;
            // name block (click to tune)
            let name_rect = Rect::from_min_size(row.min, egui::vec2(name_w, TILE.y));
            let nresp = ui.interact(name_rect, ui.id().with(("name", i)), Sense::click()).on_hover_text("Click to set level and Cosmic Insight for this enemy");
            if nresp.clicked() { self.expanded = if self.expanded == Some(i) { None } else { Some(i) }; }
            let e = &self.tracker.enemies[i];
            let painter = ui.painter();
            let c = egui::pos2(name_rect.left() + 9.0, name_rect.center().y);
            painter.circle_filled(c, 9.0, p.accent_soft);
            painter.text(c, Align2::CENTER_CENTER, format!("{}", i + 1), theme::semi(10.0), p.accent_deep);
            let nx = name_rect.left() + 26.0;
            painter.text(egui::pos2(nx, name_rect.center().y - 7.0), Align2::LEFT_CENTER, &e.name, theme::semi(13.0), p.text);
            let lv = painter.text(egui::pos2(nx, name_rect.center().y + 9.0), Align2::LEFT_CENTER, format!("Lv {}", e.level), theme::regular(10.0), p.muted);
            let mut x = lv.right();
            if let Some(role) = &e.role {
                x = painter.text(egui::pos2(x, lv.center().y), Align2::LEFT_CENTER, format!("  ·  {role}"), theme::semi(10.0), p.accent_deep).right();
            }
            if e.has_inspiration && !e.cosmic_insight && !e.runes_known {
                painter.text(egui::pos2(x, lv.center().y), Align2::LEFT_CENTER, "  CI?", theme::semi(10.0), p.cooling);
            }
            for (k, slot) in self.display_slots(i).iter().enumerate() {
                let r = Rect::from_min_size(egui::pos2(row.right() - (2 - k) as f32 * (TILE.x + TILE_GAP) + TILE_GAP, row.top()), TILE);
                self.tile(ui, i, *slot, r, now, gctx, false);
            }
            if self.expanded == Some(i) { self.tune(ui, i); }
        }
    }

    /// Compact mode: lane label and the two spell icons per enemy, time drawn on the icon.
    fn mini_rows(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let now = self.clock.now();
        let gctx = self.tracker.context(now, self.clock.is_live);
        if self.tracker.enemies.is_empty() {
            ui.add(egui::Label::new(egui::RichText::new("Waiting for a match").size(11.0).color(p.muted)));
            return;
        }
        for i in 0..self.tracker.enemies.len() {
            let row = ui.allocate_exact_size(egui::vec2(mini_width() - 16.0, MINI_ICON), Sense::hover()).0;
            let e = &self.tracker.enemies[i];
            let label = e.role.clone().unwrap_or_else(|| e.name.chars().take(4).collect());
            ui.painter().text(egui::pos2(row.left() + 2.0, row.center().y), Align2::LEFT_CENTER, label, theme::semi(11.5), p.accent_deep);
            for (k, slot) in self.display_slots(i).iter().enumerate() {
                let r = Rect::from_min_size(egui::pos2(row.left() + MINI_ROLE_W + k as f32 * (MINI_ICON + MINI_GAP), row.top()), egui::vec2(MINI_ICON, MINI_ICON));
                self.tile(ui, i, *slot, r, now, gctx, true);
            }
        }
    }

    fn mini_header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, w: f32) {
        let p = self.pal;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 28.0), Sense::click_and_drag());
        if resp.drag_started() { ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); }
        resp.on_hover_text(format!("Flashless v{}\n{}", env!("CARGO_PKG_VERSION"), self.status_text()));
        let cy = rect.center().y;
        theme::brand(ui.painter(), egui::pos2(rect.left() + 16.0, cy), 7.0, &p);
        ui.painter().circle_filled(egui::pos2(rect.left() + 30.0, cy), 3.0, if self.clock.is_live { p.ready } else { p.muted });
        if let Some(v) = self.update.as_ref().map(|u| u.version.clone()) {
            let ur = Rect::from_center_size(egui::pos2(rect.left() + 44.0, cy), egui::vec2(16.0, 16.0));
            let ub = ui.interact(ur, ui.id().with("upd"), Sense::click()).on_hover_text(format!("Update to {v}: click to install"));
            ui.painter().text(ur.center(), Align2::CENTER_CENTER, "\u{E896}", theme::icon(10.0), if ub.hovered() { p.accent_deep } else { p.accent });
            if ub.clicked() { let ctx = ctx.clone(); self.start_update(&ctx); }
        }
        let r = Rect::from_min_size(egui::pos2(rect.right() - 30.0, cy - 11.0), egui::vec2(24.0, 22.0));
        let b = ui.interact(r, ui.id().with("expand"), Sense::click()).on_hover_text("Full view");
        if b.hovered() { ui.painter().rect_filled(r, 6.0_f32, p.surface_hover); }
        ui.painter().text(r.center(), Align2::CENTER_CENTER, "\u{E740}", theme::icon(11.0), if b.hovered() { p.accent_deep } else { p.muted });
        if b.clicked() { self.settings.compact = false; self.settings.save(); }
    }

    /// One spell: its icon with a cooldown pie, plus the time (beside it in full view, on it in compact view).
    #[allow(clippy::too_many_arguments)]
    fn tile(&mut self, ui: &mut egui::Ui, i: usize, slot: Slot, r: Rect, now: f64, gctx: crate::cooldown::GameContext, mini: bool) {
        let p = self.pal;
        let e = &self.tracker.enemies[i];
        let preview = self.tracker.preview(i, slot, gctx, &self.data, &self.settings.rules);
        let spell = self.data.find_spell(e.spell_id(slot));
        let spell_name = spell.map(|s| s.name.clone()).unwrap_or_default();
        let cd = e.cooldowns.get(&slot).copied();
        let remaining = self.tracker.remaining(i, slot, now);
        let on_cd = remaining > 0.0 && cd.is_some();
        let frac = cd.filter(|c| c.duration() > 0.0).map(|c| (remaining / c.duration()) as f32).unwrap_or(0.0);
        let tip = match &preview {
            Some(b) => format!("{}: {:.1}s  (base {:.0}s, haste {:.0}){}\nLeft click: used now\nRight click: clear\nWheel: ±5s",
                b.ability_name, b.cooldown, b.base, b.haste, if b.detail.is_empty() { String::new() } else { format!("\n{}", b.detail) }),
            None => "no cooldown data".into(),
        };
        let resp = ui.interact(r, ui.id().with(("tile", i, slot as u8)), Sense::click()).on_hover_text(tip);
        let painter = ui.painter();
        let tint = if preview.is_some() { Color32::WHITE } else { Color32::from_gray(90) };
        let s = remaining.ceil() as i64;
        let time = format!("{}:{:02}", s / 60, s % 60);

        let icon_rect = if mini { r } else {
            painter.rect(r, 8.0_f32, p.bg, Stroke::new(1.0_f32, if resp.hovered() { p.accent } else if on_cd { p.cooling_line } else { p.line }));
            Rect::from_min_size(r.min + egui::vec2(5.0, 5.0), egui::vec2(r.height() - 10.0, r.height() - 10.0))
        };
        match super::icons::spell_texture(ui.ctx(), &spell_name) {
            Some(tex) => super::icons::draw_icon(painter, tex, icon_rect, 5.0, tint),
            None => {
                painter.rect_filled(icon_rect, 5.0_f32, p.surface_hover);
                let a = self.settings.message.abbreviate(&spell_name).to_uppercase();
                painter.text(icon_rect.center(), Align2::CENTER_CENTER, if a.is_empty() { "?".into() } else { a.chars().take(2).collect::<String>() }, theme::semi(10.5), p.muted);
            }
        }
        if on_cd {
            super::icons::draw_pie(painter, icon_rect, frac, Color32::from_black_alpha(165), p.accent);
        }
        if let Some(until) = self.alerts.get(&(e.champion_id.clone(), slot)) {
            // Flash just came back: a soft pulsing ring for a few seconds
            let left = until.saturating_duration_since(Instant::now()).as_secs_f32();
            let wave = (left * std::f32::consts::TAU * 1.5).sin() * 0.5 + 0.5;
            let ring = p.ready.gamma_multiply(0.35 + 0.65 * wave);
            painter.rect_stroke(icon_rect.expand(1.5 + 1.5 * wave), 6.0_f32, Stroke::new(2.0_f32 + wave, ring));
        }
        if mini {
            let border = if resp.hovered() { p.accent } else if on_cd { p.cooling_line } else { p.ready.gamma_multiply(0.55) };
            painter.rect_stroke(icon_rect, 5.0_f32, Stroke::new(1.2_f32, border));
            if on_cd {
                let label = if s >= 100 { format!("{}", s) } else { time.clone() };
                let label = if s >= 600 { format!("{}m", s / 60) } else { label };
                super::icons::outlined_text(painter, icon_rect.center(), &label, theme::semi(if label.len() > 3 { 10.0 } else { 11.0 }), Color32::WHITE);
            }
        } else {
            painter.rect_stroke(icon_rect, 5.0_f32, Stroke::new(1.0_f32, Color32::from_black_alpha(120)));
            let tx = r.right() - 9.0;
            if on_cd {
                let ty = if self.clock.is_live { r.center().y - 6.0 } else { r.center().y };
                painter.text(egui::pos2(tx, ty), Align2::RIGHT_CENTER, &time, theme::semi(13.5), p.cooling);
                if self.clock.is_live {
                    let at = cd.unwrap().ready_at.round() as i64;
                    painter.text(egui::pos2(tx, r.center().y + 9.0), Align2::RIGHT_CENTER, format!("@{}:{:02}", at / 60, at % 60), theme::regular(9.5), p.muted);
                }
            } else {
                let (t, col) = if preview.is_some() { ("Ready", p.ready) } else { ("n/a", p.muted) };
                painter.text(egui::pos2(tx, r.center().y), Align2::RIGHT_CENTER, t, theme::semi(13.0), col);
            }
        }
        if resp.clicked() { self.register(i, slot); }
        if resp.secondary_clicked() { self.tracker.clear(i, slot); }
        if resp.hovered() {
            let d = ui.input(|inp| inp.raw_scroll_delta.y);
            if d.abs() > 0.5 { self.tracker.adjust(i, slot, if d > 0.0 { 5.0 } else { -5.0 }); }
        }
    }

    fn tune(&mut self, ui: &mut egui::Ui, i: usize) {
        let p = self.pal;
        let now = self.clock.now();
        egui::Frame::none().fill(p.inset).stroke(Stroke::new(1.0_f32, p.line)).rounding(7.0_f32).inner_margin(egui::Margin::symmetric(10.0, 7.0)).show(ui, |ui| {
            ui.set_width(CARD_W - 44.0);
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
            let mut changed = false;
            ui.horizontal(|ui| {
                let e = &mut self.tracker.enemies[i];
                ui.label(egui::RichText::new("Level").color(p.muted).size(11.0)).on_hover_text("Unleashed Teleport's cooldown depends on level");
                if small_btn(ui, "−") { e.level = (e.level - 1).max(1); changed = true; }
                ui.label(egui::RichText::new(format!("{}", e.level)).font(theme::semi(11.5)));
                if small_btn(ui, "+") { e.level = (e.level + 1).min(18); changed = true; }
                ui.add_space(10.0);
                changed |= ui.checkbox(&mut e.cosmic_insight, egui::RichText::new("Cosmic Insight").size(11.0)).on_hover_text("+18 summoner spell haste (rune)").changed();
            });
            let (data, rules) = (self.data.clone(), self.settings.rules.clone());
            if changed { self.tracker.recalculate(i, now, self.clock.is_live, &data, &rules); }
            let gctx = self.tracker.context(now, self.clock.is_live);
            let parts: Vec<String> = SLOTS.iter().filter_map(|s| self.tracker.preview(i, *s, gctx, &data, &rules)).map(|b| format!("{} {:.0}s", b.ability_name, b.cooldown)).collect();
            ui.label(egui::RichText::new(parts.join("  ·  ")).color(p.muted).size(10.5));
        });
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let msg = self.message();
        ui.add_space(4.0);
        let resp = egui::Frame::none().fill(p.inset).stroke(Stroke::new(1.0_f32, p.line)).rounding(8.0_f32)
            .inner_margin(egui::Margin { left: 13.0, right: 10.0, top: 6.0, bottom: 7.0 }).show(ui, |ui| {
                ui.set_width(CARD_W - 24.0 - 23.0);
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
                ui.label(egui::RichText::new("CHAT MESSAGE").font(theme::semi(9.5)).color(p.accent_deep));
                let text = if msg.is_empty() { "No cooldowns tracked".to_string() } else { msg };
                ui.add(egui::Label::new(egui::RichText::new(text).font(theme::mono(12.0)).color(p.text)).wrap());
            });
        let r = resp.response.rect;
        ui.painter().rect_filled(Rect::from_min_size(r.min, egui::vec2(3.0, r.height())), Rounding { nw: 8.0, sw: 8.0, ne: 0.0, se: 0.0 }, p.accent);
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 0.0);
            let copy = egui::Button::new(egui::RichText::new("Copy").font(theme::semi(12.0)).color(p.accent_text))
                .fill(p.accent).stroke(Stroke::NONE).rounding(6.0_f32).min_size(egui::vec2(72.0, 26.0));
            if ui.add(copy).on_hover_text("Copy the message to the clipboard").clicked() { self.copy_now(); }
            let mut auto = self.settings.auto_copy;
            if ui.checkbox(&mut auto, egui::RichText::new("Auto-copy").size(11.5)).on_hover_text("Copy the message whenever a cooldown is registered").changed() {
                self.settings.auto_copy = auto;
                self.settings.save();
            }
            let mut clock = self.settings.message.format == TimeFormat::GameClock;
            if ui.checkbox(&mut clock, egui::RichText::new("Clock").size(11.5)).on_hover_text("On: game time the ability is back up (2230 = 22:30). Off: seconds remaining.").changed() {
                self.settings.message.format = if clock { TimeFormat::GameClock } else { TimeFormat::SecondsRemaining };
                self.settings.save();
            }
            if let Some(r) = &self.update {
                let (v, page) = (r.version.clone(), r.page.clone());
                let label = if self.updating.is_some() { "Updating...".to_string() } else { format!("Update to {v}") };
                let link = ui.link(egui::RichText::new(label).size(11.5).color(p.accent_deep))
                    .on_hover_text("Download and install it now (Flashless restarts, settings are kept).\nRight click: see what's new");
                if link.clicked() { let ctx = ui.ctx().clone(); self.start_update(&ctx); }
                if link.secondary_clicked() { win::open_url(&page); }
            }
        });
    }
}

fn mini_width() -> f32 { 8.0 + MINI_ROLE_W + 2.0 * MINI_ICON + MINI_GAP + 8.0 + 2.0 }

fn small_btn(ui: &mut egui::Ui, t: &str) -> bool {
    ui.add(egui::Button::new(egui::RichText::new(t).size(12.0)).min_size(egui::vec2(22.0, 20.0)).rounding(5.0_f32)).clicked()
}
