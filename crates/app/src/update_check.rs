//! Lightweight update check, as in GitAgent and TrackCut.
//!
//! Fetches the `latest.json` published with each release and compares its version to this
//! build's `CARGO_PKG_VERSION`. Cheap and side-effect-free, so it runs in the background a few
//! seconds after start.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

/// Served from mayorana.ch alongside the builds it describes, so update checks don't depend on
/// the source repository staying publicly readable.
const LATEST_URL: &str = "https://mayorana.ch/downloads/polishcast/latest/latest.json";
/// Fallback when `latest.json` has no build for this OS (e.g. an Intel Mac: only Apple Silicon
/// is built), so the button leads somewhere to pick one instead of to a 404.
const RELEASES_URL: &str = "https://mayorana.ch/en/apps/polishcast";

/// Lets the download logs tell an existing user updating from a new install, and which versions
/// are still in use.
const USER_AGENT: &str = concat!("polishcast/", env!("CARGO_PKG_VERSION"), " (updater)");

#[derive(Deserialize)]
struct LatestJson {
    version: String,
    #[serde(default)]
    platforms: Platforms,
}

/// Builds per OS, keyed by package format (`dmg`, `exe_or_msi`…), not by CPU architecture. A
/// `BTreeMap` so the fallback pick is the same on every launch.
#[derive(Default, Deserialize)]
struct Platforms {
    #[serde(default)]
    macos: BTreeMap<String, Artifact>,
    #[serde(default)]
    windows: BTreeMap<String, Artifact>,
    #[serde(default)]
    linux: BTreeMap<String, Artifact>,
}

#[derive(Deserialize)]
struct Artifact {
    url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateInfo {
    pub latest_version: String,
    /// Direct link to this OS's build, so the banner downloads the build itself. The download
    /// happens in the user's browser, so there is no checksum to verify here.
    pub download_url: String,
}

/// `Some` if a newer release is published. Any network or parse failure is `None`; setting
/// `DISABLE_UPDATE_CHECK` turns the check off.
pub async fn check() -> Option<UpdateInfo> {
    if std::env::var_os("DISABLE_UPDATE_CHECK").is_some() {
        return None;
    }
    let body = tokio::task::spawn_blocking(fetch).await.ok()??;
    newer_than(&body, env!("CARGO_PKG_VERSION"), std::env::consts::OS)
}

fn fetch() -> Option<String> {
    let agent: ureq::Agent =
        polishcast_export::http::config().timeout_global(Some(Duration::from_secs(5))).build().into();
    agent.get(LATEST_URL).header("User-Agent", USER_AGENT).call().ok()?.body_mut().read_to_string().ok()
}

/// What `latest.json` says, if it names a release newer than `current`.
fn newer_than(body: &str, current: &str, os: &str) -> Option<UpdateInfo> {
    let latest: LatestJson = serde_json::from_str(body).ok()?;
    is_newer(&latest.version, current)
        .then(|| UpdateInfo { download_url: platform_url(os, &latest.platforms), latest_version: latest.version })
}

/// Formats to offer, best first, per OS.
fn preferred_formats(os: &str) -> &'static [&'static str] {
    match os {
        "macos" => &["dmg"],
        "windows" => &["msi", "exe", "exe_or_msi"],
        "linux" => &["appimage", "deb", "tarball"],
        _ => &[],
    }
}

/// The download link for `os`: the preferred format that is published, else any build for that
/// OS, else the app's page.
fn platform_url(os: &str, platforms: &Platforms) -> String {
    let by_format = match os {
        "macos" => &platforms.macos,
        "windows" => &platforms.windows,
        "linux" => &platforms.linux,
        _ => return RELEASES_URL.to_string(),
    };
    preferred_formats(os)
        .iter()
        .find_map(|format| by_format.get(*format))
        .or_else(|| by_format.values().next())
        .map(|a| a.url.as_str())
        .filter(|u| !u.is_empty())
        // Marks the hit as an update in the download logs: the browser fetches the file, not
        // this app, so the User-Agent above doesn't reach that request.
        .map(|u| format!("{u}?src=updater"))
        .unwrap_or_else(|| RELEASES_URL.to_string())
}

fn is_newer(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Option<(u32, u32, u32)> {
        let mut parts = s.trim_start_matches('v').split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.split(['-', '+']).next()?.parse().ok()?;
        Some((major, minor, patch))
    };
    matches!((parse(a), parse(b)), (Some(a), Some(b)) if a > b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape published today: a dmg for macOS, an installer for Windows, no Linux.
    const LATEST: &str = r#"{
      "name": "polishcast", "version": "0.1.3", "tag": "v0.1.3",
      "platforms": {
        "macos":   { "dmg": { "url": "https://mayorana.ch/downloads/polishcast/v0.1.3/polishcast-macos-arm64.dmg", "sha256": "x" } },
        "windows": { "exe_or_msi": { "url": "https://mayorana.ch/downloads/polishcast/v0.1.3/polishcast-setup.exe", "sha256": "y" } }
      }
    }"#;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("v0.2.0", "0.1.2"));
        assert!(!is_newer("0.1.2", "0.1.2"));
        assert!(!is_newer("0.1.1", "0.1.2"));
        assert!(!is_newer("garbage", "0.1.2"));
    }

    #[test]
    fn links_to_this_systems_build_and_marks_it_as_an_update() {
        let mac = newer_than(LATEST, "0.1.2", "macos").unwrap();
        assert_eq!(mac.latest_version, "0.1.3");
        assert_eq!(
            mac.download_url,
            "https://mayorana.ch/downloads/polishcast/v0.1.3/polishcast-macos-arm64.dmg?src=updater"
        );
        let win = newer_than(LATEST, "0.1.2", "windows").unwrap();
        assert!(win.download_url.ends_with("polishcast-setup.exe?src=updater"));
        // Nothing built for it: the app's page, to pick a build by hand.
        assert_eq!(newer_than(LATEST, "0.1.2", "linux").unwrap().download_url, RELEASES_URL);
    }

    #[test]
    fn says_nothing_when_up_to_date_or_unreadable() {
        assert_eq!(newer_than(LATEST, "0.1.3", "macos"), None);
        assert_eq!(newer_than("not json", "0.1.2", "macos"), None);
    }
}
