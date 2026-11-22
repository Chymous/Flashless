//! Settings window (a second native window): a sidebar (Overview, Settings, Hotkeys, Roster, About) and, for
//! Settings, tabs (General, Overlay, Chat Message, Roster, Hotkeys) of cards. Edits go to a draft that Save
//! applies and Cancel drops.

use super::app::{self, App};
use super::theme::{self, Pal};
use crate::message::TimeFormat;
use crate::settings::{Settings, SpellOrder, DEFAULT_CHAT_REGION};
use crate::tracker::{Enemy, Slot, Tracker};
use crate::win::{self, hotkeys, watcher};
use eframe::egui::{self, Align2, Color32, Rect, RichText, Sense, Stroke, ViewportBuilder, ViewportId};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ROLES: [&str; 5] = ["Top", "Jg", "Mid", "ADC", "Sup"];
const RIOT_DEV_URL: &str = "https://developer.riotgames.com/";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page { Overview, General, Overlay, Message, Roster, Hotkeys, About }

impl Page {
    fn is_settings_tab(self) -> bool { matches!(self, Page::General | Page::Overlay | Page::Message | Page::Roster | Page::Hotkeys) }
}

/// Sidebar: label, Segoe icon glyph, page it opens.
const NAV: [(&str, &str, Page); 5] = [
    ("Overview", "\u{E80F}", Page::Overview),
    ("Settings", "\u{E713}", Page::Overlay),
    ("Hotkeys", "\u{E765}", Page::Hotkeys),
    ("Roster", "\u{E716}", Page::Roster),
    ("About", "\u{E946}", Page::About),
];

/// Tabs on the Settings page.
const TABS: [(&str, &str, Page); 5] = [
    ("General", "\u{E713}", Page::General),
    ("Overlay", "\u{E7F4}", Page::Overlay),
    ("Chat Message", "\u{E8BD}", Page::Message),
    ("Roster", "\u{E716}", Page::Roster),
    ("Hotkeys", "\u{E765}", Page::Hotkeys),
];

/// Game time the Settings preview pretends it is.
const SAMPLE_NOW: f64 = 14.0 * 60.0;

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
    page: Page,
    rows: Vec<Row>,
    roster_note: String,
    area_note: String,
    listening: Option<String>,
    detect_rx: Option<Receiver<Result<([f64; 4], usize), String>>>,
    countdown: Option<Instant>,
    picker: Option<Picker>,
    chrome_for: Option<bool>,
    /// Brought in front of other windows once it exists.
    raised: bool,
    /// Key being typed; cleared once saved. Never shown again after saving.
    key_input: String,
    key_note: String,
    /// Sample enemies for the previews: a tracker of their own, never the real one.
    sample: Tracker,
}

impl SetupState {
    /// The unsaved portrait choice, so the overlay previews it while Settings is open.
    pub fn draft_portraits(&self) -> bool { self.draft.show_portraits }

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
            page: Page::Overlay,
            rows,
            roster_note: if app.clock.is_live { "A live game is detected: roster, spells, levels and items come from the game and refresh every second. Manual entries would be overwritten.".into() }
                         else { "No game detected. Enter the enemy champions and spells to track them by hand. A live game replaces this automatically.".into() },
            area_note: String::new(),
            listening: None,
            detect_rx: None,
            countdown: None,
            picker: None,
            chrome_for: None,
            raised: false,
            key_input: String::new(),
            key_note: String::new(),
            sample: sample_tracker(app),
        }
    }
}

/// Four made-up enemies with cooldowns running, for the previews only.
fn sample_tracker(app: &App) -> Tracker {
    let mk = |id: &str, role: &str, s2: &str| Enemy {
        champion_id: id.into(), name: app.data.find_champion(id).map(|c| c.name.clone()).unwrap_or_else(|| id.into()),
        level: 9, spell1_id: Some("SummonerFlash".into()), spell2_id: Some(s2.into()), role: Some(role.into()), ..Default::default()
    };
    let mut t = Tracker::default();
    let mut roster = vec![mk("Lucian", "ADC", "SummonerDot"), mk("Nautilus", "Sup", "SummonerExhaust"), mk("Shen", "Top", "SummonerTeleport"), mk("Syndra", "Mid", "SummonerBarrier")];
    crate::tracker::sort_by_role(&mut roster); // lane order, Top first, like a live game
    t.set_roster(roster);
    let rules = crate::cooldown::CooldownRules::default();
    for (name, slot, ago) in [("Lucian", Slot::Spell1, 48.0), ("Lucian", Slot::Spell2, 60.0), ("Nautilus", Slot::Spell1, 115.0), ("Shen", Slot::Spell2, 140.0), ("Syndra", Slot::Spell1, 20.0)] {
        if let Some(i) = t.enemies.iter().position(|e| e.champion_id == name) { t.register(i, slot, ago, SAMPLE_NOW, true, &app.data, &rules); }
    }
    t
}

// ---------- building blocks ----------

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Icon glyph and label as one button text.
fn icon_label(glyph: &str, label: &str, size: f32, col: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(glyph, 0.0, egui::TextFormat { font_id: theme::icon(size), color: col, valign: egui::Align::Center, ..Default::default() });
    job.append(label, 9.0, egui::TextFormat { font_id: theme::regular(size), color: col, valign: egui::Align::Center, ..Default::default() });
    job
}

