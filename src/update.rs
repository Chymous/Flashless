//! Update check and in-app update: asks GitHub for the latest release, and on request downloads its
//! Flashless-Setup.exe, checks it against the size and SHA-256 that GitHub lists, and runs it silently.
//! Requests carry nothing but a generic user agent.

use std::path::PathBuf;
use std::time::Duration;

/// "owner/name" of the public repository that hosts Flashless releases. Empty: no update check.
pub const REPO: &str = "Chymous/Flashless";
const ASSET: &str = "Flashless-Setup.exe";
const UA: &str = "Flashless-updater";

/// HTTPS with the Windows certificate store and normal certificate checks.
fn agent(timeout: u64) -> Option<ureq::Agent> {
    let tls = native_tls::TlsConnector::new().ok()?;
    Some(ureq::AgentBuilder::new().tls_connector(std::sync::Arc::new(tls)).timeout(Duration::from_secs(timeout)).user_agent(UA).build())
}

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    /// The release page, for the "what's new" link.
    pub page: String,
    pub download: String,
    pub size: u64,
    /// Lowercase hex, when GitHub lists one.
    pub sha256: Option<String>,
}

/// "v1.2.3" / "1.2.3" -> (1, 2, 3)
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.trim().trim_start_matches(['v', 'V']).split('.').map(|x| x.parse::<u32>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(a), Some(b)) if a > b)
}

/// The latest release's setup file, if it is newer than this build. Blocking; call from a background thread.
pub fn check() -> Option<Release> {
    latest().filter(|r| is_newer(&r.version, env!("CARGO_PKG_VERSION")))
}

/// The latest published release with a setup file, newer or not. Blocking.
pub fn latest() -> Option<Release> {
    if REPO.is_empty() { return None; }
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = agent(8)?.get(&url).set("Accept", "application/vnd.github+json").call().ok()?.into_string().ok()?;
    parse_release(&body)
}

fn parse_release(body: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) { return None; }
    let version = v["tag_name"].as_str()?.trim_start_matches(['v', 'V']).to_string();
    let asset = v["assets"].as_array()?.iter().find(|a| a["name"].as_str() == Some(ASSET))?;
    let download = asset["browser_download_url"].as_str()?.to_string();
    // only ever download from this repository's own release files
    if !download.starts_with(&format!("https://github.com/{REPO}/releases/download/")) { return None; }
    Some(Release {
        version,
        page: v["html_url"].as_str().unwrap_or_default().to_string(),
        download,
        size: asset["size"].as_u64()?,
        sha256: asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(|h| h.to_ascii_lowercase()),
    })
}

/// Downloads the setup file to %TEMP%\Flashless-Update and checks it. Blocking.
pub fn download(r: &Release) -> Result<PathBuf, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let resp = agent(120).ok_or("no HTTPS support")?.get(&r.download).call().map_err(|e| format!("download failed: {e}"))?;
    let mut bytes = Vec::with_capacity(r.size as usize);
    resp.into_reader().take(64 * 1024 * 1024).read_to_end(&mut bytes).map_err(|e| format!("download failed: {e}"))?;
    if bytes.len() as u64 != r.size { return Err("download incomplete, try again".into()); }
    if let Some(want) = &r.sha256 {
        let got: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        if &got != want { return Err("downloaded file failed its checksum, not installing".into()); }
    }
    if bytes.get(..2) != Some(b"MZ") { return Err("downloaded file is not a Windows program".into()); }
    let dir = std::env::temp_dir().join("Flashless-Update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(ASSET);
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Removes the setup left in %TEMP% by the last in-app update (call once on start).
pub fn cleanup() {
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("Flashless-Update"));
}

/// Runs the downloaded setup silently: it closes Flashless, replaces it, keeps the user's shortcut
/// choices and settings, and starts the new version.
pub fn run_installer(setup: &std::path::Path) -> Result<(), String> {
    let mut args = vec!["--silent", "--relaunch"];
    if !crate::win::shell::has_desktop_link() { args.push("--no-desktop"); }
    if !crate::win::shell::has_start_menu_link() { args.push("--no-start-menu"); }
    std::process::Command::new(setup).args(&args).current_dir(std::env::temp_dir()).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(is_newer("v1.0.4", "1.0.3"));
        assert!(is_newer("1.1", "1.0.9"));
        assert!(!is_newer("v1.0.3", "1.0.3"));
        assert!(!is_newer("garbage", "1.0.3"));
    }

    #[test]
    fn reads_release_and_rejects_foreign_downloads() {
        let ok = r#"{"tag_name":"v1.0.4","html_url":"https://github.com/Chymous/Flashless/releases/tag/v1.0.4","draft":false,"prerelease":false,
            "assets":[{"name":"Flashless-Setup.exe","size":123,"digest":"sha256:ABCD",
            "browser_download_url":"https://github.com/Chymous/Flashless/releases/download/v1.0.4/Flashless-Setup.exe"}]}"#;
        let r = parse_release(ok).unwrap();
        assert_eq!((r.version.as_str(), r.size, r.sha256.as_deref()), ("1.0.4", 123, Some("abcd")));
        let foreign = ok.replace("https://github.com/Chymous/Flashless/releases/download", "https://example.com/x");
        assert!(parse_release(&foreign).is_none());
        assert!(parse_release(&ok.replace("\"prerelease\":false", "\"prerelease\":true")).is_none());
        assert!(parse_release(&ok.replace("Flashless-Setup.exe\",\"size", "Other.exe\",\"size")).is_none());
    }
}
