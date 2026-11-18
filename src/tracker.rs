//! Enemy roster and running cooldowns.

use crate::cooldown::{self, Breakdown, CooldownRules, GameContext};
use crate::data::GameData;
use crate::live::{self, LiveSnapshot};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Spell1,
    Spell2,
}

pub const SLOTS: [Slot; 2] = [Slot::Spell1, Slot::Spell2];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveCooldown {
    pub started_at: f64,
    pub ready_at: f64,
}

impl ActiveCooldown {
    pub fn duration(&self) -> f64 { self.ready_at - self.started_at }
}

#[derive(Clone, Debug, Default)]
pub struct Enemy {
    pub champion_id: String,
    pub name: String,
    /// Short lane name used in chat: Top, Jg, Mid, ADC, Sup.
    pub role: Option<String>,
    pub level: i32,
    pub item_ids: Vec<i64>,
    pub spell1_id: Option<String>,
    pub spell2_id: Option<String>,
    /// Runs the Inspiration tree, so Cosmic Insight is possible (the API never shows minor runes).
    pub has_inspiration: bool,
    pub cosmic_insight: bool,
    pub extra_summoner_haste: i32,
    pub cooldowns: HashMap<Slot, ActiveCooldown>,
}

impl Enemy {
    pub fn spell_id(&self, slot: Slot) -> Option<&str> {
        match slot {
            Slot::Spell1 => self.spell1_id.as_deref(),
            Slot::Spell2 => self.spell2_id.as_deref(),
        }
    }
}

/// Game time while a match is live (re-anchored on every API poll), otherwise a local monotonic clock.
pub struct GameClock {
    start: Instant,
    anchor: f64,
    anchor_elapsed: f64,
    pub is_live: bool,
    /// A live game was seen this session (stays true after the game ends, until the next one).
    pub seen_live: bool,
}

impl Default for GameClock {
    fn default() -> Self { GameClock { start: Instant::now(), anchor: 0.0, anchor_elapsed: 0.0, is_live: false, seen_live: false } }
}

impl GameClock {
    fn elapsed(&self) -> f64 { self.start.elapsed().as_secs_f64() }
    pub fn now(&self) -> f64 { self.anchor + (self.elapsed() - self.anchor_elapsed) }
    pub fn sync_live(&mut self, game_time: f64) {
        self.anchor = game_time;
        self.anchor_elapsed = self.elapsed();
        self.is_live = true;
        self.seen_live = true;
    }
    pub fn go_offline(&mut self) {
        let now = self.now();
        self.sync_live(now);
        self.is_live = false;
    }
}

/// Owns the roster and cooldowns. The UI reads it every frame; `revision` changes on every edit.
#[derive(Default)]
pub struct Tracker {
    pub enemies: Vec<Enemy>,
    pub map_number: i32,
    /// Changes on any edit (roster, levels, items, cooldowns): redraw.
    pub revision: u64,
    /// Changes only when a cooldown is set, cleared, adjusted, undone, reset or recalculated: auto-copy.
    pub cooldown_revision: u64,
    undo: Vec<(usize, String, Slot, Option<ActiveCooldown>)>,
    last_game_time: Option<f64>,
}

impl Tracker {
    fn touch(&mut self) { self.revision += 1; }
    fn touch_cooldowns(&mut self) { self.revision += 1; self.cooldown_revision += 1; }

    pub fn context(&self, now: f64, is_live: bool) -> GameContext {
        GameContext { game_time: now, is_live, map_number: if self.map_number == 0 { 11 } else { self.map_number } }
    }

    /// The cooldown this ability would get if it were used now.
    pub fn preview(&self, i: usize, slot: Slot, ctx: GameContext, data: &GameData, rules: &CooldownRules) -> Option<Breakdown> {
        let e = self.enemies.get(i)?;
        let spell = data.find_spell(e.spell_id(slot))?;
        (spell.cooldown > 0.0).then(|| cooldown::for_summoner(spell, e, ctx, data, rules))
    }

