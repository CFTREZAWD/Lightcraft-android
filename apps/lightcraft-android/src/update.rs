//! Updates for LightCraft for Android: which release to offer, and the update settings.
//!
//! Releases are this repository's GitHub Releases tagged `android-v<version>`, each with an
//! `…-arm64.apk` asset. The newest one above the running version is offered (betas only when the
//! user wants them, never a version they skipped). The APK is downloaded with its size and
//! SHA-256 checked (GitHub reports each asset's digest), then handed to Android's package
//! installer, which also refuses an APK not signed with the installed app's key.
//!
//! The network and the installer live in the Android host (`android/updater.rs`); this module is
//! plain data, so it is tested without a device.

use std::cmp::Ordering;
use std::path::Path;

use serde_json::Value;

/// The repository releases come from (`owner/name`); a build for a fork sets
/// `LIGHTCRAFT_UPDATE_REPO`.
pub const REPO: &str = match option_env!("LIGHTCRAFT_UPDATE_REPO") {
    Some(r) => r,
    None => "CFTREZAWD/Lightcraft-android",
};

/// The version this build is: the release's (`LIGHTCRAFT_ANDROID_VERSION`, set by the release
/// build from its tag, e.g. `0.21.1-beta.2`), else the workspace's.
pub const CURRENT: &str = match option_env!("LIGHTCRAFT_ANDROID_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

/// The tag prefix of Android releases.
pub const TAG_PREFIX: &str = "android-v";

/// How often the check at start runs at most (seconds).
pub const CHECK_INTERVAL: i64 = 20 * 60 * 60;

/// A version `major.minor.patch[-pre]`, ordered as semantic versions are (a pre-release comes
/// before its release; `beta.10` after `beta.9`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub core: [u64; 3],
    pub pre: Vec<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim().trim_start_matches(TAG_PREFIX).trim_start_matches('v');
        let s = s.split('+').next()?;
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
        let v = [parts.next()??, parts.next().unwrap_or(Some(0))?, parts.next().unwrap_or(Some(0))?];
        if parts.next().is_some() {
            return None;
        }
        let pre: Vec<String> = pre.map(|p| p.split('.').map(str::to_string).collect()).unwrap_or_default();
        if pre.iter().any(|p| p.is_empty()) {
            return None;
        }
        Some(Version { core: v, pre })
    }

    pub fn is_pre(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&other.pre) {
                    let o = match (a.parse::<u64>(), b.parse::<u64>()) {
                        (Ok(x), Ok(y)) => x.cmp(&y),
                        (Ok(_), Err(_)) => Ordering::Less,
                        (Err(_), Ok(_)) => Ordering::Greater,
                        (Err(_), Err(_)) => a.cmp(b),
                    };
                    if o != Ordering::Equal {
                        return o;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.core[0], self.core[1], self.core[2])?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

/// A release that can be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub name: String,
    /// The release notes (Markdown).
    pub notes: String,
    pub page: String,
    pub apk_name: String,
    pub apk_url: String,
    pub size: u64,
    /// Lower-case hex SHA-256 of the APK, when GitHub reports it.
    pub sha256: Option<String>,
}

/// The largest APK accepted (a hostile listing can't make the app fill the disk).
pub const MAX_APK: u64 = 512 * 1024 * 1024;

fn release_of(r: &Value) -> Option<Release> {
    if r.get("draft").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let tag = r.get("tag_name")?.as_str()?;
    if !tag.starts_with(TAG_PREFIX) {
        return None;
    }
    let mut version = Version::parse(tag)?;
    // a release GitHub marks as a pre-release is one, whatever its tag says
    if r.get("prerelease").and_then(Value::as_bool).unwrap_or(false) && !version.is_pre() {
        version.pre = vec!["pre".into()];
    }
    let asset = r.get("assets")?.as_array()?.iter().find(|a| {
        a.get("name").and_then(Value::as_str).is_some_and(|n| n.ends_with("-arm64.apk"))
            && a.get("state").and_then(Value::as_str).is_none_or(|s| s == "uploaded")
    })?;
    let apk_url = asset.get("browser_download_url")?.as_str()?;
    if !apk_url.starts_with("https://github.com/") {
        return None;
    }
    let size = asset.get("size")?.as_u64().filter(|s| (1..=MAX_APK).contains(s))?;
    let sha256 = asset
        .get("digest")
        .and_then(Value::as_str)
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase);
    Some(Release {
        tag: tag.to_string(),
        name: r.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or(tag).to_string(),
        notes: r.get("body").and_then(Value::as_str).unwrap_or_default().to_string(),
        page: r.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
        apk_name: asset.get("name")?.as_str()?.to_string(),
        apk_url: apk_url.to_string(),
        size,
        sha256,
        version,
    })
}

