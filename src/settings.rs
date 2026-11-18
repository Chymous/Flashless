//! Settings in %LOCALAPPDATA%\Flashless\settings.json.

use crate::cooldown::CooldownRules;
use crate::message::MessageOptions;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Flashless")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Settings {
    pub auto_copy: bool,
    pub copy_delay_ms: u64,
    /// Seconds a spell is assumed to have been used before Flashless hears about it (click, hotkey or chat),
    /// for the time it takes to see it and call it.
    #[serde(rename = "CallDelaySeconds")]
    pub reaction_offset_seconds: f64,
    pub chat_key_delay_ms: u64,
    /// After typing into chat, Ctrl+A / Ctrl+C in the chat box so League's own clipboard holds the line.
    pub copy_in_chat_after_typing: bool,

    pub chat_detection: bool,
    /// Chat area as fractions of the game window: left, top, width, height.
    pub chat_region: [f64; 4],
    pub chat_ocr_scale: u32,
    pub chat_scan_interval_ms: u64,
    pub chat_log: bool,

    /// "System", "Light" or "Dark".
    pub theme: String,
    pub opacity: f32,
    pub always_on_top: bool,
    pub show_preview: bool,
    /// Mini HUD: lane and two spell icons per enemy, no message or buttons.
    pub compact: bool,
    /// Compact view while a game is live, full view in the lobby.
    pub auto_compact: bool,
    /// Hide the overlay while no game is running.
    pub hide_outside_game: bool,
    /// Clicks pass through the overlay to League while League is in front.
    pub click_through_in_game: bool,
    /// Pulse an enemy's Flash tile when it comes back up.
    pub flash_alert: bool,
    /// Also play the Windows notification sound then.
    pub flash_alert_sound: bool,
    /// Ask GitHub for a newer release on start.
    pub check_updates: bool,
    /// Which summoner spell sits in the left tile.
    pub spell_order: SpellOrder,
    pub left: Option<f32>,
    pub top: Option<f32>,

    pub message: MessageOptions,
    pub rules: CooldownRules,
    /// Action -> combo such as "Ctrl+Alt+C". Empty = unbound.
    pub hotkeys: BTreeMap<String, String>,
}

pub const DEFAULT_CHAT_REGION: [f64; 4] = [0.0, 0.40, 0.30, 0.57];

pub fn default_hotkeys() -> BTreeMap<String, String> {
    let mut h = BTreeMap::new();
    h.insert("CopySummary".into(), "Ctrl+Alt+C".into());
    h.insert("TypeInChat".into(), "Ctrl+Alt+V".into());
    h.insert("Undo".into(), "Ctrl+Alt+Z".into());
    h.insert("ResetAll".into(), "".into());
    h.insert("ToggleOverlay".into(), "Ctrl+Alt+O".into());
    h.insert("ToggleCompact".into(), "".into());
    for i in 1..=5 {
        h.insert(format!("Enemy{i}Flash"), format!("Ctrl+Alt+{i}"));
        h.insert(format!("Enemy{i}OtherSpell"), "".into());
    }
    h
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            auto_copy: true,
            copy_delay_ms: 150,
            reaction_offset_seconds: 7.0,
            chat_key_delay_ms: 20,
            copy_in_chat_after_typing: true,
            chat_detection: true,
            chat_region: DEFAULT_CHAT_REGION,
            chat_ocr_scale: 2,
            chat_scan_interval_ms: 1000,
            chat_log: true,
            theme: "System".into(),
            opacity: 0.95,
            always_on_top: true,
            show_preview: true,
            compact: false,
            auto_compact: false,
            hide_outside_game: false,
            click_through_in_game: true,
            flash_alert: true,
            flash_alert_sound: false,
            check_updates: true,
            spell_order: SpellOrder::FlashLeft,
            left: None,
            top: None,
            message: MessageOptions::default(),
            rules: CooldownRules::default(),
            hotkeys: default_hotkeys(),
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf { data_dir().join("settings.json") }

    pub fn load() -> Settings {
        let mut s: Settings = std::fs::read_to_string(Self::path()).ok()
            .and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok()) // tolerate a BOM from hand edits
            .unwrap_or_default();
        let defaults = default_hotkeys();
        s.hotkeys.retain(|k, _| defaults.contains_key(k)); // e.g. the removed ultimate hotkeys
        for (k, v) in defaults {
            s.hotkeys.entry(k).or_insert(v);
        }
        s
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(data_dir());
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), t);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpellOrder {
    /// Flash in the left tile, the other spell on the right.
    FlashLeft,
    /// Flash in the right tile.
    FlashRight,
    /// The enemy's own D / F order.
    Game,
}

impl SpellOrder {
    /// Left and right tile for an enemy whose D / F slot holds Flash (`flash_on_f`) or not.
    pub fn slots(self, flash_on_d: bool, flash_on_f: bool) -> [crate::tracker::Slot; 2] {
        use crate::tracker::Slot::{Spell1, Spell2};
        let swap = match self {
            SpellOrder::FlashLeft => flash_on_f && !flash_on_d,
            SpellOrder::FlashRight => flash_on_d && !flash_on_f,
            SpellOrder::Game => false,
        };
        if swap { [Spell2, Spell1] } else { [Spell1, Spell2] }
    }
}
