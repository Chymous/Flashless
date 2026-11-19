//! Installer window, shown when the exe is named Flashless-Setup*.exe.

use super::theme::{self, Pal};
use crate::win::{self, shell};
use eframe::egui::{self, RichText, Stroke};
use std::sync::mpsc::{channel, Receiver};

/// Shown instead of the feature list when an older version is installed.
const WHATS_NEW: &[&str] = &[
    "Optional Riot API key (Settings): enemy runes are read when a game loads, so Cosmic Insight is known",
    "Top lane quest Teleport shows time left and when it is back (from 1.0.9)",
    "Lucidity boots badge (+10 / +20) next to each enemy (from 1.0.8)",
    "Detects Cosmic Insight by itself when there is no key (from 1.0.7)",
];

pub struct Installer {
    pal: Pal,
    desktop: bool,
    start_menu: bool,
    launch: bool,
    status: String,
    busy: Option<Receiver<Result<(), String>>>,
    done: bool,
    installed: Option<String>,
    chrome: bool,
}

impl Installer {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Installer {
        theme::install_fonts(&cc.egui_ctx);
        let pal = theme::resolve("System");
        theme::apply(&cc.egui_ctx, &pal);
        let installed = shell::installed_version();
        // when updating, keep the shortcuts the way the user has them
        let (start_menu, desktop) = if installed.is_some() { (shell::has_start_menu_link(), shell::has_desktop_link()) } else { (true, true) };
        Installer { pal, desktop, start_menu, launch: true, status: String::new(), busy: None, done: false, installed, chrome: false }
    }
}

impl eframe::App for Installer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let p = self.pal;
        if !self.chrome {
            if let Some(h) = win::find_window("Flashless Setup") {
                win::set_title_bar(h, p.dark, theme::rgb(p.bg), theme::rgb(p.text), theme::rgb(p.line_strong));
                self.chrome = true;
            }
        }
        if let Some(rx) = &self.busy {
            if let Ok(res) = rx.try_recv() {
                self.busy = None;
                match res {
                    Ok(()) if self.launch => { shell::launch_installed(); ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                    Ok(()) => {
                        self.done = true;
                        self.status = match (self.start_menu, self.desktop) {
                            (true, true) => "Installed. Search \"Flashless\" in Start, or use the desktop shortcut.".into(),
                            (true, false) => "Installed. Search \"Flashless\" in Start to open it.".into(),
                            (false, true) => "Installed. Open it from the desktop shortcut.".into(),
                            (false, false) => format!("Installed to {}.", shell::install_dir().display()),
                        };
                    }
                    Err(e) => self.status = format!("Setup failed: {e}"),
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        egui::TopBottomPanel::bottom("footer").frame(egui::Frame::none().fill(p.surface).stroke(Stroke::new(1.0_f32, p.line)).inner_margin(egui::Margin::symmetric(28.0, 14.0))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.status).size(11.5).color(p.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if self.done { "Finish" } else if self.installed.is_some() { "Update" } else { "Install" };
                    let enabled = self.busy.is_none();
                    let b = egui::Button::new(RichText::new(label).font(theme::semi(12.5)).color(p.accent_text)).fill(p.accent).stroke(Stroke::NONE).rounding(6.0_f32).min_size(egui::vec2(96.0, 30.0));
                    if ui.add_enabled(enabled, b).clicked() {
                        if self.done { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                        else {
                            let (tx, rx) = channel();
                            let (desktop, start_menu) = (self.desktop, self.start_menu);
                            std::thread::spawn(move || { let _ = tx.send(shell::install(desktop, start_menu)); });
                            self.busy = Some(rx);
                            self.status = "Installing...".into();
                        }
                    }
                    if ui.add_enabled(enabled && !self.done, egui::Button::new("Cancel").min_size(egui::vec2(80.0, 30.0))).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        });

        egui::CentralPanel::default().frame(egui::Frame::none().fill(p.bg).inner_margin(egui::Margin { left: 28.0, right: 28.0, top: 24.0, bottom: 18.0 })).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(42.0, 42.0), egui::Sense::hover());
                theme::brand(ui.painter(), r.center(), 20.0, &p);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new("Flashless").font(theme::semi(24.0)));
                    let sub = match &self.installed {
                        Some(v) if v != shell::VERSION => format!("Update {v} -> {}", shell::VERSION),
                        Some(_) => format!("Version {} (reinstall)", shell::VERSION),
                        None => format!("Version {}", shell::VERSION),
                    };
                    ui.label(RichText::new(sub).size(12.5).color(p.muted));
                });
            });
            ui.add_space(16.0);
            egui::Frame::none().fill(p.surface).stroke(Stroke::new(1.0_f32, p.line)).rounding(10.0_f32).inner_margin(egui::Margin::symmetric(16.0, 14.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let updating = self.installed.as_deref().is_some_and(|v| v != shell::VERSION);
                let title = if updating { format!("What's new in {}", shell::VERSION) } else { "League of Legends companion app.".to_string() };
                ui.add(egui::Label::new(RichText::new(title).font(theme::semi(13.0))).wrap());
                ui.add_space(6.0);
                const FEATURES: &[&str] = &[
                    "Tracks enemy Flash and summoner spells with exact cooldowns: boots, runes, map and level",
                    "Picks up \"used Flash\" calls from game chat and starts the timer for you",
                    "One-line team message, Flash first, enemies named by lane",
                    "Copy it, or type it into chat with one hotkey",
                    "Compact mode, light and dark themes, rebindable keyboard and mouse hotkeys",
                    "Built in Rust: a 7 MB app that stays light on memory and CPU",
                ];
                let list: &[&str] = if updating { WHATS_NEW } else { FEATURES };
                for f in list {
                    ui.horizontal_top(|ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 17.0), egui::Sense::hover());
                        ui.painter().circle_filled(egui::pos2(r.center().x, r.top() + 8.5), 2.5, p.accent);
                        ui.add(egui::Label::new(RichText::new(*f).size(12.0)).wrap());
                    });
                }
                if let Some(v) = &self.installed {
                    ui.add_space(6.0);
                    let text = if v == shell::VERSION { format!("Version {v} is already installed. Continuing reinstalls it and keeps your settings.") }
                               else { format!("Version {v} is installed. Updating replaces the app and keeps your settings, hotkeys and shortcuts.") };
                    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(p.muted)).wrap());
                }
                ui.add_space(10.0);
                ui.label(RichText::new("INSTALL LOCATION").font(theme::semi(11.0)).color(p.accent_deep));
                ui.label(RichText::new(shell::install_dir().display().to_string()).font(theme::mono(11.5)));
                ui.label(RichText::new("Installed for your Windows account only. No administrator rights needed.").size(11.5).color(p.muted));
                ui.add_space(8.0);
                let editable = self.busy.is_none() && !self.done;
                ui.add_enabled_ui(editable, |ui| { super::theme::check(ui, &mut self.start_menu, "Create a Start menu shortcut (find Flashless in Windows search)"); });
                ui.add_enabled_ui(editable, |ui| { super::theme::check(ui, &mut self.desktop, "Create a desktop shortcut"); });
                ui.add_enabled_ui(self.busy.is_none() && !self.done, |ui| { super::theme::check(ui, &mut self.launch, "Start Flashless when setup finishes"); });
                ui.add_space(8.0);
                ui.add(egui::Label::new(RichText::new(crate::RIOT_NOTICE).size(10.0).color(p.muted)).wrap());
            });
        });
    }
}
