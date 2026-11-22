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
use crate::tracker::{Enemy, GameClock, Slot, Tracker};
use crate::win::{self, clipboard, hotkeys, typing, watcher};
use eframe::egui::{self, Align2, Color32, Rect, Rounding, Sense, Stroke, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const MARGIN: f32 = 14.0;
pub const CARD_W: f32 = 372.0;
pub const TILE: Vec2 = Vec2::new(104.0, 40.0);
pub const TILE_GAP: f32 = 4.0;
/// The quest Teleport tile in full view: a square (icon with the time on it), on the top laner's row only.
const QUEST_W: f32 = TILE.y;
/// Champion portrait in full view, in place of the number badge (the card grows by the difference).
pub const PORTRAIT: f32 = 28.0;
const BADGE_W: f32 = 26.0;
const PORTRAIT_W: f32 = PORTRAIT + 10.0;
/// Compact view.
pub const MINI_ICON: f32 = 30.0;
pub const MINI_GAP: f32 = 4.0;
pub const MINI_ROLE_W: f32 = 32.0;
pub const MINI_PORTRAIT: f32 = 24.0;

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
    riot: Option<Receiver<Result<Vec<crate::riot::RunePage>, String>>>,
    /// Next rune lookup attempt for this game (when a Riot API key is saved), and attempts so far.
    riot_retry: Option<(Instant, u32)>,
    riot_retry_count: u32,
    /// Asked for from Settings.
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
            riot_lookup_now: false,
            riot: None,
            riot_retry: None,
            riot_retry_count: 0,
        };
        let failed = hotkeys::set_bindings(&app.settings.hotkeys);
        if !failed.is_empty() { app.flash(format!("bad or duplicate hotkey: {}", failed.join(", "))); }
        if demo { app.load_demo(); }
        app.seen_revision = app.tracker.cooldown_revision;
        app
    }

    fn rules(&self) -> &CooldownRules { &self.settings.rules }

    /// With a saved Riot API key: looks up the enemies' rune pages once per game, retrying while the game is not listed yet.
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
            mk("Syndra", "SummonerTeleport", 12, &[], "Mid", d), mk("LeeSin", "SummonerSmite", 10, &[], "Jg", d), mk("Shen", "SummonerDot", 11, &[3171], "Top", d),
        ];
        roster[2].has_inspiration = true;
        crate::tracker::sort_by_role(&mut roster); // Top first, as a live game lists them
        self.tracker.set_roster(roster);
        let at = |t: &Tracker, id: &str| t.enemies.iter().position(|e| e.champion_id == id).unwrap_or(0);
        let now = self.clock.now();
        let (data, rules) = (self.data.clone(), self.rules().clone());
        for (id, slot, ago) in [("Lucian", Slot::Spell1, 0.0), ("Lucian", Slot::Spell2, 40.0), ("Nautilus", Slot::Spell1, 120.0), ("Shen", Slot::Spell2, 200.0), ("Shen", Slot::Quest, 90.0)] {
            let i = at(&self.tracker, id);
            self.tracker.register(i, slot, ago, now, false, &data, &rules);
        }
    }

    /// Left and right tile for enemy `i`, per the spell order setting.
    fn display_slots(&self, i: usize) -> Vec<Slot> {
        let e = &self.tracker.enemies[i];
        let is_flash = |slot| self.data.find_spell(e.spell_id(slot)).is_some_and(|s| s.name == "Flash");
        let mut v = self.settings.spell_order.slots(is_flash(Slot::Spell1), is_flash(Slot::Spell2)).to_vec();
        if e.has_quest_teleport() { v.push(Slot::Quest); }
        v
    }

    /// Someone has a quest Teleport: the overlay gets a third (icon) column.
    fn quest_column(&self) -> bool { self.tracker.enemies.iter().any(|e| e.has_quest_teleport()) }

    /// Champion portraits on: the saved setting, or the Settings window's unsaved choice while it is open (Cancel restores it).
    fn portraits(&self) -> bool { self.setup.as_ref().map_or(self.settings.show_portraits, |s| s.draft_portraits()) }

    fn card_w(&self) -> f32 { card_width(self.portraits()) }

    fn mini_w(&self) -> f32 { mini_width(self.portraits()) + if self.quest_column() { MINI_ICON + MINI_GAP } else { 0.0 } }

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
            "TopTeleport" => {
                if let Some(i) = self.tracker.enemies.iter().position(|e| e.has_quest_teleport()) { self.register(i, Slot::Quest); }
                else { self.flash("no top laner with a quest Teleport"); }
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
                    let boots_before: Vec<(String, i32)> = self.tracker.enemies.iter().map(|e| (e.champion_id.clone(), boots_haste(e, &self.data).0)).collect();
                    if self.tracker.apply_live(&s, &self.data, &self.settings.rules) {
                        self.filter.reset();
                        if !self.tracker.enemies.iter().any(|e| before.contains(&e.champion_id)) {
                            self.rotate_chat_log();
                            self.riot_retry = Some((Instant::now(), 0));
                        }
                        let names: Vec<&str> = self.tracker.enemies.iter().map(|e| e.champion_id.as_str()).collect();
                        self.chat_log(&format!("ROSTER {}", names.join(", ")));
                    }
                    // boots bought or sold: log it, so the live item data can be checked against the scoreboard
                    let changes: Vec<String> = self.tracker.enemies.iter().filter_map(|e| {
                        let was = boots_before.iter().find(|(c, _)| *c == e.champion_id).map(|b| b.1)?;
                        let (now_h, names) = boots_haste(e, &self.data);
                        (now_h != was).then(|| if now_h > 0 { format!("ITEMS {}: {names} (+{now_h} summoner haste)", e.name) } else { format!("ITEMS {}: no Lucidity boots", e.name) })
                    }).collect();
                    for c in changes { self.chat_log(&c); }
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

        self.pump_riot(ctx);
        // shown as the Update button in the footer, not as a pop-up at the top
        if let Ok(r) = self.update_rx.try_recv() { self.update = Some(r); }
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

    fn flash_active(&self) -> bool { self.flash.as_ref().is_some_and(|(_, until)| Instant::now() < *until) }

    fn status_text(&self) -> String {
        if let Some((t, until)) = &self.flash { if Instant::now() < *until { return t.clone(); } }
        let t = self.clock.now() as i64;
        if self.clock.is_live { format!("In game {}:{:02}", t / 60, t % 60) } else { "No game".into() }
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
                    let h = windows::Win32::Foundation::HWND(w.hwnd.get() as _);
                    win::make_no_activate(h);
                    win::set_icon_from_resource(h);
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
            let card_w = if compact { self.mini_w() } else { self.card_w() };
            let card = egui::Frame::none().fill(p.surface).stroke(Stroke::new(1.0_f32, p.line_strong)).rounding(Rounding::same(if compact { 9.0 } else { 10.0 }))
                .shadow(egui::epaint::Shadow { offset: egui::vec2(0.0, 3.0), blur: 14.0, spread: 0.0, color: p.shadow });
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
                let (r, _) = ui.allocate_exact_size(egui::vec2(card_w, 1.0), Sense::hover());
                ui.painter().rect_filled(r, 0.0_f32, p.line);
                egui::Frame::none().inner_margin(egui::Margin { left: 12.0, right: 12.0, top: 10.0, bottom: 10.0 }).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                    self.rows(ui);
                    if self.tracker.enemies.is_empty() { self.empty_state(ui); }
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
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(self.card_w(), 38.0), Sense::click_and_drag());
        if resp.drag_started() { ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); }
        let painter = ui.painter_at(rect);
        let cy = rect.center().y;
        theme::brand(&painter, egui::pos2(rect.left() + 21.0, cy), 8.5, &p);
        let name = painter.text(egui::pos2(rect.left() + 36.0, cy), Align2::LEFT_CENTER, "Flashless", theme::semi(13.5), p.text);
        let ver = painter.text(egui::pos2(name.right() + 6.0, cy + 1.0), Align2::LEFT_CENTER, concat!("v", env!("CARGO_PKG_VERSION")), theme::regular(10.5), p.muted);
        // game status: a dot and a word, or the latest notice in its place
        let live = self.clock.is_live;
        let dot = egui::pos2(ver.right() + 14.0, cy + 0.5);
        painter.circle_filled(dot, 3.0, if live { p.ready } else { p.muted });
        let col = if self.flash_active() { p.secondary } else if live { p.text } else { p.muted };
        // icon buttons, right aligned
        let icons = [("\u{E7A7}", "Undo the last change"), ("\u{E72C}", "Clear all tracked cooldowns"), ("\u{E73F}", "Compact mode"), ("\u{E713}", "Settings"), ("\u{E8BB}", "Exit")];
        let mut x = rect.right() - 8.0 - 26.0 * icons.len() as f32;
        // long notices are cut short before the buttons (the full text is in the tooltip)
        let status = self.status_text();
        let mut job = egui::text::LayoutJob::simple_singleline(status.clone(), theme::regular(11.0), col);
        job.wrap = egui::text::TextWrapping::truncate_at_width((x - dot.x - 14.0).max(20.0));
        let g = painter.layout_job(job);
        let gr = Rect::from_min_size(egui::pos2(dot.x + 8.0, cy - g.size().y / 2.0), g.size());
        painter.galley(gr.min, g, col);
        if self.flash_active() { ui.interact(gr, ui.id().with("status"), Sense::hover()).on_hover_text(status); }
        for (i, (glyph, tip)) in icons.iter().enumerate() {
            let r = Rect::from_min_size(egui::pos2(x, cy - 12.0), egui::vec2(26.0, 24.0));
            x += 26.0;
            let b = ui.interact(r, ui.id().with(("hdr", i)), Sense::click()).on_hover_text(*tip);
            if b.hovered() { ui.painter().rect_filled(r, 5.0_f32, p.surface_hover); }
            ui.painter().text(r.center(), Align2::CENTER_CENTER, *glyph, theme::icon(if i == 4 { 10.0 } else { 12.0 }), if b.hovered() { p.text } else { p.secondary });
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

    /// Nothing to track yet.
    fn empty_state(&self, ui: &mut egui::Ui) {
        let p = self.pal;
        let (title, body) = if self.clock.is_live { ("Game detected", "Loading the enemy champions...") }
            else { ("No game detected", "Waiting for a match. Enemy champions will appear automatically.") };
        ui.add_space(2.0);
        ui.label(egui::RichText::new(title).font(theme::semi(13.0)).color(p.text));
        ui.add_space(-3.0);
        ui.add(egui::Label::new(egui::RichText::new(body).size(11.5).color(p.secondary)).wrap())
            .on_hover_text("To track without a game, enter the enemies under Settings > Enemy Roster.");
        ui.add_space(4.0);
    }

    fn rows(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let now = self.clock.now();
        let gctx = self.tracker.context(now, self.clock.is_live);
        let portraits = self.portraits();
        let row_w = self.card_w() - 24.0;
        for i in 0..self.tracker.enemies.len() {
            let row = ui.allocate_exact_size(egui::vec2(row_w, TILE.y), Sense::hover()).0;
            // name block (click to tune); a quest Teleport takes a little of it on that row only
            let quest = if self.tracker.enemies[i].has_quest_teleport() { QUEST_W + TILE_GAP } else { 0.0 };
            let name_w = row_w - 2.0 * (TILE.x + TILE_GAP) - quest;
            let name_rect = Rect::from_min_size(row.min, egui::vec2(name_w, TILE.y));
            let (boots, boots_names) = boots_haste(&self.tracker.enemies[i], &self.data);
            let tip = if boots > 0 { format!("{boots_names}: +{boots} summoner spell haste\nClick to set Cosmic Insight and level") } else { "Click to set Cosmic Insight and level".to_string() };
            let nresp = ui.interact(name_rect, ui.id().with(("name", i)), Sense::click()).on_hover_text(tip);
            if nresp.clicked() { self.expanded = if self.expanded == Some(i) { None } else { Some(i) }; }
            let e = &self.tracker.enemies[i];
            // second line: lane, boots haste, Cosmic Insight hint
            let mut bits: Vec<(String, Color32)> = Vec::new();
            if let Some(role) = &e.role { bits.push((role.clone(), p.accent_deep)); }
            if boots > 0 { bits.push((format!("+{boots}"), p.ready)); }
            if e.has_inspiration && !e.cosmic_insight && !e.runes_known { bits.push(("CI?".into(), p.accent)); }
            if e.cosmic_insight { bits.push(("CI".into(), p.ready)); }
            let look = NameLook { index: i, champion_id: &e.champion_id, name: &e.name, bits, hovered: nresp.hovered() || self.expanded == Some(i) };
            paint_name(ui.painter(), &p, name_rect, &look, portraits);
            for (k, slot) in self.display_slots(i).iter().enumerate() {
                let r = if *slot == Slot::Quest {
                    Rect::from_min_size(egui::pos2(row.right() - 2.0 * (TILE.x + TILE_GAP) - QUEST_W, row.top()), egui::vec2(QUEST_W, TILE.y))
                } else {
                    Rect::from_min_size(egui::pos2(row.right() - (2 - k) as f32 * (TILE.x + TILE_GAP) + TILE_GAP, row.top()), TILE)
                };
                self.tile(ui, i, *slot, r, now, gctx, false);
            }
            if self.expanded == Some(i) { let w = row_w; self.tune(ui, i, w); }
        }
    }

    /// Compact mode: portrait (optional), lane label and the spell icons per enemy, time drawn on the icon.
    fn mini_rows(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let now = self.clock.now();
        let gctx = self.tracker.context(now, self.clock.is_live);
        if self.tracker.enemies.is_empty() {
            let t = if self.clock.is_live { "Loading enemies" } else { "No game detected" };
            ui.add(egui::Label::new(egui::RichText::new(t).size(11.0).color(p.secondary)));
            return;
        }
        let portraits = self.portraits();
        for i in 0..self.tracker.enemies.len() {
            let row = ui.allocate_exact_size(egui::vec2(self.mini_w() - 16.0, MINI_ICON), Sense::hover()).0;
            let e = &self.tracker.enemies[i];
            let x0 = paint_mini_label(ui.painter(), &p, row, &e.champion_id, &e.name, e.role.as_deref(), portraits);
            for (k, slot) in self.display_slots(i).iter().enumerate() {
                let r = Rect::from_min_size(egui::pos2(x0 + k as f32 * (MINI_ICON + MINI_GAP), row.top()), egui::vec2(MINI_ICON, MINI_ICON));
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
            ui.painter().text(ur.center(), Align2::CENTER_CENTER, "\u{E896}", theme::icon(10.0), if ub.hovered() { p.accent_hover } else { p.accent });
            if ub.clicked() { let ctx = ctx.clone(); self.start_update(&ctx); }
        }
        let r = Rect::from_min_size(egui::pos2(rect.right() - 30.0, cy - 11.0), egui::vec2(24.0, 22.0));
        let b = ui.interact(r, ui.id().with("expand"), Sense::click()).on_hover_text("Full view");
        if b.hovered() { ui.painter().rect_filled(r, 5.0_f32, p.surface_hover); }
        ui.painter().text(r.center(), Align2::CENTER_CENTER, "\u{E740}", theme::icon(11.0), if b.hovered() { p.text } else { p.secondary });
        if b.clicked() { self.settings.compact = false; self.settings.save(); }
    }

    /// One spell: its icon with a cooldown pie, plus the time (beside it in full view, on it in compact view).
    #[allow(clippy::too_many_arguments)]
    fn tile(&mut self, ui: &mut egui::Ui, i: usize, slot: Slot, r: Rect, now: f64, gctx: crate::cooldown::GameContext, mini: bool) {
        let e = &self.tracker.enemies[i];
        let preview = self.tracker.preview(i, slot, gctx, &self.data, &self.settings.rules);
        let spell_name = self.data.find_spell(e.spell_id(slot)).map(|s| s.name.clone()).unwrap_or_default();
        let cd = e.cooldowns.get(&slot).copied();
        let remaining = if cd.is_some() { self.tracker.remaining(i, slot, now) } else { 0.0 };
        let frac = cd.filter(|c| c.duration() > 0.0).map(|c| (remaining / c.duration()) as f32).unwrap_or(0.0);
        // the game time it is back; shown once there is a game clock (or in the demo)
        let ready_at = cd.filter(|_| remaining > 0.0 && (self.clock.is_live || self.clock.seen_live || self.demo)).map(|c| c.ready_at);
        let back = ready_at.map(|t| { let t = t.round() as i64; format!("\nBack up at {}:{:02}", t / 60, t % 60) }).unwrap_or_default();
        let tip = match &preview {
            Some(b) => format!("{}: {:.1}s  (base {:.0}s, haste {:.0}){}{back}\nLeft click: used now\nRight click: clear\nWheel: ±5s",
                b.ability_name, b.cooldown, b.base, b.haste, if b.detail.is_empty() { String::new() } else { format!("\n{}", b.detail) }),
            None => "no cooldown data".into(),
        };
        let resp = ui.interact(r, ui.id().with(("tile", i, slot as u8)), Sense::click()).on_hover_text(tip);
        let pulse = self.alerts.get(&(e.champion_id.clone(), slot)).map(|until| {
            let left = until.saturating_duration_since(Instant::now()).as_secs_f32();
            (left * std::f32::consts::TAU * 1.5).sin() * 0.5 + 0.5
        });
        let look = TileLook {
            spell: &spell_name,
            abbr: self.settings.message.abbreviate(&spell_name),
            has_data: preview.is_some(),
            remaining,
            frac,
            ready_at,
            hovered: resp.hovered(),
            pulse,
        };
        paint_tile(ui.painter(), &self.pal, r, &look, mini);
        if resp.clicked() { self.register(i, slot); }
        if resp.secondary_clicked() { self.tracker.clear(i, slot); }
        if resp.hovered() {
            let d = ui.input(|inp| inp.raw_scroll_delta.y);
            if d.abs() > 0.5 { self.tracker.adjust(i, slot, if d > 0.0 { 5.0 } else { -5.0 }); }
        }
    }

    fn tune(&mut self, ui: &mut egui::Ui, i: usize, w: f32) {
        let p = self.pal;
        let now = self.clock.now();
        egui::Frame::none().fill(p.inset).stroke(Stroke::new(1.0_f32, p.line)).rounding(7.0_f32).inner_margin(egui::Margin::symmetric(10.0, 7.0)).show(ui, |ui| {
            ui.set_width(w - 22.0);
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
            let mut changed = false;
            ui.horizontal(|ui| {
                let e = &mut self.tracker.enemies[i];
                ui.label(egui::RichText::new("Level").color(p.secondary).size(11.0)).on_hover_text("Unleashed Teleport's cooldown depends on level");
                if small_btn(ui, "−") { e.level = (e.level - 1).max(1); changed = true; }
                ui.label(egui::RichText::new(format!("{}", e.level)).font(theme::semi(11.5)));
                if small_btn(ui, "+") { e.level = (e.level + 1).min(18); changed = true; }
                ui.add_space(10.0);
                changed |= theme::check(ui, &mut e.cosmic_insight, egui::RichText::new("Cosmic Insight").size(11.0)).on_hover_text("+18 summoner spell haste (rune)").changed();
            });
            let (data, rules) = (self.data.clone(), self.settings.rules.clone());
            if changed { self.tracker.recalculate(i, now, self.clock.is_live, &data, &rules); }
            let gctx = self.tracker.context(now, self.clock.is_live);
            let parts: Vec<String> = self.display_slots(i).iter().filter_map(|s| self.tracker.preview(i, *s, gctx, &data, &rules)).map(|b| format!("{} {:.0}s", b.ability_name, b.cooldown)).collect();
            ui.label(egui::RichText::new(parts.join("  ·  ")).color(p.secondary).size(10.5));
        });
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let msg = self.message();
        ui.add_space(2.0);
        chat_preview(ui, &self.pal, self.card_w() - 24.0, &msg);
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(12.0, 0.0);
            if gold_button(ui, &p, "Copy", 68.0).on_hover_text("Copy the message to the clipboard").clicked() { self.copy_now(); }
            let mut auto = self.settings.auto_copy;
            if theme::check(ui, &mut auto, egui::RichText::new("Auto-copy").size(11.5)).on_hover_text("Copy the message whenever a cooldown is registered").changed() {
                self.settings.auto_copy = auto;
                self.settings.save();
            }
            let mut clock = self.settings.message.format == TimeFormat::GameClock;
            if theme::check(ui, &mut clock, egui::RichText::new("Clock").size(11.5)).on_hover_text("On: game time the ability is back up (2230 = 22:30). Off: seconds remaining.").changed() {
                self.settings.message.format = if clock { TimeFormat::GameClock } else { TimeFormat::SecondsRemaining };
                self.settings.save();
            }
            if let Some(r) = &self.update {
                let (v, page) = (r.version.clone(), r.page.clone());
                let label = if self.updating.is_some() { "Updating...".to_string() } else { format!("Update {v}") };
                // same look as Copy, pinned to the right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let b = gold_button(ui, &p, &label, 68.0).on_hover_text("Download and install it now (Flashless restarts, settings are kept).\nRight click: see what's new");
                    if b.clicked() { let ctx = ui.ctx().clone(); self.start_update(&ctx); }
                    if b.secondary_clicked() { win::open_url(&page); }
                });
            }
        });
    }
}

