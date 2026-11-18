//! Setup window (a second native window): roster, message, clipboard and typing, chat reading
//! (with draw-area picker and auto-detect), appearance and hotkeys.

use super::app::App;
use super::theme::{self, Pal};
use crate::message::TimeFormat;
use crate::settings::{Settings, DEFAULT_CHAT_REGION};
use crate::tracker::Enemy;
use crate::win::{self, hotkeys, watcher};
use eframe::egui::{self, Color32, Rect, RichText, Stroke, ViewportBuilder, ViewportId};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ROLES: [&str; 5] = ["Top", "Jg", "Mid", "ADC", "Sup"];

struct Row {
    champ: String,
    s1: Option<String>,
    s2: Option<String>,
    level: i32,
}

struct Picker {
    bounds: Rect,
    start: Option<egui::Pos2>,
    sel: Option<Rect>,
}

pub struct SetupState {
    draft: Settings,
    rows: Vec<Row>,
    roster_note: String,
    area_note: String,
    listening: Option<String>,
    detect_rx: Option<Receiver<Result<([f64; 4], usize), String>>>,
    countdown: Option<Instant>,
    picker: Option<Picker>,
    chrome_for: Option<bool>,
}

impl SetupState {
    pub fn new(app: &App) -> SetupState {
        // put each enemy on its role's row; enemies without a known role fill the rest
        let mut slots: Vec<Option<&Enemy>> = vec![None; 5];
        let mut rest = Vec::new();
        for e in app.tracker.enemies.iter().take(5) {
            match ROLES.iter().position(|r| Some(*r) == e.role.as_deref()) {
                Some(i) if slots[i].is_none() => slots[i] = Some(e),
                _ => rest.push(e),
            }
        }
        for s in slots.iter_mut() { if s.is_none() && !rest.is_empty() { *s = Some(rest.remove(0)); } }
        let rows = slots.iter().map(|e| Row {
            champ: e.map(|e| e.name.clone()).unwrap_or_default(),
            s1: e.and_then(|e| e.spell1_id.clone()).or(Some("SummonerFlash".into())),
            s2: e.and_then(|e| e.spell2_id.clone()),
            level: e.map(|e| e.level).unwrap_or(1),
        }).collect();
        SetupState {
            draft: app.settings.clone(),
            rows,
            roster_note: if app.clock.is_live { "A live game is detected: roster, spells, levels and items come from the game and refresh every second. Manual entries would be overwritten.".into() }
                         else { "No game detected. Enter the enemy champions and spells to track them by hand. A live game replaces this automatically.".into() },
            area_note: String::new(),
            listening: None,
            detect_rx: None,
            countdown: None,
            picker: None,
            chrome_for: None,
        }
    }
}

fn card(ui: &mut egui::Ui, p: &Pal, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none().fill(p.surface).stroke(Stroke::new(1.0_f32, p.line)).rounding(10.0_f32).inner_margin(egui::Margin::symmetric(16.0, 14.0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).font(theme::semi(11.0)).color(p.accent_deep));
        ui.add_space(4.0);
        body(ui);
    });
    ui.add_space(12.0);
}

fn hint(ui: &mut egui::Ui, p: &Pal, t: &str) {
    ui.add(egui::Label::new(RichText::new(t).size(11.5).color(p.muted)).wrap());
}

fn primary(ui: &mut egui::Ui, p: &Pal, t: &str) -> bool {
    ui.add(egui::Button::new(RichText::new(t).font(theme::semi(12.5)).color(p.accent_text)).fill(p.accent).stroke(Stroke::NONE).rounding(6.0_f32).min_size(egui::vec2(84.0, 30.0))).clicked()
}