    /// Records that the ability was used `seconds_ago` seconds before `now`.
    pub fn register(&mut self, i: usize, slot: Slot, seconds_ago: f64, now: f64, is_live: bool, data: &GameData, rules: &CooldownRules) -> bool {
        let Some(cd) = self.preview(i, slot, self.context(now, is_live), data, rules) else { return false };
        let start = now - seconds_ago.max(0.0) + cd.start_delay;
        self.set(i, slot, Some(ActiveCooldown { started_at: start, ready_at: start + cd.cooldown }));
        true
    }

    pub fn clear(&mut self, i: usize, slot: Slot) {
        if self.enemies.get(i).is_some_and(|e| e.cooldowns.contains_key(&slot)) {
            self.set(i, slot, None);
        }
    }

    pub fn adjust(&mut self, i: usize, slot: Slot, delta: f64) {
        if let Some(cur) = self.enemies.get(i).and_then(|e| e.cooldowns.get(&slot)).copied() {
            self.set(i, slot, Some(ActiveCooldown { ready_at: cur.ready_at + delta, ..cur }));
        }
    }

    /// Re-derives an enemy's running cooldowns after a haste estimate changed.
    pub fn recalculate(&mut self, i: usize, now: f64, is_live: bool, data: &GameData, rules: &CooldownRules) {
        let ctx = self.context(now, is_live);
        let slots: Vec<Slot> = self.enemies[i].cooldowns.keys().copied().collect();
        for slot in slots {
            if let Some(cd) = self.preview(i, slot, ctx, data, rules) {
                let cur = self.enemies[i].cooldowns[&slot];
                self.enemies[i].cooldowns.insert(slot, ActiveCooldown { ready_at: cur.started_at + cd.cooldown, ..cur });
            }
        }
        self.touch_cooldowns();
    }

    pub fn undo(&mut self) -> bool {
        while let Some((i, champ, slot, prev)) = self.undo.pop() {
            let Some(e) = self.enemies.get_mut(i).filter(|e| e.champion_id == champ) else { continue };
            match prev {
                Some(p) => { e.cooldowns.insert(slot, p); }
                None => { e.cooldowns.remove(&slot); }
            }
            self.touch_cooldowns();
            return true;
        }
        false
    }

    pub fn reset_cooldowns(&mut self) {
        for e in &mut self.enemies { e.cooldowns.clear(); }
        self.undo.clear();
        self.touch_cooldowns();
    }

    pub fn remaining(&self, i: usize, slot: Slot, now: f64) -> f64 {
        self.enemies.get(i).and_then(|e| e.cooldowns.get(&slot)).map(|c| (c.ready_at - now).max(0.0)).unwrap_or(0.0)
    }

    /// Drops expired cooldowns; true if anything changed.
    pub fn prune(&mut self, now: f64) -> bool {
        let mut removed = false;
        for e in &mut self.enemies {
            let before = e.cooldowns.len();
            e.cooldowns.retain(|_, c| c.ready_at > now);
            removed |= e.cooldowns.len() != before;
        }
        if removed { self.touch(); }
        removed
    }

    pub fn set_roster(&mut self, enemies: Vec<Enemy>) {
        self.enemies = enemies;
        self.undo.clear();
        self.touch();
    }

