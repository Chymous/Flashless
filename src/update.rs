//! Optional update check: asks GitHub for the latest release of `REPO` and reports it if newer.
//! Does nothing while `REPO` is empty. Sends no data beyond a plain HTTPS GET with a generic user agent.

/// "owner/name" of the public repository that hosts Flashless releases. Empty: no update check.
pub const REPO: &str = "";

pub struct Release { pub version: String, pub url: String }

/// "v1.2.3" / "1.2.3" -> (1, 2, 3)
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.trim().trim_start_matches(['v', 'V']).split('.').map(|x| x.parse::<u32>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(a), Some(b)) if a > b)
}

/// Blocking; call from a background thread.
pub fn check() -> Option<Release> {
    if REPO.is_empty() { return None; }
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = ureq::get(&url).set("User-Agent", "Flashless-update-check").set("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(8)).call().ok()?.into_string().ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let tag = v["tag_name"].as_str()?.to_string();
    let page = v["html_url"].as_str()?.to_string();
    is_newer(&tag, env!("CARGO_PKG_VERSION")).then(|| Release { version: tag.trim_start_matches(['v', 'V']).to_string(), url: page })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions() {
        assert!(is_newer("v1.0.3", "1.0.2"));
        assert!(is_newer("1.1", "1.0.9"));
        assert!(!is_newer("v1.0.2", "1.0.2"));
        assert!(!is_newer("garbage", "1.0.2"));
    }
}