fn region_text(r: [f64; 4]) -> String {
    format!("Reading left {:.0}-{:.0}%, top {:.0}-{:.0}% of the game window. Saved with Save.", r[0] * 100.0, (r[0] + r[2]) * 100.0, r[1] * 100.0, (r[1] + r[3]) * 100.0)
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut st) = app.setup.take() else { return };
    let mut close = false;
    let mut save = false;
    let vb = ViewportBuilder::default().with_title("Flashless Settings").with_inner_size([680.0, 820.0]).with_min_inner_size([600.0, 480.0])
        .with_icon(Arc::new(theme::icon_data()));
    ctx.show_viewport_immediate(ViewportId::from_hash_of("flashless-settings"), vb, |ctx, _| {
        let p = app.pal;
        if st.chrome_for != Some(p.dark) {
            if let Some(h) = win::find_window("Flashless Settings") {
                win::set_title_bar(h, p.dark, theme::rgb(p.bg), theme::rgb(p.text), theme::rgb(p.line_strong));
                st.chrome_for = Some(p.dark);
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) { close = true; }

        egui::TopBottomPanel::bottom("setup-footer").frame(egui::Frame::none().fill(p.surface).inner_margin(egui::Margin::symmetric(24.0, 12.0)).stroke(Stroke::new(1.0_f32, p.line)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Changes apply when you save.").size(11.5).color(p.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if primary(ui, &p, "Save") { save = true; }
                        if ui.add(egui::Button::new("Cancel").min_size(egui::vec2(80.0, 30.0))).clicked() { close = true; }
                    });
                });
            });

        egui::CentralPanel::default().frame(egui::Frame::none().fill(p.bg).inner_margin(egui::Margin { left: 24.0, right: 12.0, top: 18.0, bottom: 0.0 })).show(ctx, |ui| {
            // title
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
                theme::brand(ui.painter(), r.center(), 13.0, &p);
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new("Settings").font(theme::semi(20.0)));
                    ui.label(RichText::new("Roster, chat message, chat reading, appearance and hotkeys").size(12.0).color(p.muted));
                });
            });
            ui.add_space(12.0);
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.set_width(ui.available_width() - 12.0);
                roster(ui, app, &mut st, &p);
                message_card(ui, &mut st, &p);
                clipboard_card(ui, &mut st, &p);
                chat_card(ui, ctx, &mut st, &p);
                appearance_card(ui, ctx, app, &mut st);
                hotkeys_card(ui, ctx, &mut st, &p);
            });
        });

        if st.countdown.is_some() || st.detect_rx.is_some() { ctx.request_repaint_after(Duration::from_millis(100)); }
    });

    if let Some(pk) = st.picker.as_mut() {
        if let Some(result) = show_picker(ctx, pk, st.draft.chat_region, &app.pal) {
            if let Some(r) = result { st.draft.chat_region = r; st.area_note = "Area set.".into(); }
            st.picker = None;
        }
    }

    if save {
        let (left, top) = (app.settings.left, app.settings.top);
        app.settings = st.draft.clone();
        app.settings.left = left;
        app.settings.top = top;
        app.apply_settings(ctx);
        close = true;
    } else if close {
        app.apply_settings_theme_only(ctx); // put back the saved theme after a preview
    }
    if close { hotkeys::set_paused(false); } else { app.setup = Some(st); }
}

