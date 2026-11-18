//! Summoner spell icons (Riot Data Dragon art, used under Riot's fan-content policy), embedded as 64x64 RGBA,
//! and the League-style cooldown pie drawn over them.

use eframe::egui::{self, epaint, Color32, Pos2, Rect, Rounding, Stroke};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

macro_rules! icon {
    ($id:literal) => { ($id, include_bytes!(concat!("../../assets/spells/", $id, ".rgba")) as &[u8]) };
}

const ICONS: &[(&str, &[u8])] = &[
    icon!("SummonerFlash"), icon!("SummonerDot"), icon!("SummonerBarrier"), icon!("SummonerExhaust"), icon!("SummonerHeal"),
    icon!("SummonerHaste"), icon!("SummonerBoost"), icon!("SummonerTeleport"), icon!("SummonerSmite"), icon!("SummonerMana"),
    icon!("SummonerSnowball"),
];

/// Spell name (as in Data Dragon) to icon id. Mode variants share the base icon.
fn icon_for_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "Flash" => "SummonerFlash",
        "Ignite" => "SummonerDot",
        "Barrier" => "SummonerBarrier",
        "Exhaust" => "SummonerExhaust",
        "Heal" => "SummonerHeal",
        "Ghost" => "SummonerHaste",
        "Cleanse" => "SummonerBoost",
        "Teleport" => "SummonerTeleport",
        "Smite" => "SummonerSmite",
        "Clarity" => "SummonerMana",
        "Mark" => "SummonerSnowball",
        _ => return None,
    })
}

/// Texture for a summoner spell, by its display name. Loaded once per spell.
pub fn spell_texture(ctx: &egui::Context, spell_name: &str) -> Option<egui::TextureId> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, egui::TextureHandle>>> = OnceLock::new();
    let id = icon_for_name(spell_name)?;
    let mut cache = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock().ok()?;
    if let Some(t) = cache.get(id) { return Some(t.id()); }
    let bytes = ICONS.iter().find(|(k, _)| *k == id)?.1;
    let tex = ctx.load_texture(format!("spell-{id}"), egui::ColorImage::from_rgba_unmultiplied([64, 64], bytes), egui::TextureOptions::LINEAR);
    let tid = tex.id();
    cache.insert(id, tex);
    Some(tid)
}

/// Draws an icon with rounded corners; `tint` dims it (e.g. for spells with no data).
pub fn draw_icon(painter: &egui::Painter, tex: egui::TextureId, rect: Rect, rounding: f32, tint: Color32) {
    let mut shape = epaint::RectShape::filled(rect, Rounding::same(rounding), tint);
    shape.fill_texture_id = tex;
    shape.uv = Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0));
    painter.add(shape);
}

/// League-style cooldown pie: darkens the share still cooling down, sweeping clockwise from 12 o'clock,
/// with a thin bright edge where the sweep is.
pub fn draw_pie(painter: &egui::Painter, rect: Rect, remaining: f32, shade: Color32, edge: Color32) {
    let remaining = remaining.clamp(0.0, 1.0);
    if remaining <= 0.0 { return; }
    let p = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let c = rect.center();
    let r = rect.size().length(); // reaches past the corners; the clip rect trims it to the icon
    let start = -std::f32::consts::FRAC_PI_2 + (1.0 - remaining) * std::f32::consts::TAU;
    let end = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU;
    let steps = ((remaining * 64.0).ceil() as usize).max(2);
    let mut mesh = epaint::Mesh::default();
    mesh.colored_vertex(c, shade);
    for k in 0..=steps {
        let a = start + (end - start) * k as f32 / steps as f32;
        mesh.colored_vertex(c + egui::vec2(a.cos(), a.sin()) * r, shade);
    }
    for k in 1..=steps as u32 { mesh.add_triangle(0, k, k + 1); }
    p.add(egui::Shape::mesh(mesh));
    if remaining < 0.999 {
        let tip = c + egui::vec2(start.cos(), start.sin()) * r;
        p.line_segment([c, tip], Stroke::new(1.5_f32, edge));
    }
}

/// Text with a dark halo so it stays readable on top of an icon.
pub fn outlined_text(painter: &egui::Painter, pos: Pos2, text: &str, font: egui::FontId, color: Color32) {
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)] {
        painter.text(pos + egui::vec2(dx, dy), egui::Align2::CENTER_CENTER, text, font.clone(), Color32::from_black_alpha(220));
    }
    painter.text(pos, egui::Align2::CENTER_CENTER, text, font, color);
}
