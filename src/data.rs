//! Game data (champions, summoner spells, summoner-spell haste items) from Riot's Data Dragon.
//! A snapshot is embedded at build time; a newer patch is downloaded and cached at runtime.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ChampionInfo {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SummonerSpellInfo {
    pub id: String,
    pub name: String,
    pub cooldown: f64,
    pub modes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ItemInfo {
    pub id: i64,
    pub name: String,
    pub summoner_haste: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GameData {
    pub version: String,
    pub champions: Vec<ChampionInfo>,
    pub spells: Vec<SummonerSpellInfo>,
    pub items: Vec<ItemInfo>,
    #[serde(skip)]
    index: Index,
}

#[derive(Clone, Debug, Default)]
struct Index {
    champ_by_id: HashMap<String, usize>,
    champ_by_name: HashMap<String, usize>,
    spell_by_id: HashMap<String, usize>,
    item_by_id: HashMap<i64, usize>,
}

const SNAPSHOT: &str = include_str!("../assets/snapshot.json");

impl GameData {
    pub fn from_json(json: &str) -> Option<GameData> {
        let mut d: GameData = serde_json::from_str(json).ok()?;
        if d.champions.is_empty() || d.spells.is_empty() {
            return None;
        }
        d.build_index();
        Some(d)
    }

    pub fn embedded() -> GameData {
        GameData::from_json(SNAPSHOT).expect("embedded snapshot is valid")
    }

    /// Cached newer data if present, otherwise the embedded snapshot.
    pub fn load(cache: &Path) -> GameData {
        std::fs::read_to_string(cache).ok().and_then(|s| GameData::from_json(&s)).unwrap_or_else(GameData::embedded)
    }

    fn build_index(&mut self) {
        let mut ix = Index::default();
        for (i, c) in self.champions.iter().enumerate() {
            ix.champ_by_id.insert(c.id.to_lowercase(), i);
            ix.champ_by_name.insert(c.name.to_lowercase(), i);
        }
        for (i, s) in self.spells.iter().enumerate() {
            ix.spell_by_id.insert(s.id.to_lowercase(), i);
        }
        for (i, it) in self.items.iter().enumerate() {
            ix.item_by_id.insert(it.id, i);
        }
        self.index = ix;
    }

    pub fn find_champion(&self, id_or_name: &str) -> Option<&ChampionInfo> {
        let k = id_or_name.trim().to_lowercase();
        if k.is_empty() {
            return None;
        }
        self.index.champ_by_id.get(&k).or_else(|| self.index.champ_by_name.get(&k)).map(|&i| &self.champions[i])
    }

    pub fn find_spell(&self, id: Option<&str>) -> Option<&SummonerSpellInfo> {
        let k = id?.to_lowercase();
        self.index.spell_by_id.get(&k).map(|&i| &self.spells[i])
    }

    pub fn find_item(&self, id: i64) -> Option<&ItemInfo> {
        self.index.item_by_id.get(&id).map(|&i| &self.items[i])
    }

    /// One entry per spell name usable in normal queues, preferring the Summoner's Rift version.
    pub fn standard_spells(&self) -> Vec<&SummonerSpellInfo> {
        let mut by_name: Vec<&SummonerSpellInfo> = Vec::new();
        for s in self.spells.iter().filter(|s| s.modes.iter().any(|m| m == "CLASSIC" || m == "ARAM")) {
            match by_name.iter().position(|x| x.name == s.name) {
                Some(i) => {
                    if !by_name[i].modes.iter().any(|m| m == "CLASSIC") && s.modes.iter().any(|m| m == "CLASSIC") {
                        by_name[i] = s;
                    }
                }
                None => by_name.push(s),
            }
        }
        by_name.sort_by(|a, b| a.name.cmp(&b.name));
        by_name
    }
}

// ---------- Data Dragon download ----------

const DDRAGON: &str = "https://ddragon.leagueoflegends.com";

fn get(url: &str) -> Result<String, String> {
    ureq::get(url).timeout(std::time::Duration::from_secs(30)).call().map_err(|e| e.to_string())?.into_string().map_err(|e| e.to_string())
}

pub fn latest_version() -> Result<String, String> {
    let v: Vec<String> = serde_json::from_str(&get(&format!("{DDRAGON}/api/versions.json"))?).map_err(|e| e.to_string())?;
    v.into_iter().next().ok_or_else(|| "empty versions list".into())
}

/// Downloads and caches newer data. Returns it only if the patch changed.
pub fn update(current_version: &str, cache: &Path) -> Result<Option<GameData>, String> {
    let latest = latest_version()?;
    if latest == current_version {
        return Ok(None);
    }
    let root = format!("{DDRAGON}/cdn/{latest}/data/en_US");
    let mut data = GameData {
        version: latest.clone(),
        champions: parse_champions(&get(&format!("{root}/champion.json"))?)?,
        spells: parse_spells(&get(&format!("{root}/summoner.json"))?)?,
        items: parse_items(&get(&format!("{root}/item.json"))?)?,
        index: Index::default(),
    };
    if data.champions.is_empty() || data.spells.is_empty() {
        return Ok(None);
    }
    if let Some(dir) = cache.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(cache, serde_json::to_string(&data).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    data.build_index();
    Ok(Some(data))
}

fn parse_champions(json: &str) -> Result<Vec<ChampionInfo>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (_, c) in v["data"].as_object().ok_or("no champion data")? {
        out.push(ChampionInfo {
            id: c["id"].as_str().unwrap_or_default().to_string(),
            name: c["name"].as_str().unwrap_or_default().to_string(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn parse_spells(json: &str) -> Result<Vec<SummonerSpellInfo>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (id, s) in v["data"].as_object().ok_or("no spell data")? {
        out.push(SummonerSpellInfo {
            id: id.clone(),
            name: s["name"].as_str().unwrap_or_default().to_string(),
            cooldown: s["cooldown"].get(0).and_then(|x| x.as_f64()).unwrap_or(0.0),
            modes: s["modes"].as_array().map(|a| a.iter().filter_map(|m| m.as_str().map(String::from)).collect()).unwrap_or_default(),
        });
    }
    Ok(out)
}

fn parse_items(json: &str) -> Result<Vec<ItemInfo>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (id, it) in v["data"].as_object().ok_or("no item data")? {
        let Ok(id) = id.parse::<i64>() else { continue };
        let sh = parse_summoner_haste(it["description"].as_str().unwrap_or_default());
        if sh == 0 {
            continue;
        }
        out.push(ItemInfo { id, name: it["name"].as_str().unwrap_or_default().to_string(), summoner_haste: sh });
    }
    Ok(out)
}

/// Data Dragon has no haste stat field, so summoner spell haste is read from the tooltip
/// ("Gain 10 Summoner Spell Haste" on Ionian Boots, Crimson Lucidity...).
pub fn parse_summoner_haste(desc: &str) -> i32 {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(\d+)\s+Summoner Spell Haste").unwrap());
    re.captures_iter(desc).filter_map(|c| c[1].parse::<i32>().ok()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_loads_with_haste_items() {
        let d = GameData::embedded();
        assert!(d.champions.len() > 150);
        for id in ["SummonerFlash", "SummonerTeleport", "SummonerDot", "SummonerBarrier", "SummonerExhaust", "SummonerHeal", "SummonerHaste", "SummonerBoost"] {
            assert!(d.find_spell(Some(id)).unwrap().cooldown > 0.0, "{id}");
        }
        assert_eq!(d.find_item(3158).unwrap().summoner_haste, 10);
        assert!(d.find_item(3161).map_or(true, |i| i.summoner_haste == 0));
        assert_eq!(d.find_champion("lee sin").unwrap().id, "LeeSin");
    }

    #[test]
    fn summoner_haste_is_parsed_from_tooltips() {
        assert_eq!(parse_summoner_haste("<stats><attention>10</attention> Ability Haste</stats><br>Gain 10 Summoner Spell Haste."), 10);
        assert_eq!(parse_summoner_haste("<stats><attention>15</attention> Ability Haste</stats>Gain 20 Ultimate Ability Haste."), 0);
    }
}