// ---------- shared painters (the overlay and the Settings preview) ----------

/// Overlay card width in full view.
pub fn card_width(portraits: bool) -> f32 {
    CARD_W + if portraits { PORTRAIT_W - BADGE_W } else { 0.0 }
}

/// What a spell tile shows. The overlay fills it from the tracker; the Settings preview from sample data.
pub struct TileLook<'a> {
    pub spell: &'a str,
    /// Chat abbreviation ("f", "ig"), the stand-in when there is no icon.
    pub abbr: String,
    /// Cooldown data is known for it.
    pub has_data: bool,
    /// Seconds until it is back (0 = up).
    pub remaining: f64,
    /// Share of the cooldown still to go, for the pie.
    pub frac: f32,
    /// Game time it is back, shown under the countdown while a game is live.
    pub ready_at: Option<f64>,
    pub hovered: bool,
    /// Flash just came back: pulse phase 0..1.
    pub pulse: Option<f32>,
}

/// "ig" -> "Ig", at most three letters.
fn short_abbr(a: &str) -> String {
    let mut c = a.chars().take(3);
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The spell icon with its cooldown pie and edge.
fn paint_slot_icon(painter: &egui::Painter, p: &Pal, r: Rect, t: &TileLook, mini: bool) {
    let on_cd = t.remaining > 0.0;
    match super::icons::spell_texture(painter.ctx(), t.spell) {
        Some(tex) => super::icons::draw_icon(painter, tex, r, 5.0, if t.has_data { Color32::WHITE } else { Color32::from_gray(90) }),
        None => {
            painter.rect_filled(r, 5.0_f32, p.elevated);
            let a = short_abbr(&t.abbr);
            painter.text(r.center(), Align2::CENTER_CENTER, if a.is_empty() { "?".into() } else { a }, theme::semi(10.5), p.muted);
        }
    }
    if on_cd { super::icons::draw_pie(painter, r, t.frac, Color32::from_black_alpha(170), p.accent); }
    if let Some(wave) = t.pulse {
        // Flash just came back: a soft pulsing ring for a few seconds
        painter.rect_stroke(r.expand(1.5 + 1.5 * wave), 6.0_f32, Stroke::new(2.0_f32 + wave, p.ready.gamma_multiply(0.35 + 0.65 * wave)));
    }
    let edge = if mini {
        Stroke::new(1.2_f32, if t.hovered { p.accent } else if on_cd { p.cooling_line } else { p.ready.gamma_multiply(0.55) })
    } else {
        Stroke::new(1.0_f32, Color32::from_black_alpha(120))
    };
    painter.rect_stroke(r, 5.0_f32, edge);
}

/// One spell: a box with the icon and its time (full view), or the icon with the time on it (compact view).
pub fn paint_tile(painter: &egui::Painter, p: &Pal, r: Rect, t: &TileLook, mini: bool) {
    let on_cd = t.remaining > 0.0;
    let s = t.remaining.ceil() as i64;
    if mini {
        paint_slot_icon(painter, p, r, t, true);
        if on_cd {
            let label = if s >= 600 { format!("{}m", s / 60) } else if s >= 100 { format!("{s}") } else { format!("{}:{:02}", s / 60, s % 60) };
            super::icons::outlined_text(painter, r.center(), &label, theme::semi(if label.len() > 3 { 10.0 } else { 11.0 }), Color32::WHITE);
        }
        return;
    }
    let (fill, edge) = if t.hovered { (p.surface_hover, p.accent) } else if on_cd { (p.elevated, p.cooling_line) } else { (p.inset, p.line) };
    painter.rect(r, 8.0_f32, fill, Stroke::new(1.0_f32, edge));
    let icon = Rect::from_min_size(r.min + egui::vec2(5.0, 5.0), egui::vec2(r.height() - 10.0, r.height() - 10.0));
    if r.width() <= r.height() + 1.0 {
        // square tile (quest Teleport): the time goes on the icon, the game time it is back in the tooltip
        paint_slot_icon(painter, p, icon, t, false);
        if on_cd {
            let label = if s >= 600 { format!("{}m", s / 60) } else if s >= 100 { format!("{s}") } else { format!("{}:{:02}", s / 60, s % 60) };
            super::icons::outlined_text(painter, icon.center(), &label, theme::semi(if label.len() > 3 { 9.5 } else { 10.5 }), Color32::WHITE);
        }
        return;
    }
    paint_slot_icon(painter, p, icon, t, false);
    let tx = r.right() - 9.0;
    if on_cd {
        let time = format!("{}:{:02}", s / 60, s % 60);
        match t.ready_at {
            Some(at) => {
                painter.text(egui::pos2(tx, r.center().y - 6.0), Align2::RIGHT_CENTER, &time, theme::semi(13.5), p.cooling);
                let at = at.round() as i64;
                painter.text(egui::pos2(tx, r.center().y + 8.5), Align2::RIGHT_CENTER, format!("@{}:{:02}", at / 60, at % 60), theme::regular(9.5), p.muted);
            }
            None => { painter.text(egui::pos2(tx, r.center().y), Align2::RIGHT_CENTER, &time, theme::semi(13.5), p.cooling); }
        }
    } else {
        let (text, col) = if t.has_data { ("Ready", p.ready) } else { ("n/a", p.muted) };
        painter.text(egui::pos2(tx, r.center().y), Align2::RIGHT_CENTER, text, theme::semi(13.0), col);
    }
}

/// Name block of a full-view row.
pub struct NameLook<'a> {
    pub index: usize,
    pub champion_id: &'a str,
    pub name: &'a str,
    /// Second line: lane, boots haste, Cosmic Insight.
    pub bits: Vec<(String, Color32)>,
    pub hovered: bool,
}

