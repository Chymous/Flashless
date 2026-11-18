//! Parses OCR'd chat lines such as "19:15 Player (Caitlyn): Diana used Flash" or
//! (posted by League when a player clicks an enemy's summoner spell). Ultimate lines ("used R - ...")
//! are ignored: Flashless does not track ultimates. OCR is noisy, so names are fuzzy-matched against the
//! five enemies only.

use crate::data::GameData;
use crate::tracker::{Enemy, Slot};
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct Detection {
    pub enemy: usize,
    pub slot: Slot,
    pub minutes: Option<i32>,
    pub seconds: Option<i32>,
}

fn used_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // "used", with the usual OCR slips (c/e, cl/d) and a lost space before the spell ("used8arrier")
    R.get_or_init(|| Regex::new(r"\b([uU][sS5][eEcC](?:[dD]|cl|c))(?:[^a-z]|$)").unwrap())
}

fn time_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // after the prefix is cleaned: "04:11", "03/28", "15.'44", "11.'56", "15„'44"
    R.get_or_init(|| Regex::new(r"^\D{0,3}(\d{1,2})[^\d]{1,3}(\d)(\d)(?:\D|$)").unwrap())
}

pub fn parse(line: &str, enemies: &[Enemy], data: &GameData) -> Option<Detection> {
    if enemies.is_empty() || line.trim().is_empty() { return None; }
    let word = used_re().captures(line)?.get(1)?;
    let before = &line[..word.start()];
    let after = &line[word.end()..];
    // only the text after the sender ("Name (Champ): ") names the enemy
    let champ_text = before.rfind([':', ')', '}', ']']).map(|i| &before[i + 1..]).unwrap_or(before);
    let enemy = match_enemy(champ_text, enemies)?;
    let slot = match_ability(after, &enemies[enemy], data)?;
    let (minutes, seconds) = parse_timestamp(line);
    Some(Detection { enemy, slot, minutes, seconds })
}

/// The leading timestamp as OCR sees it, cleaned: tokens are taken while they look like part of a time
/// ("1 5:44", "I I .'56" -> "15:44", "11.'56"), with I/l/| read as 1 and O as 0, spaces dropped.
fn time_prefix(line: &str) -> String {
    let mut out = String::new();
    for tok in line.split_whitespace() {
        let t: String = tok.chars().map(|c| match c { 'I' | 'l' | '|' | '!' => '1', 'O' | 'o' => '0', c => c }).collect();
        if t.chars().any(|c| c.is_alphabetic()) { break; }
        out.push_str(&t);
        if out.chars().filter(|c| c.is_ascii_digit()).count() >= 5 { break; }
    }
    out
}

pub fn parse_timestamp(line: &str) -> (Option<i32>, Option<i32>) {
    let p = time_prefix(line);
    if let Some(c) = time_re().captures(&p) {
        let ss = c[2].parse::<i32>().unwrap() * 10 + c[3].parse::<i32>().unwrap();
        if ss < 60 { return (c[1].parse().ok(), Some(ss)); }
    }
    let d: Vec<i32> = p.chars().filter(|c| c.is_ascii_digit()).map(|c| c as i32 - '0' as i32).collect();
    let ss = |i: usize| { let s = d[i] * 10 + d[i + 1]; (s < 60).then_some(s) };
    match d.len() {
        // separator lost ("1544") or read as a digit ("15144"): minutes from the front, seconds from the back
        4 => if let Some(s) = ss(2) { return (Some(d[0] * 10 + d[1]), Some(s)); },
        5 => if let Some(s) = ss(3) { return (Some(d[0] * 10 + d[1]), Some(s)); },
        n if n >= 2 => return (None, ss(n - 2)),
        _ => {}
    }
    (None, None)
}

fn letters(s: &str) -> String { s.chars().filter(|c| c.is_alphabetic()).flat_map(|c| c.to_lowercase()).collect() }

fn words(s: &str) -> Vec<String> { s.split(' ').map(letters).filter(|w| !w.is_empty()).collect() }

