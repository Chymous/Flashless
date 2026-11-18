//! Riot's Live Client Data API (https://127.0.0.1:2999/liveclientdata/allgamedata): enemy champions, levels,
//! items, summoner spells, rune trees and lane positions. It does not expose enemy casts or cooldowns.

use crate::data::{GameData, SummonerSpellInfo};
use crate::tracker::Enemy;
use regex::Regex;
use serde_json::Value;
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

pub const URL: &str = "https://127.0.0.1:2999/liveclientdata/allgamedata";
const INSPIRATION_TREE: i64 = 8300;

#[derive(Clone, Debug, PartialEq)]
pub struct LivePlayer {
    pub champion_raw: String,
    pub champion_name: String,
    pub team: String,
    /// "Name#TAG"
    pub riot_id: String,
    pub position: String,
    pub level: i32,
    pub item_ids: Vec<i64>,
    pub spell1_raw: String,
    pub spell1_name: String,
    pub spell2_raw: String,
    pub spell2_name: String,
    pub has_inspiration: bool,
}

#[derive(Clone, Debug)]
pub struct LiveSnapshot {
    pub game_time: f64,
    pub game_mode: String,
    pub map_number: i32,
    pub enemies: Vec<LivePlayer>,
    /// Your own "Name#TAG".
    pub me: String,
}

fn s(v: &Value, k: &str) -> String { v[k].as_str().unwrap_or_default().to_string() }

pub fn parse(json: &str) -> Option<LiveSnapshot> {
    let root: Value = serde_json::from_str(json).ok()?;
    let players = root["allPlayers"].as_array()?;
    let game = root.get("gameData")?;
    let me = &root["activePlayer"];
    let (my_riot, my_name) = (s(me, "riotId"), s(me, "summonerName"));
    let my_team = players.iter()
        .find(|p| (!my_riot.is_empty() && s(p, "riotId") == my_riot) || (!my_name.is_empty() && s(p, "summonerName") == my_name))
        .map(|p| s(p, "team"));

    let mut enemies = Vec::new();
    for p in players {
        let team = s(p, "team");
        if my_team.as_deref() == Some(team.as_str()) { continue; } // spectating: keep everyone
        // bot games can list a placeholder player with no champion; it comes and goes, so skip it
        if s(p, "championName").is_empty() && s(p, "rawChampionName").is_empty() { continue; }
        let tree = |k: &str| p["runes"][k]["id"].as_i64().unwrap_or(0);
        let spell = |k: &str| { let x = &p["summonerSpells"][k]; (s(x, "rawDisplayName"), s(x, "displayName")) };
        let (s1r, s1n) = spell("summonerSpellOne");
        let (s2r, s2n) = spell("summonerSpellTwo");
        enemies.push(LivePlayer {
            champion_raw: s(p, "rawChampionName"),
            champion_name: s(p, "championName"),
            team,
            riot_id: s(p, "riotId"),
            position: s(p, "position"),
            level: p["level"].as_i64().unwrap_or(1) as i32,
            item_ids: p["items"].as_array().map(|a| a.iter().filter_map(|i| i["itemID"].as_i64()).collect()).unwrap_or_default(),
            spell1_raw: s1r, spell1_name: s1n, spell2_raw: s2r, spell2_name: s2n,
            has_inspiration: tree("primaryRuneTree") == INSPIRATION_TREE || tree("secondaryRuneTree") == INSPIRATION_TREE,
        });
    }
    Some(LiveSnapshot {
        game_time: game["gameTime"].as_f64().unwrap_or(0.0),
        game_mode: s(game, "gameMode"),
        map_number: game["mapNumber"].as_i64().unwrap_or(11) as i32,
        enemies,
        me: my_riot,
    })
}

/// rawDisplayName looks like "GeneratedTip_SummonerSpell_SummonerFlash_DisplayName".
pub fn resolve_spell<'a>(raw: &str, display: &str, data: &'a GameData, mode: &str) -> Option<&'a SummonerSpellInfo> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"SummonerSpell_(\w+?)_DisplayName").unwrap());
    if let Some(c) = re.captures(raw) {
        if let Some(sp) = data.find_spell(Some(&c[1])) { return Some(sp); }
    }
    let mut by_name: Vec<&SummonerSpellInfo> = data.spells.iter().filter(|x| x.name.eq_ignore_ascii_case(display)).collect();
    if by_name.is_empty() && !display.is_empty() {
        // "Unleashed Teleport" / "Unleashed Smite" -> base spell
        let d = display.to_lowercase();
        by_name = data.spells.iter().filter(|x| x.name.len() > 3 && d.contains(&x.name.to_lowercase())).collect();
    }
    by_name.iter().find(|x| x.modes.iter().any(|m| m == mode))
        .or_else(|| by_name.iter().find(|x| x.modes.iter().any(|m| m == "CLASSIC")))
        .or(by_name.first()).copied()
}

