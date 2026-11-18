//! Light ("white and gold") and dark ("deep brown and gold") palettes, fonts, and egui visuals.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Rounding, Stroke};

#[derive(Clone, Copy)]
pub struct Pal {
    pub dark: bool,
    pub bg: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub inset: Color32,
    pub line: Color32,
    pub line_strong: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_text: Color32,
    pub accent_deep: Color32,
    pub ready: Color32,
    pub cooling: Color32,
    pub cooling_line: Color32,
    pub shadow: Color32,
}

const fn c(hex: u32) -> Color32 { Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8) }

pub const LIGHT: Pal = Pal {
    dark: false,
    bg: c(0xFFFFFF), surface: c(0xFBF8F1), surface_hover: c(0xF4ECDA), inset: c(0xFAF5E9),
    line: c(0xEEE5D2), line_strong: c(0xDCCBA3), text: c(0x2A2418), muted: c(0x877A63),
    accent: c(0xC9A13B), accent_soft: c(0xF7EDD3), accent_text: c(0x2B2006), accent_deep: c(0x9A771A),
    ready: c(0x2E8F62), cooling: c(0xA06F12), cooling_line: c(0xE2C57C),
    shadow: Color32::from_rgba_premultiplied(40, 32, 16, 60),
};

pub const DARK: Pal = Pal {
    dark: true,
    bg: c(0x15120E), surface: c(0x1E1A15), surface_hover: c(0x28221B), inset: c(0x100D0A),
    line: c(0x2B251E), line_strong: c(0x463A27), text: c(0xEEE6D6), muted: c(0x9C8F79),
    accent: c(0xD4AF37), accent_soft: c(0x2F2715), accent_text: c(0x1A1407), accent_deep: c(0xE2C15E),
    ready: c(0x5FC593), cooling: c(0xEDBE4F), cooling_line: c(0x6E5521),
    shadow: Color32::from_rgba_premultiplied(0, 0, 0, 110),
};

pub fn resolve(mode: &str) -> Pal {
    match mode {
        "Light" => LIGHT,
        "Dark" => DARK,
        _ => if crate::win::system_prefers_dark() { DARK } else { LIGHT },
    }
}

pub fn rgb(c: Color32) -> [u8; 3] { [c.r(), c.g(), c.b()] }

/// Segoe UI / Semibold / Consolas / Segoe icons from the Windows font folder (nothing is bundled).
pub fn install_fonts(ctx: &egui::Context) {
    let dir = std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts");
    let load = |f: &str| std::fs::read(dir.join(f)).ok();
    let mut defs = FontDefinitions::default();
    let mut add = |key: &str, files: &[&str]| -> bool {
        for f in files {
            if let Some(bytes) = load(f) {
                defs.font_data.insert(key.into(), FontData::from_owned(bytes));
                return true;
            }
        }
        false
    };
    let ui = add("ui", &["segoeui.ttf"]);
    let semi = add("semibold", &["seguisb.ttf", "segoeuib.ttf"]);
    let mono = add("mono", &["CascadiaMono.ttf", "consola.ttf"]);
    let icons = add("icons", &["SegoeIcons.ttf", "segmdl2.ttf"]);
    if ui { defs.families.entry(FontFamily::Proportional).or_default().insert(0, "ui".into()); }
    if mono { defs.families.entry(FontFamily::Monospace).or_default().insert(0, "mono".into()); }
    let mut semi_family = Vec::new();
    if semi { semi_family.push("semibold".to_string()); }
    if ui { semi_family.push("ui".to_string()); }
    semi_family.extend(defs.families[&FontFamily::Proportional].iter().cloned());
    defs.families.insert(FontFamily::Name("semibold".into()), semi_family);
    let mut icon_family = Vec::new();
    if icons { icon_family.push("icons".to_string()); }
    icon_family.extend(defs.families[&FontFamily::Proportional].iter().cloned());
    defs.families.insert(FontFamily::Name("icons".into()), icon_family);
    ctx.set_fonts(defs);
}

pub fn semi(size: f32) -> FontId { FontId::new(size, FontFamily::Name("semibold".into())) }
pub fn regular(size: f32) -> FontId { FontId::new(size, FontFamily::Proportional) }
pub fn icon(size: f32) -> FontId { FontId::new(size, FontFamily::Name("icons".into())) }
pub fn mono(size: f32) -> FontId { FontId::new(size, FontFamily::Monospace) }

pub fn apply(ctx: &egui::Context, p: &Pal) {
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.override_text_color = Some(p.text);
    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.window_stroke = Stroke::new(1.0_f32, p.line_strong);
    v.window_rounding = Rounding::same(8.0);
    v.menu_rounding = Rounding::same(6.0);
    v.extreme_bg_color = p.surface;
    v.faint_bg_color = p.surface;
    v.code_bg_color = p.inset;
    v.hyperlink_color = p.accent_deep;
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.5_f32, p.accent_text);
    v.popup_shadow = egui::epaint::Shadow { offset: egui::vec2(0.0, 2.0), blur: 12.0, spread: 0.0, color: p.shadow };
    v.window_shadow = v.popup_shadow;
    let r = Rounding::same(6.0);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.noninteractive.rounding = r;
    w.inactive.bg_fill = p.surface;
    w.inactive.weak_bg_fill = p.surface;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, p.line_strong);
    w.inactive.fg_stroke = Stroke::new(1.5_f32, p.accent_deep);
    w.inactive.rounding = r;
    w.hovered.bg_fill = p.surface_hover;
    w.hovered.weak_bg_fill = p.surface_hover;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent);
    w.hovered.fg_stroke = Stroke::new(1.5_f32, p.accent_deep);
    w.hovered.rounding = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = p.accent_soft;
    w.active.weak_bg_fill = p.accent_soft;
    w.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    w.active.fg_stroke = Stroke::new(1.5_f32, p.accent_deep);
    w.active.rounding = r;
    w.active.expansion = 0.0;
    w.open = w.active;
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.icon_width = 15.0;
        s.spacing.icon_spacing = 7.0;
        s.text_styles.insert(egui::TextStyle::Body, regular(13.0));
        s.text_styles.insert(egui::TextStyle::Button, regular(12.5));
        s.text_styles.insert(egui::TextStyle::Small, regular(11.0));
        s.text_styles.insert(egui::TextStyle::Monospace, mono(12.5));
        s.text_styles.insert(egui::TextStyle::Heading, semi(20.0));
    });
}

const ICON_128: &[u8] = include_bytes!("../../assets/icon_128.rgba");
const ICON_64: &[u8] = include_bytes!("../../assets/icon_64.rgba");

/// The brand mark: the app icon (a gold burst with a cooldown pie), drawn from an embedded texture.
pub fn brand(painter: &egui::Painter, center: egui::Pos2, radius: f32, _p: &Pal) {
    static TEX: std::sync::OnceLock<egui::TextureHandle> = std::sync::OnceLock::new();
    let tex = TEX.get_or_init(|| painter.ctx().load_texture("flashless-icon", egui::ColorImage::from_rgba_unmultiplied([128, 128], ICON_128), egui::TextureOptions::LINEAR));
    let rect = egui::Rect::from_center_size(center, egui::vec2(radius * 2.0, radius * 2.0));
    painter.image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
}

/// Window icon (taskbar / Alt+Tab).
pub fn icon_data() -> egui::IconData {
    egui::IconData { rgba: ICON_64.to_vec(), width: 64, height: 64 }
}