/// A card with a gold icon and a title.
fn card(ui: &mut egui::Ui, p: &Pal, glyph: &str, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none().fill(p.surface).stroke(Stroke::new(1.0_f32, p.line)).rounding(10.0_f32).inner_margin(egui::Margin::symmetric(20.0, 18.0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        heading(ui, p, glyph, title);
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.y = 10.0;
        body(ui);
    });
    ui.add_space(16.0);
}

fn heading(ui: &mut egui::Ui, p: &Pal, glyph: &str, title: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(glyph).font(theme::icon(17.0)).color(p.accent_deep));
        ui.add_space(4.0);
        ui.label(RichText::new(title).font(theme::semi(16.0)).color(p.text));
    });
}

/// A titled part inside a card, below a divider.
fn section(ui: &mut egui::Ui, p: &Pal, glyph: &str, title: &str) {
    ui.add_space(4.0);
    divider(ui, p);
    ui.add_space(4.0);
    heading(ui, p, glyph, title);
}

fn hint(ui: &mut egui::Ui, p: &Pal, t: &str) {
    ui.add(egui::Label::new(RichText::new(t).size(12.0).color(p.secondary)).wrap());
}

fn divider(ui: &mut egui::Ui, p: &Pal) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0_f32, p.line);
}

/// A label in a fixed-width column, then the control.
fn field(ui: &mut egui::Ui, p: &Pal, label: &str, body: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_sized([116.0, 30.0], egui::Label::new(RichText::new(label).size(13.5).color(p.text)).halign(egui::Align::LEFT));
        body(ui);
    }).response
}

/// Outlined button (Cancel, Remove key, Look up runes now).
fn outline(ui: &mut egui::Ui, p: &Pal, text: impl Into<egui::WidgetText>, min_w: f32, h: f32) -> egui::Response {
    ui.add(egui::Button::new(text).fill(Color32::TRANSPARENT).stroke(Stroke::new(1.0_f32, p.line_strong)).rounding(7.0_f32).min_size(egui::vec2(min_w, h)))
}

fn button(ui: &mut egui::Ui, p: &Pal, t: &str) -> egui::Response {
    outline(ui, p, RichText::new(t).size(13.0), 0.0, 30.0)
}

/// Large filled gold button (Save).
fn gold(ui: &mut egui::Ui, p: &Pal, t: &str, size: egui::Vec2) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.is_pointer_button_down_on() { p.accent_muted } else if resp.hovered() { p.accent_hover } else { p.accent };
    ui.painter().rect_filled(r, 7.0_f32, fill);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, t, theme::semi(14.0), p.accent_text);
    resp
}

/// Gold handle on a rail, filled gold up to the value.
fn slider(ui: &mut egui::Ui, p: &Pal, v: &mut f32, range: std::ops::RangeInclusive<f32>, w: f32) -> bool {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), Sense::click_and_drag());
    let (lo, hi) = (*range.start(), *range.end());
    let rail = Rect::from_center_size(r.center(), egui::vec2(w - 16.0, 5.0));
    let mut changed = false;
    if let Some(pos) = resp.interact_pointer_pos() {
        let t = ((pos.x - rail.left()) / rail.width()).clamp(0.0, 1.0);
        let nv = ((lo + t * (hi - lo)) * 100.0).round() / 100.0;
        if (nv - *v).abs() > f32::EPSILON { *v = nv; changed = true; }
    }
    let t = ((*v - lo) / (hi - lo)).clamp(0.0, 1.0);
    let x = rail.left() + t * rail.width();
    let painter = ui.painter();
    painter.rect_filled(rail, 2.5_f32, p.line_strong);
    painter.rect_filled(Rect::from_min_max(rail.min, egui::pos2(x, rail.max.y)), 2.5_f32, p.accent);
    let knob = if resp.hovered() || resp.dragged() { p.accent_hover } else { p.accent };
    painter.circle(egui::pos2(x, rail.center().y), 8.0, knob, Stroke::new(1.5_f32, p.accent_muted));
    changed
}

/// Two columns when there is room, one under the other when not.
fn two_col<S>(ui: &mut egui::Ui, s: &mut S, left: impl FnOnce(&mut egui::Ui, &mut S), right: impl FnOnce(&mut egui::Ui, &mut S)) {
    if ui.available_width() >= 720.0 {
        ui.spacing_mut().item_spacing.x = 16.0;
        ui.columns(2, |c| {
            let (a, b) = c.split_at_mut(1);
            left(&mut a[0], s);
            right(&mut b[0], s);
        });
    } else {
        left(ui, s);
        right(ui, s);
    }
}

fn region_text(r: [f64; 4]) -> String {
    format!("Reading left {:.0}-{:.0}%, top {:.0}-{:.0}% of the game window.", r[0] * 100.0, (r[0] + r[2]) * 100.0, r[1] * 100.0, (r[1] + r[3]) * 100.0)
}