/// The release to offer from GitHub's release list (`GET /repos/{repo}/releases`): the newest
/// above `current` that is a stable release, or a beta when `betas`, and not `skipped`.
pub fn newest(list: &Value, current: &Version, betas: bool, skipped: Option<&str>) -> Option<Release> {
    list.as_array()?
        .iter()
        .filter_map(release_of)
        .filter(|r| betas || !r.version.is_pre())
        .filter(|r| r.version > *current)
        .filter(|r| skipped != Some(r.tag.as_str()))
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// What the user chose about updates, kept in `<config>/android-updates.json`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Look for a new release when the app starts (at most every [`CHECK_INTERVAL`]).
    pub check_at_start: bool,
    /// Offer beta releases too.
    pub betas: bool,
    /// The release the user chose to skip (its tag).
    pub skipped: Option<String>,
    /// When the check at start last ran (Unix seconds).
    pub last_check: i64,
}

impl Default for Settings {
    fn default() -> Self {
        // the Android port is in beta: every release so far is one
        Settings { check_at_start: true, betas: true, skipped: None, last_check: 0 }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Settings {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        crate::prefs::write_atomic(path, &bytes).map_err(|e| format!("saving the update settings failed: {e}"))
    }

    /// Whether the check at start is due at `now` (Unix seconds).
    pub fn due(&self, now: i64) -> bool {
        self.check_at_start && (now - self.last_check >= CHECK_INTERVAL || now < self.last_check)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|| Version { core: [0, 0, 0], pre: vec!["unparsed".into()] })
    }

    #[test]
    fn versions_order_as_releases_do() {
        let ordered = ["0.21.1-alpha.2", "0.21.1-beta.1", "0.21.1-beta.2", "0.21.1-beta.10", "0.21.1-rc.1", "0.21.1", "0.21.2", "0.22.0", "1.0.0"];
        for w in ordered.windows(2) {
            assert!(v(w[0]) < v(w[1]), "{} < {}", w[0], w[1]);
        }
        assert_eq!(v("android-v0.21.1-beta.1"), v("0.21.1-beta.1"));
        assert!(Version::parse("android-vx").is_none());
        assert!(Version::parse("1.2.3.4").is_none());
    }

    fn listing() -> Value {
        let rel = |tag: &str, pre: bool, draft: bool| {
            serde_json::json!({
                "tag_name": tag, "prerelease": pre, "draft": draft, "name": tag, "body": "notes",
                "html_url": format!("https://github.com/o/r/releases/tag/{tag}"),
                "assets": [
                    {"name": format!("lightcraft-android-{}-arm64.apk", tag.trim_start_matches(TAG_PREFIX)),
                     "browser_download_url": format!("https://github.com/o/r/releases/download/{tag}/x-arm64.apk"),
                     "size": 1000, "state": "uploaded", "digest": format!("sha256:{}", "ab".repeat(32))},
                    {"name": "x.apk.sha256", "browser_download_url": "https://github.com/x", "size": 10}
                ]
            })
        };
        serde_json::json!([
            rel("android-v0.21.1-beta.1", true, false),
            rel("android-v0.21.1-beta.3", true, false),
            rel("android-v0.22.0", false, true),
            rel("v9.9.9", false, false),
            rel("android-v0.21.0", false, false),
        ])
    }

    #[test]
    fn the_newest_eligible_release_is_offered() {
        let list = listing();
        let cur = v("0.21.1-beta.1");
        let r = newest(&list, &cur, true, None);
        // the draft and the desktop release are never offered
        assert_eq!(r.as_ref().map(|r| r.tag.as_str()), Some("android-v0.21.1-beta.3"));
        assert_eq!(r.and_then(|r| r.sha256).map(|h| h.len()), Some(64));
        // without betas nothing above the running beta is stable
        assert!(newest(&list, &cur, false, None).is_none());
        // a skipped release isn't offered again
        assert!(newest(&list, &cur, true, Some("android-v0.21.1-beta.3")).is_none());
        // the running version isn't offered to itself
        assert!(newest(&list, &v("0.21.1-beta.3"), true, None).is_none());
    }

    #[test]
    fn a_listing_cannot_point_the_download_elsewhere() {
        let list = serde_json::json!([{
            "tag_name": "android-v9.0.0", "assets": [{"name": "a-arm64.apk", "size": 10,
            "browser_download_url": "http://evil.example/a-arm64.apk"}]
        }]);
        assert!(newest(&list, &v("0.1.0"), true, None).is_none());
        let huge = serde_json::json!([{
            "tag_name": "android-v9.0.0", "assets": [{"name": "a-arm64.apk", "size": MAX_APK + 1,
            "browser_download_url": "https://github.com/o/r/a-arm64.apk"}]
        }]);
        assert!(newest(&huge, &v("0.1.0"), true, None).is_none());
    }

    #[test]
    fn the_check_at_start_waits_its_interval() {
        let s = Settings { last_check: 1_000_000, ..Settings::default() };
        assert!(!s.due(1_000_000 + 60));
        assert!(s.due(1_000_000 + CHECK_INTERVAL));
        assert!(!Settings { check_at_start: false, ..s.clone() }.due(i64::MAX));
    }
}