pub fn role_from_position(position: &str) -> Option<String> {
    match position.to_uppercase().as_str() {
        "TOP" => Some("Top"),
        "JUNGLE" => Some("Jg"),
        "MIDDLE" | "MID" => Some("Mid"),
        "BOTTOM" | "BOT" => Some("ADC"),
        "UTILITY" | "SUPPORT" => Some("Sup"),
        _ => None,
    }.map(String::from)
}

pub fn to_enemy(p: &LivePlayer, data: &GameData, mode: &str) -> Enemy {
    // rawChampionName looks like "game_character_displayname_Lucian"
    let raw_id = p.champion_raw.rsplit('_').next().unwrap_or_default().to_string();
    let champ = data.find_champion(&raw_id).or_else(|| data.find_champion(&p.champion_name));
    Enemy {
        champion_id: champ.map(|c| c.id.clone()).unwrap_or_else(|| if raw_id.is_empty() { p.champion_name.clone() } else { raw_id.clone() }),
        name: champ.map(|c| c.name.clone()).unwrap_or_else(|| p.champion_name.clone()),
        riot_id: p.riot_id.clone(),
        role: role_from_position(&p.position),
        level: p.level,
        item_ids: p.item_ids.clone(),
        spell1_id: resolve_spell(&p.spell1_raw, &p.spell1_name, data, mode).map(|x| x.id.clone()),
        spell2_id: resolve_spell(&p.spell2_raw, &p.spell2_name, data, mode).map(|x| x.id.clone()),
        has_inspiration: p.has_inspiration,
        ..Default::default()
    }
}

/// Fills roles the game did not report (the only enemy with Smite is the jungler) and clears duplicates,
/// so the message never names two enemies the same way.
pub fn complete_roles(enemies: &mut [Enemy], data: &GameData) {
    let has_smite = |e: &Enemy| [e.spell1_id.as_deref(), e.spell2_id.as_deref()].into_iter().any(|id| data.find_spell(id).is_some_and(|x| x.name == "Smite"));
    if !enemies.iter().any(|e| e.role.as_deref() == Some("Jg")) {
        let smiters: Vec<usize> = (0..enemies.len()).filter(|&i| has_smite(&enemies[i])).collect();
        if smiters.len() == 1 && enemies[smiters[0]].role.is_none() {
            enemies[smiters[0]].role = Some("Jg".into());
        }
    }
    let roles: Vec<Option<String>> = enemies.iter().map(|e| e.role.clone()).collect();
    for e in enemies.iter_mut() {
        if let Some(r) = &e.role {
            if roles.iter().filter(|x| x.as_deref() == Some(r.as_str())).count() > 1 { e.role = None; }
        }
    }
}