// ---------- window ----------

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut st) = app.setup.take() else { return };
    let mut close = false;
    let mut save = false;
    let vb = ViewportBuilder::default().with_title("Flashless Settings").with_inner_size([1060.0, 720.0]).with_min_inner_size([880.0, 520.0])
        .with_icon(Arc::new(theme::icon_data()));
    ctx.show_viewport_immediate(ViewportId::from_hash_of("flashless-settings"), vb, |ctx, _| {
        let p = app.pal;
        if st.chrome_for != Some(p.dark) || !st.raised {
            if let Some(h) = win::find_window("Flashless Settings") {
                win::set_title_bar(h, p.dark, theme::rgb(p.sidebar), theme::rgb(p.text), theme::rgb(p.line));
                st.chrome_for = Some(p.dark);
                // the overlay never takes focus, so Windows would open this behind the app in front
                if !st.raised { win::set_icon_from_resource(h); win::bring_to_front(h); st.raised = true; }
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) { close = true; }
        // these run whatever page is showing
        capture_hotkey(ctx, &mut st);
        poll_chat_detect(&mut st);

        egui::SidePanel::left("setup-nav").exact_width(212.0).resizable(false)
            .frame(egui::Frame::none().fill(p.sidebar).inner_margin(egui::Margin { left: 14.0, right: 14.0, top: 20.0, bottom: 12.0 }).stroke(Stroke::new(1.0_f32, p.line)))
            .show(ctx, |ui| nav(ui, &mut st, &p));

        egui::TopBottomPanel::bottom("setup-footer").frame(egui::Frame::none().fill(p.bg).inner_margin(egui::Margin { left: 28.0, right: 20.0, top: 14.0, bottom: 14.0 }).stroke(Stroke::new(1.0_f32, p.line)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("\u{E946}").font(theme::icon(15.0)).color(p.secondary));
                    ui.add_space(4.0);
                    ui.label(RichText::new("Changes apply when you save.").size(13.0).color(p.secondary));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;
                        if gold(ui, &p, "Save", egui::vec2(116.0, 38.0)).clicked() { save = true; }
                        if outline(ui, &p, RichText::new("Cancel").size(14.0), 116.0, 38.0).clicked() { close = true; }
                    });
                });
            });

        egui::CentralPanel::default().frame(egui::Frame::none().fill(p.bg).inner_margin(egui::Margin { left: 30.0, right: 16.0, top: 22.0, bottom: 0.0 })).show(ctx, |ui| {
            let (title, sub) = match st.page {
                Page::Overview => ("Overview", "What Flashless sees right now."),
                Page::About => ("About", "Version, data and credits."),
                _ => ("Settings", "Configure tracking, messages, and appearance."),
            };
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(RichText::new(title).font(theme::semi(26.0)).color(p.text));
            ui.label(RichText::new(sub).size(13.5).color(p.secondary));
            ui.add_space(10.0);
            if st.page.is_settings_tab() {
                ui.scope(|ui| { ui.set_width(ui.available_width() - 14.0); tabs(ui, &mut st, &p); });
                ui.add_space(16.0);
            } else {
                ui.add_space(8.0);
            }
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.set_width(ui.available_width() - 14.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                match st.page {
                    Page::Overview => overview_page(ui, app, &p),
                    Page::General => two_col(ui, &mut st, |ui, st| chat_card(ui, ctx, st, &p), |ui, st| clipboard_card(ui, st, &p)),
                    Page::Overlay => overlay_page(ui, ctx, app, &mut st),
                    Page::Message => message_page(ui, app, &mut st, &p),
                    Page::Roster => roster(ui, app, &mut st, &p),
                    Page::Hotkeys => hotkeys_card(ui, &mut st, &p),
                    Page::About => about_page(ui, app, &p),
                }
                ui.add_space(8.0);
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

fn go(st: &mut SetupState, page: Page) {
    if st.page == page { return; }
    st.page = page;
    if st.listening.take().is_some() { hotkeys::set_paused(false); }
}

fn nav(ui: &mut egui::Ui, st: &mut SetupState, p: &Pal) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        let (r, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), Sense::hover());
        theme::brand(ui.painter(), r.center(), 12.0, p);
        ui.add_space(2.0);
        ui.label(RichText::new("Flashless").font(theme::semi(19.0)).color(p.text));
        let g = ui.painter().layout_no_wrap(concat!("v", env!("CARGO_PKG_VERSION")).into(), theme::regular(11.0), p.muted);
        let (pr, _) = ui.allocate_exact_size(g.size() + egui::vec2(14.0, 6.0), Sense::hover());
        ui.painter().rect_filled(pr, 9.0_f32, mix(p.sidebar, p.text, 0.05));
        ui.painter().galley(pr.center() - g.size() / 2.0, g, p.muted);
    });
    ui.add_space(26.0);
    for (label, glyph, target) in NAV {
        let sel = match target {
            Page::Overlay => st.page == Page::Overlay || st.page == Page::General || st.page == Page::Message,
            t => st.page == t,
        };
        let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 42.0), Sense::click());
        let painter = ui.painter();
        if sel {
            // gold edge, warm wash fading to the right
            let end = mix(p.accent_soft, p.sidebar, 0.3);
            painter.rect_filled(r, 8.0_f32, end);
            let g = Rect::from_min_max(egui::pos2(r.left() + 3.0, r.top()), egui::pos2(r.right() - 8.0, r.bottom()));
            let mut mesh = egui::epaint::Mesh::default();
            mesh.colored_vertex(g.left_top(), p.accent_soft);
            mesh.colored_vertex(g.right_top(), end);
            mesh.colored_vertex(g.right_bottom(), end);
            mesh.colored_vertex(g.left_bottom(), p.accent_soft);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(egui::Shape::mesh(mesh));
            painter.rect_stroke(r, 8.0_f32, Stroke::new(1.0_f32, mix(p.accent_soft, p.accent, 0.12)));
            painter.rect_filled(Rect::from_min_size(r.min, egui::vec2(3.0, r.height())), egui::Rounding { nw: 8.0, sw: 8.0, ne: 0.0, se: 0.0 }, p.accent);
        } else if resp.hovered() {
            painter.rect_filled(r, 8.0_f32, p.surface_hover);
        }
        painter.text(egui::pos2(r.left() + 22.0, r.center().y), Align2::CENTER_CENTER, glyph, theme::icon(16.0), if sel { p.accent_deep } else { mix(p.text, p.secondary, 0.6) });
        painter.text(egui::pos2(r.left() + 46.0, r.center().y), Align2::LEFT_CENTER, label, theme::regular(14.5), if sel || resp.hovered() { p.text } else { mix(p.text, p.secondary, 0.25) });
        if resp.clicked() && !sel { go(st, target); }
        ui.add_space(4.0);
    }
}