    /// Applies a Live Client snapshot: a different set of champions, or the timer running backwards,
    /// means a new match. Returns true if the roster was replaced.
    pub fn apply_live(&mut self, snap: &LiveSnapshot, data: &GameData) -> bool {
        self.map_number = snap.map_number;
        let new_game = self.last_game_time.is_some_and(|t| snap.game_time < t - 30.0);
        self.last_game_time = Some(snap.game_time);

        let mut incoming: Vec<Enemy> = snap.enemies.iter().map(|p| live::to_enemy(p, data, &snap.game_mode)).collect();
        live::complete_roles(&mut incoming, data);
        sort_by_role(&mut incoming);
        let ids = |v: &[Enemy]| { let mut x: Vec<String> = v.iter().map(|e| e.champion_id.clone()).collect(); x.sort(); x };
        if !new_game && !self.enemies.is_empty() && incoming.len() > self.enemies.len()
            && self.enemies.iter().all(|e| incoming.iter().any(|x| x.champion_id == e.champion_id)) {
            // the API briefly lost track of your team and listed all ten players: keep the current enemies
            incoming.retain(|x| self.enemies.iter().any(|e| e.champion_id == x.champion_id));
        }
        if new_game || ids(&incoming) != ids(&self.enemies) {
            if !new_game {
                // same match, roster changed (a champion resolved late, a swap): keep the timers that still apply
                for e in incoming.iter_mut() {
                    if let Some(old) = self.enemies.iter().find(|o| o.champion_id == e.champion_id) {
                        e.cooldowns = old.cooldowns.clone();
                        e.cosmic_insight = old.cosmic_insight;
                        e.extra_summoner_haste = old.extra_summoner_haste;
                    }
                }
            }
            self.set_roster(incoming);
            return true;
        }
        let mut changed = false;
        for fresh in incoming {
            if let Some(cur) = self.enemies.iter_mut().find(|e| e.champion_id == fresh.champion_id) {
                changed |= cur.level != fresh.level || cur.item_ids != fresh.item_ids || cur.role != fresh.role;
                cur.level = fresh.level;
                cur.item_ids = fresh.item_ids;
                cur.spell1_id = fresh.spell1_id;
                cur.spell2_id = fresh.spell2_id;
                cur.has_inspiration = fresh.has_inspiration;
                cur.role = fresh.role;
            }
        }
        // a role can arrive a moment after the champion: keep rows in lane order
        let before: Vec<String> = self.enemies.iter().map(|e| e.champion_id.clone()).collect();
        sort_by_role(&mut self.enemies);
        if self.enemies.iter().map(|e| &e.champion_id).ne(before.iter()) { self.undo.clear(); changed = true; }
        if changed { self.touch(); }
        false
    }

    fn set(&mut self, i: usize, slot: Slot, value: Option<ActiveCooldown>) {
        let e = &mut self.enemies[i];
        let prev = e.cooldowns.get(&slot).copied();
        self.undo.push((i, e.champion_id.clone(), slot, prev));
        if self.undo.len() > 100 { self.undo.remove(0); }
        match value {
            Some(v) => { e.cooldowns.insert(slot, v); }
            None => { e.cooldowns.remove(&slot); }
        }
        self.touch_cooldowns();
    }
}

/// Lane order for the overlay rows: Top, Jg, Mid, ADC, Sup; enemies without a known role keep their order at the end.
pub fn sort_by_role(enemies: &mut [Enemy]) {
    const ORDER: [&str; 5] = ["Top", "Jg", "Mid", "ADC", "Sup"];
    enemies.sort_by_key(|e| e.role.as_deref().and_then(|r| ORDER.iter().position(|o| *o == r)).unwrap_or(ORDER.len()));
}