fn roster(ui: &mut egui::Ui, app: &mut App, st: &mut SetupState, p: &Pal) {
    let spells: Vec<(String, String)> = app.data.standard_spells().into_iter().map(|s| (s.id.clone(), s.name.clone())).collect();
    card(ui, p, "ENEMY ROSTER", |ui| {
        hint(ui, p, &st.roster_note);
        ui.add_space(6.0);
        for (i, row) in st.rows.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized([36.0, 26.0], egui::Label::new(RichText::new(ROLES[i]).font(theme::semi(12.0)).color(p.accent_deep)));
                ui.add(egui::TextEdit::singleline(&mut row.champ).hint_text("Champion").desired_width(170.0));
                for (k, sel) in [&mut row.s1, &mut row.s2].into_iter().enumerate() {
                    let current = sel.as_deref().and_then(|id| spells.iter().find(|(i2, _)| i2 == id)).map(|(_, n)| n.clone()).unwrap_or("—".into());
                    egui::ComboBox::from_id_salt(("spell", i, k)).width(104.0).selected_text(current).show_ui(ui, |ui| {
                        for (id, name) in &spells { ui.selectable_value(sel, Some(id.clone()), name); }
                    });
                }
                ui.label(RichText::new("Lv").size(12.0).color(p.muted));
                ui.add(egui::DragValue::new(&mut row.level).range(1..=18).speed(0.1));
            });
        }
        ui.add_space(6.0);
        if ui.add(egui::Button::new("Apply roster").min_size(egui::vec2(110.0, 28.0))).clicked() {
            let mut enemies = Vec::new();
            for (i, row) in st.rows.iter().enumerate() {
                let text = row.champ.trim();
                if text.is_empty() { continue; }
                let champ = app.data.find_champion(text).or_else(|| app.data.champions.iter().find(|c| c.name.to_lowercase().starts_with(&text.to_lowercase())));
                let Some(champ) = champ else { st.roster_note = format!("Unknown champion: {text}"); return; };
                let mut e = app.tracker.enemies.iter().find(|x| x.champion_id == champ.id).cloned()
                    .unwrap_or_else(|| Enemy { champion_id: champ.id.clone(), name: champ.name.clone(), ..Default::default() });
                e.level = row.level.clamp(1, 18);
                e.spell1_id = row.s1.clone();
                e.spell2_id = row.s2.clone();
                e.role = Some(ROLES[i].into());
                enemies.push(e);
            }
            st.roster_note = format!("Roster applied: {} enemies.", enemies.len());
            app.tracker.set_roster(enemies);
        }
    });
}

fn message_card(ui: &mut egui::Ui, st: &mut SetupState, p: &Pal) {
    card(ui, p, "CHAT MESSAGE", |ui| {
        ui.horizontal(|ui| {
            ui.radio_value(&mut st.draft.message.format, TimeFormat::GameClock, "Game clock when back up (2230 = 22:30)");
            ui.radio_value(&mut st.draft.message.format, TimeFormat::SecondsRemaining, "Seconds remaining");
        });
        ui.checkbox(&mut st.draft.message.use_roles, "Name enemies by lane (Top, Jg, Mid, ADC, Sup)");
        ui.checkbox(&mut st.draft.message.longest_first, "Other spells: longest cooldown first (off = soonest first)");
        hint(ui, p, "Always one line. Flash leads: enemies are ordered by when their Flash is back.");
    });
}

fn clipboard_card(ui: &mut egui::Ui, st: &mut SetupState, p: &Pal) {
    card(ui, p, "CLIPBOARD AND TYPING", |ui| {
        ui.checkbox(&mut st.draft.auto_copy, "Copy the message automatically when a cooldown is registered");
        egui::Grid::new("clip-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Delay before copying (ms)");
            ui.add(egui::DragValue::new(&mut st.draft.copy_delay_ms).range(0..=5000).speed(5.0));
            ui.end_row();
            ui.label("Assume a spell was used this many seconds before it was tracked")
                .on_hover_text("Applies to clicks, hotkeys and chat detection. 7 s turns a fresh Flash from 5:00 into 4:53.");
            ui.add(egui::DragValue::new(&mut st.draft.reaction_offset_seconds).range(0.0..=30.0).speed(0.5).suffix(" s"));
            ui.end_row();
        });
        hint(ui, p, "The Type In Chat hotkey (Ctrl+Alt+V by default) opens chat and types the message, but does not send it: you press Enter. It simulates key presses while League is focused, only when you press the hotkey. Riot does not sanction input automation; use it at your own risk.");
    });
}

