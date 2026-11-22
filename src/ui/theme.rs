//! Palettes (charcoal and muted gold, plus a warm light variant), fonts, egui visuals and the few custom widgets
//! (checkbox, toggle, segmented control) shared by the overlay, Settings and the installer.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Rounding, Stroke};

#[derive(Clone, Copy)]
pub struct Pal {
    pub dark: bool,
    /// Window background.
    pub bg: Color32,
    /// Settings sidebar.
    pub sidebar: Color32,
    /// Cards and the overlay body.
    pub surface: Color32,
    /// Controls and tiles sitting on a surface.
    pub elevated: Color32,
    pub surface_hover: Color32,
    /// Recessed areas (chat message preview, details).
    pub inset: Color32,
    pub line: Color32,
    pub line_strong: Color32,
    pub text: Color32,
    /// Labels and hints.
    pub secondary: Color32,
    /// Least important text (version, footnotes, idle icons).
    pub muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_muted: Color32,
    pub accent_soft: Color32,
    pub accent_text: Color32,
    /// Gold for text and small marks.
    pub accent_deep: Color32,
    pub ready: Color32,
    pub cooling: Color32,
    pub cooling_line: Color32,
    pub shadow: Color32,
}

const fn c(hex: u32) -> Color32 { Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8) }

pub const LIGHT: Pal = Pal {
    dark: false,
    bg: c(0xF5F4F1), sidebar: c(0xECEAE5), surface: c(0xFFFFFF), elevated: c(0xF6F5F2), surface_hover: c(0xEEECE7), inset: c(0xF8F7F4),
    line: c(0xE4E1DA), line_strong: c(0xD2CEC5), text: c(0x1C1B19), secondary: c(0x5D5A54), muted: c(0x8D8981),
    accent: c(0xC9A13F), accent_hover: c(0xB8902F), accent_muted: c(0xD8C186), accent_soft: c(0xF4EAD2), accent_text: c(0x1E1708),
    accent_deep: c(0x8C6B16), ready: c(0x2C8A4E), cooling: c(0x996A0D), cooling_line: c(0xDCC17A),
    shadow: Color32::from_rgba_premultiplied(20, 18, 12, 40),
};

/// Near-black charcoal with warm gold, sampled from the reference design.
pub const DARK: Pal = Pal {
    dark: true,
    bg: c(0x0F0F11), sidebar: c(0x131215), surface: c(0x161618), elevated: c(0x19191B), surface_hover: c(0x222224), inset: c(0x111113),
    line: c(0x2C2C2F), line_strong: c(0x4A4A4C), text: c(0xEDEDEB), secondary: c(0xAAAAAC), muted: c(0x8A8A8C),
    accent: c(0xD4AE64), accent_hover: c(0xE2BE76), accent_muted: c(0x9C8045), accent_soft: c(0x3A3323), accent_text: c(0x17130B),
    accent_deep: c(0xE3C98C), ready: c(0x86C67C), cooling: c(0xD2D2D4), cooling_line: c(0x8A7038),
    shadow: Color32::from_rgba_premultiplied(0, 0, 0, 130),
};

pub fn resolve(mode: &str) -> Pal {
    match mode {
        "Light" => LIGHT,
        "Dark" => DARK,
        _ => if crate::win::system_prefers_dark() { DARK } else { LIGHT },
    }
}

pub fn rgb(c: Color32) -> [u8; 3] { [c.r(), c.g(), c.b()] }