/// Polls the Live Client API on loopback. Sends Some(snapshot) in game, None when the game ends.
pub fn start_poller(tx: Sender<Option<LiveSnapshot>>, wake: Arc<dyn Fn() + Send + Sync>) {
    std::thread::Builder::new().name("live-client".into()).spawn(move || {
        // The game serves this endpoint with Riot's self-signed certificate; it is only ever on 127.0.0.1.
        let tls = native_tls::TlsConnector::builder().danger_accept_invalid_certs(true).danger_accept_invalid_hostnames(true).build();
        let Ok(tls) = tls else { return };
        let agent = ureq::AgentBuilder::new().tls_connector(Arc::new(tls)).timeout(Duration::from_secs(2)).build();
        let (mut live, mut failures) = (false, 0);
        loop {
            let snap = agent.get(URL).call().ok().and_then(|r| r.into_string().ok()).and_then(|b| parse(&b));
            match snap {
                Some(s) => {
                    failures = 0;
                    live = true;
                    if tx.send(Some(s)).is_err() { return; }
                    wake();
                }
                None if live => {
                    failures += 1;
                    if failures >= 2 {
                        live = false;
                        if tx.send(None).is_err() { return; }
                        wake();
                    }
                }
                None => {}
            }
            // allgamedata is built by the game itself, so ask rarely; the overlay clock runs locally in between
            std::thread::sleep(Duration::from_millis(if live { 5000 } else { 3000 }));
        }
    }).expect("spawn live poller");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(name: &str, champ: &str, team: &str, pos: &str, lvl: i32, s1: &str, s2: &str, items: &str, secondary: i64) -> String {
        format!(r#"{{"championName":"{champ}","rawChampionName":"game_character_displayname_{raw}","team":"{team}","position":"{pos}","level":{lvl},
            "riotId":"{name}#NA1","summonerName":"{name}","items":[{items}],
            "runes":{{"keystone":{{"id":8005}},"primaryRuneTree":{{"id":8100}},"secondaryRuneTree":{{"id":{secondary}}}}},
            "summonerSpells":{{"summonerSpellOne":{{"displayName":"{s1}","rawDisplayName":"GeneratedTip_SummonerSpell_Summoner{s1}_DisplayName"}},
                               "summonerSpellTwo":{{"displayName":"{s2}","rawDisplayName":"GeneratedTip_SummonerSpell_Bogus_DisplayName"}}}}}}"#,
            raw = champ.replace(' ', ""))
    }

    fn game(time: f64, players: &[String]) -> String {
        format!(r#"{{"activePlayer":{{"riotId":"Me#NA1","summonerName":"Me"}},"allPlayers":[{}],"events":{{"Events":[]}},
            "gameData":{{"gameMode":"CLASSIC","gameTime":{time},"mapNumber":11}}}}"#, players.join(","))
    }

    #[test]
    fn parses_enemies_spells_items_and_roles() {
        let d = GameData::embedded();
        let json = game(754.2, &[
            player("Me", "Ahri", "ORDER", "MIDDLE", 9, "Flash", "Ignite", "", 8000),
            player("Foe1", "Lucian", "CHAOS", "BOTTOM", 8, "Flash", "Barrier", r#"{"itemID":3158},{"itemID":1055}"#, 8300),
            player("Foe2", "Nautilus", "CHAOS", "UTILITY", 7, "Flash", "Unleashed Teleport", "", 8000),
        ]);
        let snap = parse(&json).unwrap();
        assert_eq!(snap.enemies.len(), 2);
        let mut t = crate::tracker::Tracker::default();
        assert!(t.apply_live(&snap, &d, &crate::cooldown::CooldownRules::default()));
        let lucian = &t.enemies[0];
        assert_eq!(lucian.champion_id, "Lucian");
        assert_eq!(lucian.spell1_id.as_deref(), Some("SummonerFlash"));
        assert_eq!(lucian.spell2_id.as_deref(), Some("SummonerBarrier"));
        assert!(lucian.has_inspiration);
        assert_eq!(lucian.role.as_deref(), Some("ADC"));
        assert_eq!(t.enemies[1].spell2_id.as_deref(), Some("SummonerTeleport"));
        assert_eq!(t.enemies[1].role.as_deref(), Some("Sup"));
    }

    #[test]
    fn same_roster_keeps_cooldowns_new_match_resets() {
        let d = GameData::embedded();
        let r = crate::cooldown::CooldownRules::default();
        let me = player("Me", "Ahri", "ORDER", "", 9, "Flash", "Ignite", "", 8000);
        let foe = |lvl| player("Foe1", "Lucian", "CHAOS", "", lvl, "Flash", "Heal", "", 8000);
        let mut t = crate::tracker::Tracker::default();
        t.apply_live(&parse(&game(700.0, &[me.clone(), foe(8)])).unwrap(), &d, &crate::cooldown::CooldownRules::default());
        t.register(0, crate::tracker::Slot::Spell1, 0.0, 700.0, true, &d, &r);
        assert!(!t.apply_live(&parse(&game(701.0, &[me.clone(), foe(9)])).unwrap(), &d, &crate::cooldown::CooldownRules::default()));
        assert_eq!(t.enemies[0].level, 9);
        assert_eq!(t.enemies[0].cooldowns.len(), 1);
        assert!(t.apply_live(&parse(&game(20.0, &[me, foe(1)])).unwrap(), &d, &crate::cooldown::CooldownRules::default()));
        assert!(t.enemies[0].cooldowns.is_empty());
    }

    #[test]
    fn roles_from_positions_smite_and_duplicates() {
        let d = GameData::embedded();
        assert_eq!(role_from_position("JUNGLE").as_deref(), Some("Jg"));
        assert_eq!(role_from_position("").as_deref(), None);
        let mut a = crate::tracker::test_enemy("Diana", 6, &[]);
        a.spell2_id = Some("SummonerSmite".into());
        let b = crate::tracker::test_enemy("Rakan", 6, &[]);
        let mut v = vec![a, b];
        complete_roles(&mut v, &d);
        assert_eq!(v[0].role.as_deref(), Some("Jg"));
        v[1].role = Some("Jg".into());
        complete_roles(&mut v, &d);
        assert!(v[0].role.is_none() && v[1].role.is_none());
    }

    #[test]
    fn ignores_garbage() {
        assert!(parse("").is_none());
        assert!(parse(r#"{"errorCode":"RESOURCE_NOT_FOUND","httpStatus":404}"#).is_none());
    }
}