fn chat_card(ui: &mut egui::Ui, ctx: &egui::Context, st: &mut SetupState, p: &Pal) {
    card(ui, p, "CHAT READING", |ui| {
        ui.checkbox(&mut st.draft.chat_detection, "Register \"Champion used Spell\" lines from the game chat automatically");
        hint(ui, p, "Captures the chat area of the game window about once a second and reads it with the OCR built into Windows. Picks up the line League posts when someone clicks an enemy's summoner spell. Needs Borderless or Windowed mode and chat timestamps on.");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Chat area").size(12.5));
            ui.add_space(8.0);
            if ui.button(RichText::new("Draw area").font(theme::regular(12.0))).on_hover_text("Drag a box around the chat over the game window").clicked() {
                let ppp = ctx.pixels_per_point();
                let bounds = win::game_window().and_then(win::window_rect)
                    .map(|r| Rect::from_min_max(egui::pos2(r.left as f32 / ppp, r.top as f32 / ppp), egui::pos2(r.right as f32 / ppp, r.bottom as f32 / ppp)))
                    .or_else(|| ctx.input(|i| i.viewport().monitor_size).map(|s| Rect::from_min_size(egui::Pos2::ZERO, s)))
                    .unwrap_or(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0)));
                st.picker = Some(Picker { bounds, start: None, sel: None });
            }
            let detecting = st.countdown.is_some() || st.detect_rx.is_some();
            if ui.add_enabled(!detecting, egui::Button::new(RichText::new("Auto-detect").font(theme::regular(12.0))))
                .on_hover_text("Finds the chat from its timestamps. Open the chat history in game during the countdown.").clicked() {
                if win::game_window().is_none() { st.area_note = "League is not running.".into(); } else { st.countdown = Some(Instant::now() + Duration::from_secs(5)); }
            }
            if ui.button("Reset").clicked() { st.draft.chat_region = DEFAULT_CHAT_REGION; st.area_note = "Reset to default.".into(); }
        });
        if let Some(t) = st.countdown {
            let left = t.saturating_duration_since(Instant::now()).as_secs_f32().ceil() as i32;
            if left <= 0 {
                st.countdown = None;
                let (tx, rx) = channel();
                st.detect_rx = Some(rx);
                std::thread::spawn(move || { let _ = tx.send(watcher::detect_chat_area()); });
                st.area_note = "Reading...".into();
            } else {
                st.area_note = format!("Switch to League and press Enter so the chat history shows. Reading in {left}...");
            }
        }
        if let Some(rx) = &st.detect_rx {
            if let Ok(res) = rx.try_recv() {
                match res {
                    Ok((r, n)) => { st.draft.chat_region = r; st.area_note = format!("Found {n} chat lines."); }
                    Err(e) => st.area_note = e,
                }
                st.detect_rx = None;
            }
        }
        let text = if st.area_note.is_empty() { region_text(st.draft.chat_region) } else { format!("{}  {}", st.area_note, region_text(st.draft.chat_region)) };
        hint(ui, p, &text);
    });
}

fn appearance_card(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut App, st: &mut SetupState) {
    let p = app.pal;
    card(ui, &p, "APPEARANCE", |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Theme").size(12.5));
            ui.add_space(12.0);
            for mode in ["System", "Light", "Dark"] {
                if ui.radio_value(&mut st.draft.theme, mode.to_string(), mode).changed() {
                    app.pal = theme::resolve(mode); // live preview; Cancel restores the saved theme
                    theme::apply(ctx, &app.pal);
                }
            }
        });
        ui.checkbox(&mut st.draft.always_on_top, "Keep the overlay on top of other windows");
        ui.checkbox(&mut st.draft.show_preview, "Show the chat message preview in the overlay");
        ui.checkbox(&mut st.draft.compact, "Compact mode: lane and spell icons only (toggle from the overlay or tray too)");
        ui.checkbox(&mut st.draft.auto_compact, "Switch to compact mode during a game, full view in the lobby");
        ui.checkbox(&mut st.draft.hide_outside_game, "Hide the overlay when no game is running (tray icon still shows it)");
        ui.checkbox(&mut st.draft.click_through_in_game, "Clicks go through the overlay while League is in front (use hotkeys, or alt-tab to edit)");
        ui.checkbox(&mut st.draft.flash_alert, "Pulse an enemy's Flash when it comes back up");
        ui.add_enabled(st.draft.flash_alert, egui::Checkbox::new(&mut st.draft.flash_alert_sound, "Also play the Windows notification sound"));
        ui.checkbox(&mut st.draft.check_updates, "Check for a new version when Flashless starts");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Overlay opacity");
            ui.add(egui::Slider::new(&mut st.draft.opacity, 0.3..=1.0).show_value(false).trailing_fill(true));
            ui.label(format!("{:.0}%", st.draft.opacity * 100.0));
        });
        ui.add_space(4.0);
        ui.label("Spell order in the overlay");
        ui.horizontal(|ui| {
            use crate::settings::SpellOrder;
            ui.radio_value(&mut st.draft.spell_order, SpellOrder::FlashLeft, "Flash on the left");
            ui.radio_value(&mut st.draft.spell_order, SpellOrder::FlashRight, "Flash on the right");
            ui.radio_value(&mut st.draft.spell_order, SpellOrder::Game, "As the enemy has them (D / F)");
        });
    });
}