fn match_enemy(text: &str, enemies: &[Enemy]) -> Option<usize> {
    let w = words(text);
    if w.is_empty() { return None; }
    // last 1, 2 and 3 words ("Jinx", "Lee Sin", "Aurelion Sol")
    let cands: Vec<String> = (1..=w.len().min(3)).map(|n| w[w.len() - n..].concat()).collect();
    let options: Vec<(usize, Vec<String>)> = enemies.iter().enumerate().map(|(i, e)| (i, vec![letters(&e.name), letters(&e.champion_id)])).collect();
    pick_closest(&options, &cands, false)
}

fn match_ability(text: &str, e: &Enemy, data: &GameData) -> Option<Slot> {
    let first = text.trim_start().split(' ').next().unwrap_or_default();
    if letters(first) == "r" && first.trim_matches(|c| "-,.:;".contains(c)).chars().count() <= 1 {
        return None; // "used R - ...": ultimates are not tracked
    }
    let w: Vec<String> = words(text).into_iter().take(4).collect();
    if w.is_empty() { return None; }
    let cands: Vec<String> = (1..=w.len()).map(|n| w[..n].concat()).collect();
    let mut options: Vec<(Slot, Vec<String>)> = Vec::new();
    for slot in [Slot::Spell1, Slot::Spell2] {
        if let Some(sp) = data.find_spell(e.spell_id(slot)) { options.push((slot, vec![letters(&sp.name)])); }
    }
    pick_closest(&options, &cands, true)
}

/// Closest option by edit distance; accepted within the normal tolerance, or within half the name's length
/// when it beats every other option by at least two edits.
fn pick_closest<T: Copy>(options: &[(T, Vec<String>)], cands: &[String], ends_with: bool) -> Option<T> {
    let mut scored: Vec<(T, usize, usize)> = Vec::new();
    for (opt, names) in options {
        let mut best = (usize::MAX, 0);
        for name in names.iter().filter(|n| !n.is_empty()) {
            for c in cands {
                let d = if ends_with && c.ends_with(name.as_str()) { 0 } else { distance(c, name) };
                if d < best.0 { best = (d, name.chars().count()); }
            }
        }
        if best.0 != usize::MAX { scored.push((*opt, best.0, best.1)); }
    }
    if scored.is_empty() { return None; }
    scored.sort_by_key(|x| x.1);
    let (opt, dist, len) = scored[0];
    let runner_up = scored.get(1).map(|x| x.1).unwrap_or(usize::MAX);
    if dist == runner_up { return None; }
    if dist <= tolerance(len) { return Some(opt); }
    if dist <= (len + 1) / 2 && runner_up - dist >= 2 { return Some(opt); }
    None
}

/// Measured OCR slips: "Udyr" -> "I.Jdyr" (2 edits), "Flash" -> "Flish", "Nautilus" -> "Naucilus".
fn tolerance(len: usize) -> usize { if len <= 3 { 1 } else if len <= 7 { 2 } else { 3 } }

