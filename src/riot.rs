//! Personal builds only (`--features personal`): reads the enemies' full rune pages from the Riot API
//! (Account-V1 for your PUUID, Spectator-V5 for the live game) to know who took Cosmic Insight.
//! The API key is the user's own, entered in Settings and stored encrypted with Windows DPAPI
//! (only this Windows user can decrypt it). It is never written in plain text or logged.

use std::path::PathBuf;
use std::time::Duration;

const COSMIC_INSIGHT: i64 = 8347;
const INSPIRATION: i64 = 8300;

/// Region code shown in Settings -> (platform host, regional host).
pub const REGIONS: &[(&str, &str, &str)] = &[
    ("NA", "na1", "americas"), ("EUW", "euw1", "europe"), ("EUNE", "eun1", "europe"), ("KR", "kr", "asia"),
    ("JP", "jp1", "asia"), ("BR", "br1", "americas"), ("LAN", "la1", "americas"), ("LAS", "la2", "americas"),
    ("OCE", "oc1", "sea"), ("TR", "tr1", "europe"), ("ME", "me1", "europe"),
];

#[derive(Clone, Debug)]
pub struct RunePage {
    /// "Name#TAG", as the Live Client API shows it.
    pub riot_id: String,
    pub cosmic_insight: bool,
    pub inspiration: bool,
}

fn key_path() -> PathBuf { crate::settings::data_dir().join("riot_key.bin") }

pub fn has_key() -> bool { key_path().exists() }

pub fn save_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() { return Err("empty key".into()); }
    let blob = crate::win::dpapi::protect(key.as_bytes()).ok_or("Windows could not encrypt the key")?;
    std::fs::create_dir_all(crate::settings::data_dir()).map_err(|e| e.to_string())?;
    std::fs::write(key_path(), blob).map_err(|e| e.to_string())
}

pub fn remove_key() { let _ = std::fs::remove_file(key_path()); }

fn load_key() -> Option<String> {
    let blob = std::fs::read(key_path()).ok()?;
    String::from_utf8(crate::win::dpapi::unprotect(&blob)?).ok()
}

fn agent() -> Option<ureq::Agent> {
    let tls = native_tls::TlsConnector::new().ok()?;
    Some(ureq::AgentBuilder::new().tls_connector(std::sync::Arc::new(tls)).timeout(Duration::from_secs(8)).user_agent("Flashless").build())
}

fn get(agent: &ureq::Agent, url: &str, key: &str) -> Result<serde_json::Value, String> {
    match agent.get(url).set("X-Riot-Token", key).call() {
        Ok(r) => serde_json::from_str(&r.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string()),
        Err(ureq::Error::Status(401 | 403, _)) => Err("the API key is invalid or expired (development keys last 24 h)".into()),
        Err(ureq::Error::Status(404, _)) => Err("not found".into()),
        Err(ureq::Error::Status(429, _)) => Err("rate limited, try again in a minute".into()),
        Err(ureq::Error::Status(c, _)) => Err(format!("Riot API error {c}")),
        Err(e) => Err(format!("no connection to the Riot API ({e})")),
    }
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Rune pages of everyone not on your team in your current game. Blocking; call from a background thread.
pub fn enemy_runes(region: &str, my_riot_id: &str) -> Result<Vec<RunePage>, String> {
    let key = load_key().ok_or("no API key saved")?;
    let (_, platform, regional) = REGIONS.iter().find(|r| r.0.eq_ignore_ascii_case(region)).copied().ok_or("unknown region")?;
    let (name, tag) = my_riot_id.rsplit_once('#').ok_or("your Riot ID is not known yet")?;
    let agent = agent().ok_or("no HTTPS support")?;
    let acct = get(&agent, &format!("https://{regional}.api.riotgames.com/riot/account/v1/accounts/by-riot-id/{}/{}", enc(name), enc(tag)), &key)
        .map_err(|e| if e == "not found" { "your Riot ID was not found in this region".into() } else { e })?;
    let puuid = acct["puuid"].as_str().ok_or("unexpected account reply")?;
    let game = get(&agent, &format!("https://{platform}.api.riotgames.com/lol/spectator/v5/active-games/by-summoner/{puuid}"), &key)
        .map_err(|e| if e == "not found" { "no live game found (bot and practice games are not listed)".into() } else { e })?;
    parse_game(&game, puuid)
}

fn parse_game(game: &serde_json::Value, my_puuid: &str) -> Result<Vec<RunePage>, String> {
    let players = game["participants"].as_array().ok_or("unexpected game reply")?;
    let my_team = players.iter().find(|p| p["puuid"].as_str() == Some(my_puuid)).and_then(|p| p["teamId"].as_i64());
    Ok(players.iter()
        .filter(|p| my_team.is_none() || p["teamId"].as_i64() != my_team)
        .map(|p| {
            let perks: Vec<i64> = p["perks"]["perkIds"].as_array().map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
            let styles = [p["perks"]["perkStyle"].as_i64(), p["perks"]["perkSubStyle"].as_i64()];
            RunePage {
                riot_id: p["riotId"].as_str().unwrap_or_default().to_string(),
                cosmic_insight: perks.contains(&COSMIC_INSIGHT),
                inspiration: styles.contains(&Some(INSPIRATION)),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_enemy_rune_pages() {
        let g: serde_json::Value = serde_json::from_str(r#"{"participants":[
            {"puuid":"me","teamId":100,"riotId":"Me#NA1","perks":{"perkIds":[8347],"perkStyle":8300,"perkSubStyle":8000}},
            {"puuid":"a","teamId":200,"riotId":"Foe One#NA1","perks":{"perkIds":[8010,8347,8304],"perkStyle":8000,"perkSubStyle":8300}},
            {"puuid":"b","teamId":200,"riotId":"Foe Two#EUW","perks":{"perkIds":[8112,8139],"perkStyle":8100,"perkSubStyle":8200}}]}"#).unwrap();
        let r = parse_game(&g, "me").unwrap();
        assert_eq!(r.len(), 2);
        assert!(r[0].cosmic_insight && r[0].inspiration);
        assert!(!r[1].cosmic_insight && !r[1].inspiration);
        assert_eq!(enc("Foe One"), "Foe%20One");
    }
}