fn hotkeys_card(ui: &mut egui::Ui, ctx: &egui::Context, st: &mut SetupState, p: &Pal) {
    // capture a combination while a box is listening
    if let Some(action) = st.listening.clone() {
        hotkeys::set_paused(true);
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            // mouse buttons 3/4/5 (left and right click stay free for using the window)
            if let egui::Event::PointerButton { button, pressed: true, modifiers, .. } = ev {
                let name = match button {
                    egui::PointerButton::Middle => "Mouse3",
                    egui::PointerButton::Extra1 => "Mouse4",
                    egui::PointerButton::Extra2 => "Mouse5",
                    _ => continue,
                };
                let mut parts = Vec::new();
                if modifiers.ctrl { parts.push("Ctrl".to_string()); }
                if modifiers.alt { parts.push("Alt".to_string()); }
                if modifiers.shift { parts.push("Shift".to_string()); }
                parts.push(name.to_string());
                st.draft.hotkeys.insert(action.clone(), parts.join("+"));
                st.listening = None;
                hotkeys::set_paused(false);
                break;
            }
            if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev {
                if matches!(key, egui::Key::Escape | egui::Key::Backspace | egui::Key::Delete) {
                    st.draft.hotkeys.insert(action.clone(), String::new());
                } else {
                    let name = key.name().to_string();
                    if hotkeys::key_to_vk(&name).is_none() { continue; }
                    let mut parts = Vec::new();
                    if modifiers.ctrl { parts.push("Ctrl".to_string()); }
                    if modifiers.alt { parts.push("Alt".to_string()); }
                    if modifiers.shift { parts.push("Shift".to_string()); }
                    parts.push(name);
                    st.draft.hotkeys.insert(action.clone(), parts.join("+"));
                }
                st.listening = None;
                hotkeys::set_paused(false);
                break;
            }
        }
    }
    card(ui, p, "HOTKEYS", |ui| {
        hint(ui, p, "Click a box, then press a key combination or mouse button 3, 4 or 5 (Ctrl/Alt/Shift work with both). Esc or Backspace clears it.");
        ui.add_space(6.0);
        let names: Vec<String> = st.draft.hotkeys.keys().cloned().collect();
        let order = |n: &String| match n.as_str() { "CopySummary" => 0, "TypeInChat" => 1, "Undo" => 2, "ResetAll" => 3, "ToggleOverlay" => 4, _ => 10 };
        let mut names = names;
        names.sort_by_key(|n| (order(n), n.clone()));
        egui::Grid::new("hotkeys").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
            for (k, action) in names.iter().enumerate() {
                let label = hotkey_label(action);
                ui.label(RichText::new(label).size(12.5));
                let listening = st.listening.as_deref() == Some(action.as_str());
                let combo = st.draft.hotkeys.get(action).cloned().unwrap_or_default();
                let text = if listening { "Key or mouse...".to_string() } else if combo.is_empty() { "—".to_string() } else { combo };
                let b = egui::Button::new(RichText::new(text).size(12.0).color(if listening { p.accent_deep } else { p.text }))
                    .min_size(egui::vec2(128.0, 26.0)).fill(if listening { p.accent_soft } else { p.surface });
                if ui.add(b).clicked() { st.listening = Some(action.clone()); }
                if k % 2 == 1 { ui.end_row(); }
            }
        });
    });
}