/// Number badge (or the champion portrait in its place), the name, and the lane line under it.
pub fn paint_name(painter: &egui::Painter, p: &Pal, r: Rect, n: &NameLook, portraits: bool) {
    let cy = r.center().y;
    let nx = if portraits {
        paint_portrait(painter, p, Rect::from_min_size(egui::pos2(r.left(), cy - PORTRAIT / 2.0), egui::vec2(PORTRAIT, PORTRAIT)), n.champion_id, n.name);
        r.left() + PORTRAIT_W
    } else {
        let c = egui::pos2(r.left() + 9.0, cy);
        painter.circle_filled(c, 9.0, p.accent_soft);
        painter.text(c, Align2::CENTER_CENTER, format!("{}", n.index + 1), theme::semi(10.0), p.accent_deep);
        r.left() + BADGE_W
    };
    let clip = painter.with_clip_rect(Rect::from_min_max(egui::pos2(nx, r.top()), egui::pos2(r.right() - 2.0, r.bottom())).intersect(painter.clip_rect()));
    let line1 = if n.bits.is_empty() { cy } else { cy - 7.0 };
    clip.text(egui::pos2(nx, line1), Align2::LEFT_CENTER, n.name, theme::semi(13.0), if n.hovered { p.accent_deep } else { p.text });
    let mut x = nx;
    for (k, (t, col)) in n.bits.iter().enumerate() {
        if k > 0 { x = clip.text(egui::pos2(x, cy + 9.0), Align2::LEFT_CENTER, "  ·  ", theme::regular(10.0), p.muted).right(); }
        x = clip.text(egui::pos2(x, cy + 9.0), Align2::LEFT_CENTER, t, theme::semi(10.0), *col).right();
    }
}