/// Segoe UI Variable (Windows 11, else Segoe UI) / Semibold / Cascadia or Consolas / Segoe icons from the Windows
/// font folder (nothing is bundled). egui's built-in fonts stay as the fallback.
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
    let ui = add("ui", &["SegUIVar.ttf", "segoeui.ttf"]);
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
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0_f32, p.line_strong);
    v.window_rounding = Rounding::same(8.0);
    v.menu_rounding = Rounding::same(6.0);
    v.extreme_bg_color = p.inset;
    v.faint_bg_color = p.surface;
    v.code_bg_color = p.inset;
    v.hyperlink_color = p.accent_deep;
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.5_f32, p.accent_text);
    v.popup_shadow = egui::epaint::Shadow { offset: egui::vec2(0.0, 2.0), blur: 10.0, spread: 0.0, color: p.shadow };
    v.window_shadow = v.popup_shadow;
    v.slider_trailing_fill = true;
    let r = Rounding::same(6.0);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.noninteractive.rounding = r;
    w.inactive.bg_fill = p.line_strong; // slider rail, radio fill
    w.inactive.weak_bg_fill = p.elevated; // buttons, combo boxes
    w.inactive.bg_stroke = Stroke::new(1.0_f32, p.line_strong);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.inactive.rounding = r;
    w.hovered.bg_fill = p.accent_hover;
    w.hovered.weak_bg_fill = p.surface_hover;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent_muted);
    w.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.hovered.rounding = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = p.accent;
    w.active.weak_bg_fill = p.accent_soft;
    w.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    w.active.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.active.rounding = r;
    w.active.expansion = 0.0;
    w.open = w.active;
    w.open.weak_bg_fill = p.elevated;
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

/// Linear filtering with mipmaps: images drawn much smaller than their source stay clean instead of blurry.
pub const SMOOTH: egui::TextureOptions = egui::TextureOptions {
    magnification: egui::TextureFilter::Linear,
    minification: egui::TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: Some(egui::TextureFilter::Linear),
};

const ICON_128: &[u8] = include_bytes!("../../assets/icon_128.rgba");
const ICON_64: &[u8] = include_bytes!("../../assets/icon_64.rgba");

/// The brand mark: the app icon (a gold burst with a cooldown pie), drawn from an embedded texture.
pub fn brand(painter: &egui::Painter, center: egui::Pos2, radius: f32, _p: &Pal) {
    static TEX: std::sync::OnceLock<egui::TextureHandle> = std::sync::OnceLock::new();
    let tex = TEX.get_or_init(|| painter.ctx().load_texture("flashless-icon", egui::ColorImage::from_rgba_unmultiplied([128, 128], ICON_128), SMOOTH));
    // whole physical pixels, so the small mark stays sharp
    let ppp = painter.ctx().pixels_per_point();
    let side = (radius * 2.0 * ppp).round() / ppp;
    let min = ((center - egui::vec2(side, side) / 2.0) * ppp).round() / ppp;
    let rect = egui::Rect::from_min_size(min, egui::vec2(side, side));
    painter.image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
}

/// Window icon (taskbar / Alt+Tab).
pub fn icon_data() -> egui::IconData {
    egui::IconData { rgba: ICON_64.to_vec(), width: 64, height: 64 }
}

/// Checkbox: a small rounded square, gold with a dark tick when on (replaces egui's tick box everywhere).
pub fn check(ui: &mut egui::Ui, on: &mut bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    let d = 16.0;
    let gap = 10.0;
    let galley = text.into().into_galley(ui, Some(egui::TextWrapMode::Wrap), (ui.available_width() - d - gap).max(20.0), egui::TextStyle::Body);
    let size = egui::vec2(d + gap + galley.size().x, galley.size().y.max(d));
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() { *on = !*on; resp.mark_changed(); }
    let checked = *on;
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), checked, galley.text()));
    if ui.is_rect_visible(rect) {
        let vis = ui.visuals();
        let b = egui::Rect::from_center_size(egui::pos2(rect.left() + d / 2.0, rect.center().y), egui::vec2(d, d));
        let p = ui.painter();
        if checked {
            p.rect_filled(b, 3.5_f32, vis.selection.bg_fill);
            let ink = Stroke::new(1.8_f32, vis.selection.stroke.color);
            let o = b.left_top();
            let (a, m, z) = (o + egui::vec2(4.0, 8.3), o + egui::vec2(6.8, 11.0), o + egui::vec2(12.0, 5.2));
            p.line_segment([a, m], ink);
            p.line_segment([m, z], ink);
        } else {
            let edge = if resp.hovered() { vis.widgets.hovered.bg_stroke.color } else { vis.widgets.inactive.bg_stroke.color };
            p.rect(b.shrink(0.5), 3.5_f32, Color32::TRANSPARENT, Stroke::new(1.2_f32, edge));
        }
        p.galley(egui::pos2(rect.left() + d + gap, rect.center().y - galley.size().y / 2.0), galley, vis.text_color());
    }
    resp
}

