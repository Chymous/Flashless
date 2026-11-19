//! Cooldown maths: cooldown = base * 100 / (100 + haste), plus the rules Data Dragon does not carry.

use crate::data::{GameData, SummonerSpellInfo};
use crate::tracker::Enemy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Values from the official LoL wiki (Nov 2026). Editable in settings.json if a patch changes them.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct CooldownRules {
    /// Summoner spell haste from a map aura, by Live Client mapNumber (12 = Howling Abyss).
    pub map_summoner_haste: HashMap<i32, i32>,
    pub cosmic_insight_summoner_haste: i32,
    /// Teleport becomes Unleashed Teleport at this game time (seconds).
    pub teleport_unleash_at: f64,
    /// Unleashed Teleport is "330 - 240 (based on level)"; per-level values are interpolated linearly (an assumption).
    pub unleashed_teleport_level1: f64,
    pub unleashed_teleport_level18: f64,
    /// Teleport's cooldown starts after the 3 s channel and the dash.
    pub teleport_start_delay: f64,
    /// Unleashed Teleport a top laner gets from the 2026 role quest when they did not take Teleport.
    pub quest_teleport_cooldown: f64,
}

impl Default for CooldownRules {
    fn default() -> Self {
        CooldownRules {
            map_summoner_haste: HashMap::from([(12, 70)]),
            cosmic_insight_summoner_haste: 18,
            teleport_unleash_at: 600.0,
            unleashed_teleport_level1: 330.0,
            unleashed_teleport_level18: 240.0,
            teleport_start_delay: 4.0,
            quest_teleport_cooldown: 420.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GameContext {
    pub game_time: f64,
    pub is_live: bool,
    pub map_number: i32,
}

/// How a cooldown was derived, shown in tooltips.
#[derive(Clone, Debug)]
pub struct Breakdown {
    pub ability_name: String,
    pub base: f64,
    pub haste: f64,
    pub cooldown: f64,
    pub start_delay: f64,
    pub detail: String,
}

pub fn apply_haste(base: f64, haste: f64) -> f64 {
    if haste <= -100.0 { base } else { base * 100.0 / (100.0 + haste) }
}

pub fn is_teleport(spell: &SummonerSpellInfo) -> bool {
    spell.id.to_lowercase().contains("teleport")
}

pub fn for_summoner(spell: &SummonerSpellInfo, enemy: &Enemy, ctx: GameContext, data: &GameData, rules: &CooldownRules) -> Breakdown {
    let mut base = spell.cooldown;
    let mut delay = 0.0;
    let mut parts = Vec::new();

    if is_teleport(spell) {
        delay = rules.teleport_start_delay;
        if ctx.is_live && ctx.game_time >= rules.teleport_unleash_at {
            let lvl = enemy.level.clamp(1, 18);
            base = rules.unleashed_teleport_level1 + (rules.unleashed_teleport_level18 - rules.unleashed_teleport_level1) * (lvl - 1) as f64 / 17.0;
            parts.push(format!("Unleashed lv{lvl}"));
        }
    }

    let mut haste = 0;
    for &id in &enemy.item_ids {
        if let Some(it) = data.find_item(id) {
            if it.summoner_haste > 0 {
                haste += it.summoner_haste;
                parts.push(format!("{} +{}", it.name, it.summoner_haste));
            }
        }
    }
    if enemy.cosmic_insight {
        haste += rules.cosmic_insight_summoner_haste;
        parts.push(format!("Cosmic Insight +{}", rules.cosmic_insight_summoner_haste));
    }
    if ctx.is_live {
        if let Some(&aura) = rules.map_summoner_haste.get(&ctx.map_number) {
            haste += aura;
            parts.push(format!("map aura +{aura}"));
        }
    }
    if enemy.extra_summoner_haste != 0 {
        haste += enemy.extra_summoner_haste;
        parts.push(format!("manual {:+}", enemy.extra_summoner_haste));
    }
    Breakdown { ability_name: spell.name.clone(), base, haste: haste as f64, cooldown: apply_haste(base, haste as f64), start_delay: delay, detail: parts.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::test_enemy as make;

    fn data() -> GameData { GameData::embedded() }
    const RIFT: GameContext = GameContext { game_time: 300.0, is_live: true, map_number: 11 };

    fn flash(e: &Enemy, ctx: GameContext) -> f64 {
        let d = data();
        for_summoner(d.find_spell(Some("SummonerFlash")).unwrap(), e, ctx, &d, &CooldownRules::default()).cooldown
    }

    #[test]
    fn flash_follows_summoner_haste() {
        assert!((flash(&make("Lucian", 6, &[]), RIFT) - 300.0).abs() < 0.01);
        assert!((flash(&make("Lucian", 6, &[3158]), RIFT) - 272.73).abs() < 0.01);
        let mut ci = make("Lucian", 6, &[3158]);
        ci.cosmic_insight = true;
        assert!((flash(&ci, RIFT) - 234.38).abs() < 0.01);
        assert!((flash(&make("Lucian", 6, &[]), GameContext { map_number: 12, ..RIFT }) - 176.47).abs() < 0.01);
    }

    #[test]
    fn teleport_unleashes_at_ten_minutes() {
        let d = data();
        let tp = d.find_spell(Some("SummonerTeleport")).unwrap();
        let r = CooldownRules::default();
        let early = for_summoner(tp, &make("Shen", 9, &[]), GameContext { game_time: 599.0, ..RIFT }, &d, &r);
        assert!((early.cooldown - 300.0).abs() < 0.01);
        assert_eq!(early.start_delay, 4.0);
        assert!((for_summoner(tp, &make("Shen", 1, &[]), GameContext { game_time: 600.0, ..RIFT }, &d, &r).cooldown - 330.0).abs() < 0.01);
        assert!((for_summoner(tp, &make("Shen", 18, &[]), GameContext { game_time: 1500.0, ..RIFT }, &d, &r).cooldown - 240.0).abs() < 0.01);
    }
}