/// Square champion portrait with soft corners; initials if the champion is newer than this build.
pub fn paint_portrait(painter: &egui::Painter, p: &Pal, r: Rect, champion_id: &str, name: &str) {
    match super::icons::portrait_texture(painter.ctx(), champion_id) {
        Some(tex) => super::icons::draw_icon(painter, tex, r, 4.0, Color32::WHITE),
        None => {
            painter.rect_filled(r, 4.0_f32, p.elevated);
            let initials: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
            painter.text(r.center(), Align2::CENTER_CENTER, initials, theme::semi(r.height() * 0.4), p.secondary);
        }
    }
    painter.rect_stroke(r, 4.0_f32, Stroke::new(1.0_f32, p.line_strong));
}

/// Compact row: optional portrait and the lane label. Returns where the spell icons start.
pub fn paint_mini_label(painter: &egui::Painter, p: &Pal, row: Rect, champion_id: &str, name: &str, role: Option<&str>, portraits: bool) -> f32 {
    let mut x = row.left();
    if portraits {
        paint_portrait(painter, p, Rect::from_min_size(egui::pos2(x, row.center().y - MINI_PORTRAIT / 2.0), egui::vec2(MINI_PORTRAIT, MINI_PORTRAIT)), champion_id, name);
        x += MINI_PORTRAIT + 6.0;
    }
    let label = role.map(str::to_string).unwrap_or_else(|| name.chars().take(4).collect());
    painter.text(egui::pos2(x + 1.0, row.center().y), Align2::LEFT_CENTER, label, theme::semi(11.5), p.accent_deep);
    x + MINI_ROLE_W
}