/// "Enemy1Flash" -> "Top Flash", "TypeInChat" -> "Type In Chat".
fn hotkey_label(action: &str) -> String {
    if action == "TypeInChat" { return "Type In Chat".into(); }
    if let Some(rest) = action.strip_prefix("Enemy") {
        if let Some(n) = rest.chars().next().and_then(|c| c.to_digit(10)) {
            if (1..=5).contains(&n) { return format!("{} {}", ROLES[n as usize - 1], split_words(&rest[1..])); }
        }
    }
    split_words(action)
}

fn split_words(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (c.is_uppercase() || (c.is_ascii_digit() && !chars[i - 1].is_ascii_digit())) { out.push(' '); }
        out.push(*c);
    }
    out
}

/// Full-window dimmed layer over the game for dragging a box around the chat.
/// Returns Some(Some(region)) when saved, Some(None) when cancelled, None while open.
fn show_picker(ctx: &egui::Context, pk: &mut Picker, current: [f64; 4], p: &Pal) -> Option<Option<[f64; 4]>> {
    let mut result = None;
    let vb = ViewportBuilder::default().with_title("Flashless chat area").with_decorations(false).with_transparent(true)
        .with_always_on_top().with_taskbar(false).with_position(pk.bounds.min).with_inner_size(pk.bounds.size()).with_resizable(false);
    ctx.show_viewport_immediate(ViewportId::from_hash_of("flashless-picker"), vb, |ctx, _| {
        egui::CentralPanel::default().frame(egui::Frame::none().fill(Color32::from_black_alpha(100))).show(ctx, |ui| {
            let full = ui.max_rect();
            let resp = ui.allocate_rect(full, egui::Sense::drag());
            let size = full.size();
            let painter = ui.painter();
            let old = Rect::from_min_size(full.min + egui::vec2(current[0] as f32 * size.x, current[1] as f32 * size.y),
                                          egui::vec2(current[2] as f32 * size.x, current[3] as f32 * size.y));
            painter.rect_stroke(old, 0.0_f32, Stroke::new(1.5_f32, Color32::from_rgb(251, 191, 36)));
            if resp.drag_started() { pk.start = resp.interact_pointer_pos(); }
            if let (Some(s), Some(now)) = (pk.start, resp.interact_pointer_pos()) {
                if resp.dragged() { pk.sel = Some(Rect::from_two_pos(s, now)); }
            }
            if let Some(sel) = pk.sel {
                painter.rect(sel, 0.0_f32, Color32::from_rgba_unmultiplied(50, 205, 50, 40), Stroke::new(2.0_f32, Color32::from_rgb(50, 205, 50)));
            }
            let help = "Drag a box around the chat, including where older lines appear above it.\nYellow box = current area.   Enter = save   Esc = cancel";
            let g = painter.layout(help.into(), theme::regular(15.0), Color32::WHITE, 700.0);
            let pos = egui::pos2(full.center().x - g.size().x / 2.0, full.top() + size.y * 0.12);
            painter.rect_filled(Rect::from_min_size(pos - egui::vec2(16.0, 12.0), g.size() + egui::vec2(32.0, 24.0)), 8.0, p.bg.gamma_multiply(0.92));
            painter.galley(pos, g, Color32::WHITE);
        });
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Escape) || i.viewport().close_requested() { result = Some(None); }
            if i.key_pressed(egui::Key::Enter) {
                if let Some(sel) = pk.sel.filter(|s| s.width() > 20.0 && s.height() > 20.0) {
                    let full = i.screen_rect();
                    let f = |v: f32, total: f32| ((v / total) as f64 * 10000.0).round() / 10000.0;
                    result = Some(Some([f(sel.left() - full.left(), full.width()), f(sel.top() - full.top(), full.height()), f(sel.width(), full.width()), f(sel.height(), full.height())]));
                }
            }
        });
    });
    result
}