#[cfg(test)]
pub fn test_enemy(champ: &str, level: i32, items: &[i64]) -> Enemy {
    let d = crate::data::GameData::embedded();
    Enemy {
        champion_id: champ.to_string(),
        name: d.find_champion(champ).map(|c| c.name.clone()).unwrap_or_else(|| champ.to_string()),
        level,
        item_ids: items.to_vec(),
        spell1_id: Some("SummonerFlash".into()),
        spell2_id: Some("SummonerBarrier".into()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_cooldown_edits_trigger_auto_copy() {
        let d = GameData::embedded();
        let r = CooldownRules::default();
        let mut t = Tracker::default();
        t.set_roster(vec![test_enemy("Lucian", 6, &[])]);
        let start = t.cooldown_revision;
        // live data refresh (level up, new item) and timers running out must not overwrite the clipboard
        t.enemies[0].level = 7;
        t.touch();
        t.register(0, Slot::Spell1, 0.0, 100.0, true, &d, &r);
        let after_register = t.cooldown_revision;
        assert_eq!(after_register, start + 1);
        assert!(t.prune(1000.0));
        assert_eq!(t.cooldown_revision, after_register);
        t.register(0, Slot::Spell1, 0.0, 1000.0, true, &d, &r);
        t.undo();
        t.reset_cooldowns();
        assert_eq!(t.cooldown_revision, after_register + 3);
    }

    #[test]
    fn rows_are_in_lane_order() {
        let mut v: Vec<Enemy> = [("Vi", Some("Jg")), ("KogMaw", Some("ADC")), ("Maokai", Some("Top")), ("Malphite", Some("Mid")), ("Milio", Some("Sup")), ("Teemo", None)]
            .into_iter().map(|(c, r)| { let mut e = test_enemy(c, 1, &[]); e.role = r.map(String::from); e }).collect();
        v.rotate_left(5); // unknown role first
        sort_by_role(&mut v);
        let order: Vec<&str> = v.iter().map(|e| e.champion_id.as_str()).collect();
        assert_eq!(order, ["Maokai", "Vi", "Malphite", "KogMaw", "Milio", "Teemo"]);
    }

    #[test]
    fn register_undo_adjust_recalculate() {
        let d = GameData::embedded();
        let r = CooldownRules::default();
        let mut t = Tracker::default();
        t.set_roster(vec![test_enemy("Lucian", 6, &[])]);
        let now = 600.0;
        assert!(t.register(0, Slot::Spell1, 0.0, now, true, &d, &r));
        assert!((t.remaining(0, Slot::Spell1, now) - 300.0).abs() < 0.01);
        t.adjust(0, Slot::Spell1, -5.0);
        assert!((t.remaining(0, Slot::Spell1, now) - 295.0).abs() < 0.01);
        assert!(t.undo());
        assert!((t.remaining(0, Slot::Spell1, now) - 300.0).abs() < 0.01);
        t.enemies[0].cosmic_insight = true;
        t.recalculate(0, now, true, &d, &r);
        assert!((t.remaining(0, Slot::Spell1, now) - 254.24).abs() < 0.01);
        t.clear(0, Slot::Spell1);
        assert_eq!(t.remaining(0, Slot::Spell1, now), 0.0);
        assert!(t.undo());
        assert!(t.remaining(0, Slot::Spell1, now) > 0.0);
        assert!(t.register(0, Slot::Spell1, 10.0, now + 10.0, true, &d, &r));
        assert!((t.remaining(0, Slot::Spell1, now + 10.0) - 244.24).abs() < 0.01);
        assert!(t.prune(now + 400.0));
        assert!(t.enemies[0].cooldowns.is_empty());
    }

    #[test]
    fn a_glitched_poll_does_not_clear_cooldowns() {
        let d = GameData::embedded();
        let r = CooldownRules::default();
        let p = |c: &str, team: &str| live::LivePlayer { champion_raw: format!("game_character_displayname_{c}"), champion_name: c.into(), team: team.into(),
            position: String::new(), level: 6, item_ids: vec![], spell1_raw: String::new(), spell1_name: "Flash".into(), spell2_raw: String::new(),
            spell2_name: "Heal".into(), has_inspiration: false };
        let snap = |enemies: Vec<live::LivePlayer>, t: f64| LiveSnapshot { game_time: t, game_mode: "CLASSIC".into(), map_number: 11, enemies };
        let mut t = Tracker::default();
        t.apply_live(&snap(vec![p("Diana", "CHAOS"), p("Vayne", "CHAOS")], 1000.0), &d);
        assert!(t.register(0, Slot::Spell1, 0.0, 1000.0, true, &d, &r));
        assert!(t.register(1, Slot::Spell1, 0.0, 1010.0, true, &d, &r));
        // one poll lists everyone (your team not found)
        t.apply_live(&snap(vec![p("Diana", "CHAOS"), p("Vayne", "CHAOS"), p("Ahri", "ORDER"), p("Jinx", "ORDER")], 1012.0), &d);
        assert_eq!(t.enemies.len(), 2);
        // a late champion change in the same game keeps the others' timers
        t.apply_live(&snap(vec![p("Diana", "CHAOS"), p("Garen", "CHAOS")], 1014.0), &d);
        let diana = t.enemies.iter().position(|e| e.champion_id == "Diana").unwrap();
        assert!(t.remaining(diana, Slot::Spell1, 1014.0) > 200.0);
    }
}