/// The chat message as it will be pasted, on a recessed panel with a gold edge.
pub fn chat_preview(ui: &mut egui::Ui, p: &Pal, width: f32, msg: &str) {
    let resp = egui::Frame::none().fill(p.inset).stroke(Stroke::new(1.0_f32, p.line)).rounding(7.0_f32)
        .inner_margin(egui::Margin { left: 12.0, right: 10.0, top: 6.0, bottom: 7.0 }).show(ui, |ui| {
            ui.set_width(width - 22.0);
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
            ui.label(egui::RichText::new("CHAT MESSAGE").font(theme::semi(9.5)).color(p.accent_muted));
            let (text, col) = if msg.is_empty() { ("No cooldowns tracked", p.muted) } else { (msg, p.text) };
            ui.add(egui::Label::new(egui::RichText::new(text).font(theme::mono(12.0)).color(col)).wrap());
        });
    let r = resp.response.rect;
    ui.painter().rect_filled(Rect::from_min_size(r.min, egui::vec2(2.5, r.height())), Rounding { nw: 7.0, sw: 7.0, ne: 0.0, se: 0.0 }, p.accent);
}

/// Filled gold button (Copy, Update, Save): brighter on hover.
pub fn gold_button(ui: &mut egui::Ui, p: &Pal, text: &str, min_w: f32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::semi(12.0), p.accent_text);
    let size = egui::vec2((galley.size().x + 24.0).max(min_w), 26.0);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.is_pointer_button_down_on() { p.accent_muted } else if resp.hovered() { p.accent_hover } else { p.accent };
    ui.painter().rect_filled(r, 6.0_f32, fill);
    ui.painter().galley(r.center() - galley.size() / 2.0, galley, p.accent_text);
    resp
}

pub fn mini_width(portraits: bool) -> f32 {
    8.0 + if portraits { MINI_PORTRAIT + 6.0 } else { 0.0 } + MINI_ROLE_W + 2.0 * MINI_ICON + MINI_GAP + 8.0 + 2.0
}

fn small_btn(ui: &mut egui::Ui, t: &str) -> bool {
    ui.add(egui::Button::new(egui::RichText::new(t).size(12.0)).min_size(egui::vec2(22.0, 20.0)).rounding(5.0_f32)).clicked()
}

/// Summoner haste from an enemy's items (Ionian Boots of Lucidity +10, Crimson Lucidity +20) and their names.
fn boots_haste(e: &Enemy, data: &GameData) -> (i32, String) {
    let items: Vec<&crate::data::ItemInfo> = e.item_ids.iter().filter_map(|&id| data.find_item(id)).filter(|it| it.summoner_haste > 0).collect();
    (items.iter().map(|it| it.summoner_haste).sum(), items.iter().map(|it| it.name.as_str()).collect::<Vec<_>>().join(", "))
}
