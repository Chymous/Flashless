//! Summoner spell icons and champion portraits (Riot Data Dragon art, used under Riot's fan-content policy),
//! embedded as raw RGBA at build time (no downloads at runtime), and the League-style cooldown pie drawn over them.

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
    let tex = ctx.load_texture(format!("spell-{id}"), egui::ColorImage::from_rgba_unmultiplied([64, 64], bytes), super::theme::SMOOTH);
    let tid = tex.id();
    cache.insert(id, tex);
    Some(tid)
}

/// Champion portraits: see assets/fetch_champion_portraits.py for the format.
const PORTRAITS: &[u8] = include_bytes!("../../assets/champions.bin");
pub const PORTRAIT_PX: usize = 40;

/// RGBA bytes of a champion's portrait by Data Dragon id ("LeeSin"), case-insensitive.
pub fn portrait_rgba(champion_id: &str) -> Option<&'static [u8]> {
    let len = PORTRAIT_PX * PORTRAIT_PX * 4;
    let mut i = 0;
    while i < PORTRAITS.len() {
        let n = PORTRAITS[i] as usize;
        let id = PORTRAITS.get(i + 1..i + 1 + n)?;
        let px = PORTRAITS.get(i + 1 + n..i + 1 + n + len)?;
        if id.eq_ignore_ascii_case(champion_id.as_bytes()) { return Some(px); }
        i += 1 + n + len;
    }
    None
}

/// Texture for a champion portrait. Loaded once per champion; None for champions newer than the build.
pub fn portrait_texture(ctx: &egui::Context, champion_id: &str) -> Option<egui::TextureId> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<egui::TextureHandle>>>> = OnceLock::new();
    let key = champion_id.to_ascii_lowercase();
    let mut cache = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock().ok()?;
    if let Some(t) = cache.get(&key) { return t.as_ref().map(|t| t.id()); }
    let tex = portrait_rgba(champion_id).map(|px| ctx.load_texture(format!("champ-{key}"),
        egui::ColorImage::from_rgba_unmultiplied([PORTRAIT_PX, PORTRAIT_PX], px), super::theme::SMOOTH));
    let id = tex.as_ref().map(|t| t.id());
    cache.insert(key, tex);
    id
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_snapshot_champion_has_a_portrait() {
        let d = crate::data::GameData::embedded();
        for c in &d.champions { assert!(portrait_rgba(&c.id).is_some(), "{}", c.id); }
        assert!(portrait_rgba("leesin").is_some());
        assert!(portrait_rgba("NotAChampion").is_none());
        assert!(portrait_rgba("").is_none());
    }
}