fn tabs(ui: &mut egui::Ui, st: &mut SetupState, p: &Pal) {
    let (bar, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), Sense::hover());
    ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0_f32, p.line));
    let mut x = bar.left();
    for (label, glyph, page) in TABS {
        let g = ui.painter().layout_no_wrap(label.into(), theme::regular(14.0), p.text);
        let w = 16.0 + 26.0 + g.size().x + 18.0;
        let r = Rect::from_min_size(egui::pos2(x, bar.top()), egui::vec2(w, bar.height()));
        x += w + 6.0;
        let resp = ui.interact(r, ui.id().with(("tab", label)), Sense::click());
        let sel = st.page == page;
        let col = if sel { p.accent_deep } else if resp.hovered() { p.text } else { mix(p.text, p.secondary, 0.15) };
        let painter = ui.painter();
        painter.text(egui::pos2(r.left() + 24.0, r.center().y - 1.0), Align2::CENTER_CENTER, glyph, theme::icon(16.0), col);
        painter.galley_with_override_text_color(egui::pos2(r.left() + 42.0, r.center().y - 1.0 - g.size().y / 2.0), g, col);
        if sel { painter.rect_filled(Rect::from_min_max(egui::pos2(r.left(), r.bottom() - 2.5), r.right_bottom()), 1.0_f32, p.accent_deep); }
        if resp.clicked() { go(st, page); }
    }
}

// ---------- pages ----------

fn overview_page(ui: &mut egui::Ui, app: &App, p: &Pal) {
    let mut s = ();
    two_col(ui, &mut s, |ui, _| {
        card(ui, p, "\u{E7F4}", "Game", |ui| {
            let t = app.clock.now() as i64;
            field(ui, p, "Status", |ui| {
                let (dot, text) = if app.clock.is_live { (p.ready, format!("In game {}:{:02}", t / 60, t % 60)) } else { (p.muted, "No game detected".to_string()) };
                ui.label(RichText::new("\u{25CF}").size(11.0).color(dot));
                ui.label(RichText::new(text).size(13.5));
            });
            field(ui, p, "Enemies", |ui| {
                let names: Vec<&str> = app.tracker.enemies.iter().map(|e| e.name.as_str()).collect();
                let text = if names.is_empty() { "None yet".to_string() } else { names.join(", ") };
                ui.add(egui::Label::new(RichText::new(text).size(13.5)).wrap());
            });
            let active = app.tracker.enemies.iter().map(|e| e.cooldowns.values().filter(|c| c.ready_at > app.clock.now()).count()).sum::<usize>();
            field(ui, p, "On cooldown", |ui| { ui.label(RichText::new(format!("{active} summoner spells")).size(13.5)); });
        });
    }, |ui, _| {
        card(ui, p, "\u{E765}", "Main hotkeys", |ui| {
            for (action, label) in [("CopySummary", "Copy message"), ("TypeInChat", "Type in chat"), ("Undo", "Undo"), ("ToggleOverlay", "Show / hide overlay")] {
                let combo = app.settings.hotkeys.get(action).cloned().filter(|c| !c.is_empty()).unwrap_or_else(|| "—".into());
                field(ui, p, label, |ui| { ui.label(RichText::new(combo).font(theme::mono(12.5)).color(p.accent_deep)); });
            }
        });
    });
}