pub fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            cur[j] = (cur[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Decides which detections are new uses: a line stays on screen and reappears with chat history,
/// so it must count once, and only while it is fresh.
#[derive(Default)]
pub struct Filter {
    seen: HashMap<(String, Slot, i32), f64>,
}

pub const FRESH_WINDOW: f64 = 10.0;
const REPEAT_WINDOW: f64 = 120.0;

impl Filter {
    pub fn reset(&mut self) { self.seen.clear(); }

    /// Seconds since the use, or Err(reason) if the line is ignored. Seconds-only timestamps are ambiguous
    /// by whole minutes, so such a line counts only on the scan where it first appears.
    pub fn accept(&mut self, d: &Detection, champ: &str, game_time: f64, remaining: f64, just_appeared: bool) -> Result<f64, String> {
        let now = game_time.floor();
        let (stamp, ago) = match (d.minutes, d.seconds) {
            (Some(mm), Some(ss)) => {
                let stamp = mm * 60 + ss;
                let ago = now - stamp as f64;
                if ago < -3.0 {
                    // minutes misread ("607'.25" = 60:25 at 7:27): judge it by the seconds alone
                    let mut d2 = d.clone();
                    d2.minutes = None;
                    return self.accept(&d2, champ, game_time, remaining, just_appeared)
                        .map_err(|e| format!("timestamp {mm}:{ss:02} is ahead of the game clock, {e}"));
                }
                let ago = ago.max(0.0);
                if ago > FRESH_WINDOW { return Err(format!("old line ({ago:.0}s ago)")); }
                (stamp, ago)
            }
            (None, Some(ss)) => {
                if !just_appeared { return Err("only seconds readable and the line was already on screen".into()); }
                let ago = ((now - ss as f64) % 60.0 + 60.0) % 60.0;
                if ago > FRESH_WINDOW { return Err(format!("old line (:{ss:02})")); }
                (-1 - ss, ago)
            }
            _ => return Err("no readable timestamp".into()),
        };
        let key = (champ.to_string(), d.slot, stamp);
        if self.seen.get(&key).is_some_and(|&t| game_time - t < REPEAT_WINDOW) { return Err("already counted".into()); }
        if remaining > 15.0 { return Err(format!("already on cooldown ({remaining:.0}s left)")); }
        self.seen.insert(key, game_time);
        Ok(ago)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::test_enemy;

    fn roster(spec: &[(&str, &str, &str)]) -> Vec<Enemy> {
        spec.iter().map(|(c, s1, s2)| {
            let mut e = test_enemy(c, 6, &[]);
            e.spell1_id = Some(s1.to_string());
            e.spell2_id = Some(s2.to_string());
            e
        }).collect()
    }

    fn live_roster() -> Vec<Enemy> {
        roster(&[("Soraka", "SummonerFlash", "SummonerHeal"), ("Udyr", "SummonerFlash", "SummonerSmite"), ("Ezreal", "SummonerBarrier", "SummonerFlash"),
                 ("TwistedFate", "SummonerFlash", "SummonerTeleport"), ("Rumble", "SummonerFlash", "SummonerDot")])
    }

    fn roster3() -> Vec<Enemy> {
        roster(&[("Hecarim", "SummonerHaste", "SummonerSmite"), ("Galio", "SummonerFlash", "SummonerTeleport"), ("Rumble", "SummonerFlash", "SummonerDot"),
                 ("Lulu", "SummonerFlash", "SummonerHeal"), ("Varus", "SummonerBarrier", "SummonerFlash")])
    }

    #[test]
    fn parses_real_ocr_lines() {
        let d = GameData::embedded();
        let cases: &[(&str, &str, Slot, Option<i32>)] = &[
            ("02:51 Player (Talon): I.Jdyr used Flish", "Udyr", Slot::Spell1, Some(51)),
            ("16:36 Player (Talon): CJdyr used Flash", "Udyr", Slot::Spell1, Some(36)),
            ("1440 Player (Talon): Ezreal used Barrier", "Ezreal", Slot::Spell1, Some(40)),
            ("15.03 Player (Talan).' Soraka used Heal", "Soraka", Slot::Spell2, Some(3)),
            ("15:22 Player (Talan).' Ezreal used Flash", "Ezreal", Slot::Spell2, Some(22)),
        ];
        let r = live_roster();
        for (line, champ, slot, sec) in cases {
            let det = parse(line, &r, &d).unwrap_or_else(|| panic!("no match: {line}"));
            assert_eq!(r[det.enemy].champion_id, *champ, "{line}");
            assert_eq!(det.slot, *slot, "{line}");
            assert_eq!(det.seconds, *sec, "{line}");
        }
        let r3 = roster3();
        for (line, champ, slot) in [
            ("00;23 Player (Talon); Varus used earner", "Varus", Slot::Spell1),
            ("00:42 Player (Talon): Calio used Flash", "Galio", Slot::Spell1),
            ("00:43 Player (Talon): Gano used flash", "Galio", Slot::Spell1),
            ("03:28 Player (Talon): Varus used8arrier", "Varus", Slot::Spell1),
        ] {
            let det = parse(line, &r3, &d).unwrap_or_else(|| panic!("no match: {line}"));
            assert_eq!(r3[det.enemy].champion_id, champ, "{line}");
            assert_eq!(det.slot, slot, "{line}");
        }
    }

    #[test]
    fn ignores_other_lines() {
        let d = GameData::embedded();
        let r = roster3();
        for line in ["00:24 Player (Talon): Lulu Flash", "00:36Player (Talon): Galio used", "00:23 Player (Talon). used Flash",
                     "03:09 Player (Talon) has slain Galio Bot (Galio) for a double kill!", "You must wait before issuing more pings.",
                     "00:27 Player (Talon): Ahri used Flash", "02:36 [Team] Player: Mid 2134f",
                     "04:1 1 Player (Talon): Hecarim used R - Onslaught of Shadows", "04:11 Player (Talon): Hecarim used R, Onslaught of Shadows"] {
            assert!(parse(line, &r, &d).is_none(), "{line}");
        }
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_timestamp("04:1 1 Player"), (Some(4), Some(11)));
        assert_eq!(parse_timestamp("03/28 Player"), (Some(3), Some(28)));
        assert_eq!(parse_timestamp("15.03 Player"), (Some(15), Some(3)));
        assert_eq!(parse_timestamp("80123 Player"), (Some(80), Some(23)));
        assert_eq!(parse_timestamp("023 Player"), (None, Some(23)));
        // from a live game (small chat at 1440p)
        assert_eq!(parse_timestamp("1 5:44 Ally (Jhin): Graves used Flash"), (Some(15), Some(44)));
        assert_eq!(parse_timestamp("15.'44 Ally (Jhin): Graves used Flash"), (Some(15), Some(44)));
        assert_eq!(parse_timestamp("15„'44 Ally (Jhin): Graves used Flash"), (Some(15), Some(44)));
        assert_eq!(parse_timestamp("17'.01 Teammate (Viktor): Diana used Flash"), (Some(17), Some(1)));
        assert_eq!(parse_timestamp("1 5144 Ally (Jhin): Graves used Flash"), (Some(15), Some(44)));
        assert_eq!(parse_timestamp("1544 Ally (Jhin): Graves used Flash"), (Some(15), Some(44)));
        assert_eq!(parse_timestamp("1 1.'56 Teammate (Viktor): Renata Glasc used Heal"), (Some(11), Some(56)));
        assert_eq!(parse_timestamp("I I .'56 Teammate (Viktor): Renata Glasc used Heal"), (Some(11), Some(56)));
        assert_eq!(parse_timestamp("15:44 Ilovecats (Jhin): Graves used Flash"), (Some(15), Some(44)));
        assert_eq!(parse_timestamp("hin): Graves used Flash"), (None, None));
        assert_eq!(parse_timestamp("Player"), (None, None));
    }

    #[test]
    fn filter_rejects_old_and_repeated_lines() {
        let d = GameData::embedded();
        let r = roster3();
        let varus = parse("03:28 Player (Talon): Varus used Barrier", &r, &d).unwrap();
        assert!(Filter::default().accept(&varus, "Varus", 269.4, 0.0, true).is_err()); // a minute old
        let mut f = Filter::default();
        assert_eq!(f.accept(&varus, "Varus", 209.2, 0.0, true), Ok(1.0));
        assert!(f.accept(&varus, "Varus", 210.0, 0.0, true).is_err());
        assert!(Filter::default().accept(&varus, "Varus", 120.0, 0.0, true).is_err()); // ahead of clock
        let secs = parse("023 Player (Talon): Varus used Flash", &r, &d).unwrap();
        assert!(Filter::default().accept(&secs, "Varus", 24.5, 0.0, false).is_err());
        assert_eq!(Filter::default().accept(&secs, "Varus", 24.5, 0.0, true), Ok(1.0));
        assert!(Filter::default().accept(&secs, "Varus", 24.5, 250.0, true).is_err());
    }
}