/// On/off switch with its label on the right.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    let sw = egui::vec2(34.0, 18.0);
    let gap = 10.0;
    let galley = text.into().into_galley(ui, Some(egui::TextWrapMode::Wrap), (ui.available_width() - sw.x - gap).max(20.0), egui::TextStyle::Body);
    let size = egui::vec2(sw.x + gap + galley.size().x, galley.size().y.max(sw.y));
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() { *on = !*on; resp.mark_changed(); }
    let checked = *on;
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), checked, galley.text()));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(resp.id, checked, 0.12);
        let vis = ui.visuals();
        let track = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.center().y - sw.y / 2.0), sw);
        let fill = if checked { vis.selection.bg_fill } else { vis.widgets.inactive.bg_stroke.color };
        let p = ui.painter();
        p.rect_filled(track, sw.y / 2.0, fill);
        if resp.hovered() { p.rect_stroke(track, sw.y / 2.0, Stroke::new(1.0_f32, vis.widgets.hovered.bg_stroke.color)); }
        let x = egui::lerp((track.left() + sw.y / 2.0)..=(track.right() - sw.y / 2.0), t);
        let knob = if checked { Color32::from_rgb(0xEF, 0xEF, 0xEB) } else { Color32::from_rgb(0xB7, 0xB6, 0xB2) };
        p.circle_filled(egui::pos2(x, track.center().y), sw.y / 2.0 - 2.5, knob);
        p.galley(egui::pos2(track.right() + gap, rect.center().y - galley.size().y / 2.0), galley, vis.text_color());
    }
    resp
}

/// Joined options with the selected one filled gold. Returns true when the value changed.
pub fn segmented<T: PartialEq + Clone>(ui: &mut egui::Ui, p: &Pal, value: &mut T, options: &[(T, &str)], seg_w: f32) -> bool {
    let h = 30.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(seg_w * options.len() as f32, h), egui::Sense::hover());
    ui.painter().rect_stroke(rect, 7.0_f32, Stroke::new(1.0_f32, p.line_strong));
    let sel_at = options.iter().position(|(v, _)| *v == *value);
    let mut changed = false;
    for (k, (v, label)) in options.iter().enumerate() {
        let cell = egui::Rect::from_min_size(rect.min + egui::vec2(k as f32 * seg_w, 0.0), egui::vec2(seg_w, h));
        let r = cell.shrink(3.0);
        let resp = ui.interact(r, ui.id().with(("seg", *label)), egui::Sense::click());
        let sel = sel_at == Some(k);
        // thin separator between two unselected neighbours
        if k > 0 && !sel && sel_at != Some(k - 1) {
            ui.painter().line_segment([egui::pos2(cell.left(), cell.top() + 8.0), egui::pos2(cell.left(), cell.bottom() - 8.0)], Stroke::new(1.0_f32, p.line_strong));
        }
        if sel { ui.painter().rect_filled(r, 5.0_f32, p.accent); } else if resp.hovered() { ui.painter().rect_filled(r, 5.0_f32, p.surface_hover); }
        let col = if sel { p.accent_text } else { p.text };
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, *label, if sel { semi(13.0) } else { regular(13.0) }, col);
        if resp.clicked() && !sel { *value = v.clone(); changed = true; }
    }
    changed
}