fn about_page(ui: &mut egui::Ui, app: &App, p: &Pal) {
    card(ui, p, "\u{E946}", "Flashless", |ui| {
        field(ui, p, "Version", |ui| { ui.label(RichText::new(env!("CARGO_PKG_VERSION")).size(13.5)); });
        field(ui, p, "Game data", |ui| { ui.label(RichText::new(format!("Patch {}", app.data.version)).size(13.5)); });
        hint(ui, p, "A free League of Legends companion overlay that tracks enemy summoner spell cooldowns from the game's own Live Client data, chat and your clicks. It does not read game memory and does not track ultimates.");
        ui.horizontal(|ui| {
            if outline(ui, p, icon_label("\u{E8A7}", "Open on GitHub", 13.0, p.text), 0.0, 32.0).clicked() { win::open_url(&format!("https://github.com/{}", crate::update::REPO)); }
        });
        hint(ui, p, "Flashless isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or anyone officially involved in producing or managing Riot Games properties. Riot Games and all associated properties are trademarks or registered trademarks of Riot Games, Inc.");
    });
}

struct OverlayPage<'a> { app: &'a mut App, st: &'a mut SetupState, ctx: &'a egui::Context }

fn overlay_page(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut App, st: &mut SetupState) {
    let mut s = OverlayPage { app, st, ctx };
    two_col(ui, &mut s, |ui, s| {
        let p = s.app.pal;
        let st = &mut *s.st;
        card(ui, &p, "\u{E7F4}", "Overlay Appearance", |ui| {
            field(ui, &p, "Theme", |ui| {
                let opts = [("System".to_string(), "System"), ("Light".to_string(), "Light"), ("Dark".to_string(), "Dark")];
                let w = ((ui.available_width() - 4.0) / 3.0).clamp(64.0, 92.0);
                if theme::segmented(ui, &p, &mut st.draft.theme, &opts, w) {
                    s.app.pal = theme::resolve(&st.draft.theme); // live preview; Cancel restores the saved theme
                    theme::apply(s.ctx, &s.app.pal);
                }
            });
            field(ui, &p, "Opacity", |ui| {
                let w = (ui.available_width() - 52.0).max(80.0);
                slider(ui, &p, &mut st.draft.opacity, 0.3..=1.0, w);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{:.0}%", st.draft.opacity * 100.0)).size(13.5));
                });
            });
            divider(ui, &p);
            theme::check(ui, &mut st.draft.always_on_top, "Keep the overlay on top of other windows");
            theme::check(ui, &mut st.draft.show_preview, "Show the chat message preview in the overlay");
            theme::check(ui, &mut st.draft.show_portraits, "Show champion portraits");
            theme::check(ui, &mut st.draft.compact, "Compact mode (lane and spell icons only)");
            theme::check(ui, &mut st.draft.auto_compact, "Switch to compact mode during a game");
            theme::check(ui, &mut st.draft.hide_outside_game, "Hide the overlay when no game is running");
            theme::check(ui, &mut st.draft.click_through_in_game, "Click through the overlay while in-game")
                .on_hover_text("Clicks pass to League while it is in front. Use the hotkeys, or Alt+Tab to the overlay to edit.");
        });
        card(ui, &p, "\u{E81E}", "Spell Order", |ui| {
            let name = |o: SpellOrder| match o { SpellOrder::FlashLeft => "Flash  \u{2192}  Other spells", SpellOrder::FlashRight => "Other spells  \u{2192}  Flash", SpellOrder::Game => "In-game order (D / F)" };
            egui::ComboBox::from_id_salt("spell-order").width(ui.available_width() - 8.0).height(200.0)
                .selected_text(RichText::new(name(st.draft.spell_order)).size(13.5)).show_ui(ui, |ui| {
                    for o in [SpellOrder::FlashLeft, SpellOrder::FlashRight, SpellOrder::Game] { ui.selectable_value(&mut st.draft.spell_order, o, name(o)); }
                });
        });
    }, |ui, s| {
        let p = s.app.pal;
        card(ui, &p, "\u{E7B3}", "Overlay Preview", |ui| {
            sample_overlay(ui, s.app, s.st);
            hint(ui, &p, "Sample data, not your game.");
            section(ui, &p, "\u{E945}", "Behavior");
            let st = &mut *s.st;
            theme::toggle(ui, &mut st.draft.flash_alert, RichText::new("Pulse an enemy's Flash when it comes back up").size(13.5));
            ui.add_enabled_ui(st.draft.flash_alert, |ui| { theme::toggle(ui, &mut st.draft.flash_alert_sound, RichText::new("Also play the Windows notification sound").size(13.5)); });
            theme::toggle(ui, &mut st.draft.check_updates, RichText::new("Check for updates when Flashless starts").size(13.5));
            section(ui, &p, "\u{E8D7}", "Riot API (Optional)");
            riot_section(ui, s.app, s.st, &p);
        });
    });
}

