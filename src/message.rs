//! Builds the one-line cooldown message for team chat, e.g. "Mid 1554f 1600b | Sup 1657f".

use crate::data::GameData;
use crate::tracker::{Enemy, Slot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeFormat {
    /// Game clock when the ability is back, 2230 = 22:30. Stays correct after pasting.
    GameClock,
    /// Whole seconds remaining when the message is built.
    SecondsRemaining,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct MessageOptions {
    pub format: TimeFormat,
    /// Other spells longest first (off = soonest first). Flash always leads.
    pub longest_first: bool,
    pub flash_first: bool,
    /// Name enemies by lane (Top, Jg, Mid, ADC, Sup) when known.
    pub use_roles: bool,
    pub abbreviations: BTreeMap<String, String>,
}

impl Default for MessageOptions {
    fn default() -> Self {
        MessageOptions {
            format: TimeFormat::GameClock,
            longest_first: false,
            flash_first: true,
            use_roles: true,
            abbreviations: default_abbreviations(),
        }
    }
}

pub fn default_abbreviations() -> BTreeMap<String, String> {
    [("Flash", "f"), ("Ignite", "ig"), ("Barrier", "b"), ("Exhaust", "exh"), ("Heal", "h"), ("Ghost", "g"), ("Cleanse", "cl"),
     ("Teleport", "tp"), ("Smite", "smite"), ("Clarity", "clar"), ("Mark", "snow")]
        .into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

impl MessageOptions {
    pub fn abbreviate(&self, spell_name: &str) -> String {
        self.abbreviations.iter().find(|(k, _)| k.eq_ignore_ascii_case(spell_name)).map(|(_, v)| v.clone())
            .unwrap_or_else(|| spell_name.replace(' ', "").to_lowercase())
    }
}

pub fn format_time(ready_at: f64, now: f64, is_live: bool, format: TimeFormat) -> String {
    if format == TimeFormat::GameClock && is_live {
        let t = ready_at.round() as i64;
        format!("{}{:02}", t / 60, t % 60)
    } else {
        format!("{}", (ready_at - now).round() as i64)
    }
}

fn is_flash(e: &Enemy, slot: Slot, data: &GameData) -> bool {
    data.find_spell(e.spell_id(slot)).is_some_and(|s| s.name == "Flash")
}

fn abbr(e: &Enemy, slot: Slot, data: &GameData, opt: &MessageOptions) -> String {
    data.find_spell(e.spell_id(slot)).map(|s| opt.abbreviate(&s.name)).unwrap_or_else(|| "ss".into())
}

/// Every ability still on cooldown, one line: enemies ordered by when their Flash is back.
pub fn build(enemies: &[Enemy], data: &GameData, now: f64, is_live: bool, opt: &MessageOptions) -> String {
    struct Block { has_flash: bool, key: f64, index: usize, text: String }
    let mut blocks = Vec::new();
    for (index, e) in enemies.iter().enumerate() {
        let mut entries: Vec<(String, f64, bool)> = e.cooldowns.iter()
            .filter(|(_, c)| c.ready_at - now > 0.5)
            .map(|(&slot, c)| (abbr(e, slot, data, opt), c.ready_at, is_flash(e, slot, data)))
            .collect();
        if entries.is_empty() { continue; }
        entries.sort_by(|a, b| {
            let fa = if opt.flash_first && a.2 { 0 } else { 1 };
            let fb = if opt.flash_first && b.2 { 0 } else { 1 };
            fa.cmp(&fb).then_with(|| {
                let (x, y) = if opt.longest_first { (b.1, a.1) } else { (a.1, b.1) };
                x.partial_cmp(&y).unwrap()
            })
        });
        let label = match &e.role { Some(r) if opt.use_roles && !r.is_empty() => r.clone(), _ => e.name.clone() };
        let parts: Vec<String> = entries.iter().map(|(a, t, _)| format!("{}{}", format_time(*t, now, is_live, opt.format), a)).collect();
        let flash = entries.iter().find(|x| x.2);
        let has_flash = opt.flash_first && flash.is_some();
        let key = if has_flash { flash.unwrap().1 } else { entries.iter().map(|x| x.1).fold(f64::MAX, f64::min) };
        blocks.push(Block { has_flash, key, index, text: format!("{label} {}", parts.join(" ")) });
    }
    if opt.flash_first {
        blocks.sort_by(|a, b| (!a.has_flash).cmp(&(!b.has_flash)).then(a.key.partial_cmp(&b.key).unwrap()).then(a.index.cmp(&b.index)));
    }
    blocks.into_iter().map(|b| b.text).collect::<Vec<_>>().join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cooldown::CooldownRules;
    use crate::tracker::{test_enemy, Tracker};

    #[test]
    fn flash_leads_and_roles_name_enemies() {
        let d = GameData::embedded();
        let r = CooldownRules::default();
        let mut t = Tracker::default();
        let mut galio = test_enemy("Galio", 6, &[]);
        galio.spell2_id = Some("SummonerTeleport".into());
        let mut varus = test_enemy("Varus", 6, &[]);
        varus.spell1_id = Some("SummonerBarrier".into());
        varus.spell2_id = Some("SummonerFlash".into());
        varus.role = Some("ADC".into());
        let mut lulu = test_enemy("Lulu", 6, &[]);
        lulu.spell2_id = Some("SummonerHeal".into());
        lulu.role = Some("Sup".into());
        let mut rumble = test_enemy("Rumble", 6, &[]);
        rumble.spell2_id = Some("SummonerDot".into());
        t.set_roster(vec![galio, varus, lulu, rumble]);
        let now = 900.0;
        t.register(3, Slot::Spell2, 170.0, now, true, &d, &r); // ignite 15:10
        t.register(0, Slot::Spell1, 9.0, now, true, &d, &r);   // flash 19:51
        t.register(1, Slot::Spell1, 120.0, now, true, &d, &r); // barrier 16:00
        t.register(1, Slot::Spell2, 246.0, now, true, &d, &r); // flash 15:54
        t.register(2, Slot::Spell1, 183.0, now, true, &d, &r); // flash 16:57
        t.register(2, Slot::Spell2, 230.0, now, true, &d, &r); // heal 15:10
        let msg = build(&t.enemies, &d, now, true, &MessageOptions::default());
        assert_eq!(msg, "ADC 1554f 1600b | Sup 1657f 1510h | Galio 1951f | Rumble 1510ig");
        let secs = build(&t.enemies, &d, now, true, &MessageOptions { format: TimeFormat::SecondsRemaining, use_roles: false, ..Default::default() });
        assert_eq!(secs, "Varus 54f 60b | Lulu 117f 10h | Galio 291f | Rumble 10ig");
        assert_eq!(format_time(142.0, 100.0, false, TimeFormat::GameClock), "42");
    }
}