/// The overlay's enemy list drawn from the sample tracker with the draft options.
fn sample_overlay(ui: &mut egui::Ui, app: &App, st: &SetupState) {
    let p = app.pal;
    let d = &st.draft;
    let t = &st.sample;
    let slots = |i: usize| {
        let e = &t.enemies[i];
        let is_flash = |s| app.data.find_spell(e.spell_id(s)).is_some_and(|x| x.name == "Flash");
        d.spell_order.slots(is_flash(Slot::Spell1), is_flash(Slot::Spell2))
    };
    let look = |i: usize, slot: Slot| {
        let e = &t.enemies[i];
        let spell = app.data.find_spell(e.spell_id(slot)).map(|s| s.name.clone()).unwrap_or_default();
        let cd = e.cooldowns.get(&slot).copied();
        let remaining = if cd.is_some() { t.remaining(i, slot, SAMPLE_NOW) } else { 0.0 };
        let frac = cd.filter(|c| c.duration() > 0.0).map(|c| (remaining / c.duration()) as f32).unwrap_or(0.0);
        (d.message.abbreviate(&spell), spell, remaining, frac, cd.map(|c| c.ready_at))
    };
    let portraits = d.show_portraits;
    ui.scope(|ui| {
        ui.set_opacity(d.opacity.clamp(0.3, 1.0));
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        if d.compact {
            let w = app::mini_width(portraits);
            egui::Frame::none().fill(p.surface).stroke(Stroke::new(1.0_f32, p.line_strong)).rounding(9.0_f32).inner_margin(8.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, app::MINI_GAP);
                for i in 0..t.enemies.len() {
                    let e = &t.enemies[i];
                    let row = ui.allocate_exact_size(egui::vec2(w - 16.0, app::MINI_ICON), Sense::hover()).0;
                    let x0 = app::paint_mini_label(ui.painter(), &p, row, &e.champion_id, &e.name, e.role.as_deref(), portraits);
                    for (k, slot) in slots(i).iter().enumerate() {
                        let (abbr, spell, remaining, frac, ready_at) = look(i, *slot);
                        let tl = app::TileLook { spell: &spell, abbr, has_data: true, remaining, frac, ready_at, hovered: false, pulse: None };
                        let r = Rect::from_min_size(egui::pos2(x0 + k as f32 * (app::MINI_ICON + app::MINI_GAP), row.top()), egui::vec2(app::MINI_ICON, app::MINI_ICON));
                        app::paint_tile(ui.painter(), &p, r, &tl, true);
                    }
                }
            });
            return;
        }
        // the overlay's own rows, as wide as the overlay when there is room (the name narrows when there is not)
        let row_w = (ui.available_width() - 24.0).min(app::card_width(portraits) - 24.0);
        let name_w = row_w - 2.0 * (app::TILE.x + app::TILE_GAP);
        egui::Frame::none().fill(p.surface).stroke(Stroke::new(1.0_f32, p.line_strong)).rounding(10.0_f32).inner_margin(12.0).show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
            for i in 0..t.enemies.len() {
                let e = &t.enemies[i];
                let row = ui.allocate_exact_size(egui::vec2(row_w, app::TILE.y), Sense::hover()).0;
                let bits: Vec<(String, Color32)> = e.role.iter().map(|r| (r.clone(), p.accent_deep)).collect();
                let name = app::NameLook { index: i, champion_id: &e.champion_id, name: &e.name, bits, hovered: false };
                app::paint_name(ui.painter(), &p, Rect::from_min_size(row.min, egui::vec2(name_w, app::TILE.y)), &name, portraits);
                for (k, slot) in slots(i).iter().enumerate() {
                    let (abbr, spell, remaining, frac, ready_at) = look(i, *slot);
                    let tl = app::TileLook { spell: &spell, abbr, has_data: true, remaining, frac, ready_at, hovered: false, pulse: None };
                    let r = Rect::from_min_size(egui::pos2(row.right() - (2 - k) as f32 * (app::TILE.x + app::TILE_GAP) + app::TILE_GAP, row.top()), app::TILE);
                    app::paint_tile(ui.painter(), &p, r, &tl, false);
                }
            }
            if d.show_preview {
                ui.add_space(2.0);
                app::chat_preview(ui, &p, row_w, &sample_message(app, st));
            }
        });
    });
}

fn sample_message(app: &App, st: &SetupState) -> String {
    crate::message::build(&st.sample.enemies, &app.data, SAMPLE_NOW, true, &st.draft.message)
}

fn message_page(ui: &mut egui::Ui, app: &App, st: &mut SetupState, p: &Pal) {
    two_col(ui, st, |ui, st| {
        card(ui, p, "\u{E8BD}", "Message Format", |ui| {
            field(ui, p, "Times", |ui| {
                let opts = [(TimeFormat::GameClock, "Game clock"), (TimeFormat::SecondsRemaining, "Seconds left")];
                theme::segmented(ui, p, &mut st.draft.message.format, &opts, 110.0);
            });
            hint(ui, p, if st.draft.message.format == TimeFormat::GameClock { "Game time each spell is back up: 2230 = 22:30. Stays correct after pasting." } else { "Seconds left when the message is built." });
            divider(ui, p);
            theme::check(ui, &mut st.draft.message.use_roles, "Name enemies by lane (Top, Jg, Mid, ADC, Sup)");
            theme::check(ui, &mut st.draft.message.longest_first, "Other spells: longest cooldown first");
            hint(ui, p, "Always one line. Flash leads: enemies are ordered by when their Flash is back.");
        });
    }, |ui, st| {
        card(ui, p, "\u{E7B3}", "Example", |ui| {
            let w = ui.available_width();
            app::chat_preview(ui, p, w, &sample_message(app, st));
            hint(ui, p, "Sample enemies at 14:00 game time.");
        });
    });
}

fn roster(ui: &mut egui::Ui, app: &mut App, st: &mut SetupState, p: &Pal) {
    let spells: Vec<(String, String)> = app.data.standard_spells().into_iter().map(|s| (s.id.clone(), s.name.clone())).collect();
    card(ui, p, "\u{E716}", "Enemy Roster", |ui| {
        hint(ui, p, &st.roster_note);
        ui.spacing_mut().item_spacing.y = 8.0;
        for (i, row) in st.rows.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized([40.0, 28.0], egui::Label::new(RichText::new(ROLES[i]).font(theme::semi(13.0)).color(p.accent_deep)));
                ui.add(egui::TextEdit::singleline(&mut row.champ).hint_text("Champion").desired_width(180.0).margin(egui::vec2(8.0, 5.0)));
                for (k, sel) in [&mut row.s1, &mut row.s2].into_iter().enumerate() {
                    let current = sel.as_deref().and_then(|id| spells.iter().find(|(i2, _)| i2 == id)).map(|(_, n)| n.clone()).unwrap_or("—".into());
                    egui::ComboBox::from_id_salt(("spell", i, k)).width(110.0).selected_text(current).show_ui(ui, |ui| {
                        for (id, name) in &spells { ui.selectable_value(sel, Some(id.clone()), name); }
                    });
                }
                ui.label(RichText::new("Level").size(12.5).color(p.secondary));
                ui.add(egui::DragValue::new(&mut row.level).range(1..=18).speed(0.1));
            });
        }
        if button(ui, p, "Apply roster").clicked() {
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

fn clipboard_card(ui: &mut egui::Ui, st: &mut SetupState, p: &Pal) {
    card(ui, p, "\u{E8C8}", "Clipboard & Typing", |ui| {
        theme::check(ui, &mut st.draft.auto_copy, "Copy the message automatically when a cooldown is registered");
        field(ui, p, "Copy delay", |ui| { ui.add(egui::DragValue::new(&mut st.draft.copy_delay_ms).range(0..=5000).speed(5.0).suffix(" ms")); });
        field(ui, p, "Call delay", |ui| { ui.add(egui::DragValue::new(&mut st.draft.reaction_offset_seconds).range(0.0..=30.0).speed(0.5).suffix(" s")); })
            .on_hover_text("Seconds a spell is assumed to have been used before it was tracked. Applies to clicks, hotkeys and chat detection. 7 s turns a fresh Flash from 5:00 into 4:53.");
        divider(ui, p);
        theme::check(ui, &mut st.draft.copy_in_chat_after_typing, "After typing, select and copy the line in the chat box");
        hint(ui, p, "The Type In Chat hotkey (Ctrl+Alt+V by default) opens chat and types the message, but does not send it: you press Enter. It simulates key presses while League is focused, only when you press the hotkey. Riot does not sanction input automation; use it at your own risk.");
    });
}

fn chat_card(ui: &mut egui::Ui, ctx: &egui::Context, st: &mut SetupState, p: &Pal) {
    card(ui, p, "\u{E8BD}", "Chat Reading", |ui| {
        theme::toggle(ui, &mut st.draft.chat_detection, RichText::new("Register \"Champion used Spell\" lines from chat").size(13.5));
        hint(ui, p, "Captures the chat area of the game window about once a second and reads it with the OCR built into Windows. Needs Borderless or Windowed mode and chat timestamps on.");
        ui.horizontal_wrapped(|ui| {
            if button(ui, p, "Draw area").on_hover_text("Drag a box around the chat over the game window").clicked() {
                let ppp = ctx.pixels_per_point();
                let bounds = win::game_window().and_then(win::window_rect)
                    .map(|r| Rect::from_min_max(egui::pos2(r.left as f32 / ppp, r.top as f32 / ppp), egui::pos2(r.right as f32 / ppp, r.bottom as f32 / ppp)))
                    .or_else(|| ctx.input(|i| i.viewport().monitor_size).map(|s| Rect::from_min_size(egui::Pos2::ZERO, s)))
                    .unwrap_or(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0)));
                st.picker = Some(Picker { bounds, start: None, sel: None });
            }
            let detecting = st.countdown.is_some() || st.detect_rx.is_some();
            let auto = ui.add_enabled(!detecting, egui::Button::new(RichText::new("Auto-detect").size(13.0)).fill(Color32::TRANSPARENT)
                .stroke(Stroke::new(1.0_f32, p.line_strong)).rounding(7.0_f32).min_size(egui::vec2(0.0, 30.0)));
            if auto.on_hover_text("Finds the chat from its timestamps. Open the chat history in game during the countdown.").clicked() {
                if win::game_window().is_none() { st.area_note = "League is not running.".into(); } else { st.countdown = Some(Instant::now() + Duration::from_secs(5)); }
            }
            if button(ui, p, "Reset").clicked() { st.draft.chat_region = DEFAULT_CHAT_REGION; st.area_note = "Reset to default.".into(); }
        });
        let text = if st.area_note.is_empty() { region_text(st.draft.chat_region) } else { format!("{}  {}", st.area_note, region_text(st.draft.chat_region)) };
        hint(ui, p, &text);
        divider(ui, p);
        theme::check(ui, &mut st.draft.chat_log, "Keep a log of what was read (last three games)");
    });
}

/// Auto-detect countdown and result; runs every frame so it finishes even on another page.
fn poll_chat_detect(st: &mut SetupState) {
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
}

/// Optional: your own Riot API key (stored encrypted) and region, for reading enemy rune pages.
fn riot_section(ui: &mut egui::Ui, app: &mut App, st: &mut SetupState, p: &Pal) {
    hint(ui, p, "Used to read enemy runes (Cosmic Insight) when a game starts, so it is known instead of guessed. The key is stored encrypted for your Windows account only and never shown again.");
    let link = ui.add(egui::Label::new(icon_label("\u{E8A7}", "Get a key at developer.riotgames.com", 12.5, p.accent_deep)).sense(Sense::click()))
        .on_hover_text(RIOT_DEV_URL).on_hover_cursor(egui::CursorIcon::PointingHand);
    if link.clicked() { win::open_url(RIOT_DEV_URL); }
    let has_key = crate::riot::has_key();
    ui.horizontal(|ui| {
        ui.add_sized([60.0, 32.0], egui::Label::new(RichText::new("Region").size(13.5)).halign(egui::Align::LEFT));
        egui::ComboBox::from_id_salt("riot-region").width(120.0).selected_text(RichText::new(st.draft.riot_region.clone()).size(13.5)).show_ui(ui, |ui| {
            for (code, _, _) in crate::riot::REGIONS { ui.selectable_value(&mut st.draft.riot_region, code.to_string(), *code); }
        });
        if has_key {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if outline(ui, p, icon_label("\u{E8A7}", "Look up runes now", 13.0, p.text), 0.0, 32.0).on_hover_text("Needs a live game (not bot or practice games)").clicked() {
                    app.riot_lookup_now = true;
                    st.key_note = "Looking up... see the overlay.".into();
                }
            });
        }
    });
    if has_key {
        ui.horizontal(|ui| {
            ui.label(RichText::new("\u{E73E}").font(theme::icon(13.0)).color(p.ready));
            ui.label(RichText::new("API key saved (encrypted)").size(13.5).color(p.ready));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if outline(ui, p, RichText::new("Remove key").size(13.0), 0.0, 32.0).clicked() { crate::riot::remove_key(); st.key_note = "Key removed.".into(); }
            });
        });
    } else {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut st.key_input).password(true).hint_text("RGAPI-...").desired_width((ui.available_width() - 100.0).max(120.0)).margin(egui::vec2(8.0, 6.0)));
            if button(ui, p, "Save key").clicked() {
                st.key_note = match crate::riot::save_key(&st.key_input) { Ok(()) => "Key saved.".into(), Err(e) => format!("Could not save: {e}") };
                st.key_input.clear();
            }
        });
    }
    if !st.key_note.is_empty() { hint(ui, p, &st.key_note); }
}

/// Captures a combination while a hotkey box is listening; runs every frame.
fn capture_hotkey(ctx: &egui::Context, st: &mut SetupState) {
    let Some(action) = st.listening.clone() else { return };
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

fn hotkeys_card(ui: &mut egui::Ui, st: &mut SetupState, p: &Pal) {
    card(ui, p, "\u{E765}", "Hotkeys", |ui| {
        hint(ui, p, "Click a box, then press a key combination or mouse button 3, 4 or 5 (Ctrl/Alt/Shift work with both). Esc or Backspace clears it.");
        let mut names: Vec<String> = st.draft.hotkeys.keys().cloned().collect();
        let order = |n: &String| match n.as_str() { "CopySummary" => 0, "TypeInChat" => 1, "Undo" => 2, "ResetAll" => 3, "ToggleOverlay" => 4, _ => 10 };
        names.sort_by_key(|n| (order(n), n.clone()));
        egui::Grid::new("hotkeys").num_columns(4).spacing([14.0, 8.0]).min_col_width(0.0).show(ui, |ui| {
            for (k, action) in names.iter().enumerate() {
                ui.add_sized([130.0, 30.0], egui::Label::new(RichText::new(hotkey_label(action)).size(13.5)).halign(egui::Align::LEFT));
                let listening = st.listening.as_deref() == Some(action.as_str());
                let combo = st.draft.hotkeys.get(action).cloned().unwrap_or_default();
                let (text, col) = if listening { ("Key or mouse...".to_string(), p.accent_deep) } else if combo.is_empty() { ("—".to_string(), p.muted) } else { (combo, p.text) };
                let b = egui::Button::new(RichText::new(text).size(13.0).color(col)).min_size(egui::vec2(150.0, 30.0)).rounding(7.0_f32)
                    .fill(if listening { p.accent_soft } else { Color32::TRANSPARENT }).stroke(Stroke::new(1.0_f32, if listening { p.accent } else { p.line_strong }));
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
