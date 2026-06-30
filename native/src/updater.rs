// Native updater: checks GitHub Releases for a newer native build and (if a
// native asset is published) downloads it and swaps the running binary.
//
// Native builds ship under their OWN release tags `native-vX.Y.Z` (published as
// prereleases) so they never become the repo's "latest" and never disturb the
// Tauri build's `latest.json` auto-updater. We list all releases, keep the ones
// whose tag starts with `native-v`, and take the newest. The binary itself is an
// asset whose name contains "eyecare-native" (e.g. eyecare-native-linux-x86_64).

const REPO: &str = "frankmaruf/EyeCare";
const ASSET_MARKER: &str = "eyecare-native";
const TAG_PREFIX: &str = "native-v";

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub struct Latest {
    pub version: String,
    pub asset_url: Option<String>,
}

/// Compare dotted versions numerically: is `a` newer than `b`?
fn newer(a: &str, b: &str) -> bool {
    let parts = |s: &str| -> Vec<u64> {
        s.split('.').map(|x| x.trim().parse().unwrap_or(0)).collect()
    };
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (pa.get(i).copied().unwrap_or(0), pb.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// Find the newest `native-v*` release. Blocking — call off the UI thread.
pub fn check() -> Result<Latest, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=50");
    let resp = ureq::get(&url)
        .set("User-Agent", "EyeCare-native")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let releases: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;

    let mut best: Option<Latest> = None;
    for rel in releases.as_array().into_iter().flatten() {
        let tag = rel["tag_name"].as_str().unwrap_or("");
        let Some(version) = tag.strip_prefix(TAG_PREFIX) else {
            continue;
        };
        let asset_url = rel["assets"].as_array().and_then(|assets| {
            assets
                .iter()
                .find(|a| a["name"].as_str().map(|n| n.contains(ASSET_MARKER)).unwrap_or(false))
                .and_then(|a| a["browser_download_url"].as_str())
                .map(String::from)
        });
        let is_better = best.as_ref().map(|b| newer(version, &b.version)).unwrap_or(true);
        if is_better {
            best = Some(Latest {
                version: version.to_string(),
                asset_url,
            });
        }
    }
    Ok(best.unwrap_or(Latest {
        version: String::new(),
        asset_url: None,
    }))
}

/// Evaluate the latest release → (status text, downloadable asset, has-update).
pub fn evaluate() -> (String, Option<String>, bool) {
    match check() {
        Err(e) => (format!("Check failed: {e}"), None, false),
        Ok(l) => {
            if l.version.is_empty() {
                ("No release found".into(), None, false)
            } else if !newer(&l.version, current_version()) {
                (format!("Up to date (v{})", current_version()), None, false)
            } else if let Some(url) = l.asset_url {
                (format!("Update available: v{}", l.version), Some(url), true)
            } else {
                (
                    format!("v{} is out, but no native build is published yet", l.version),
                    None,
                    false,
                )
            }
        }
    }
}

/// Download the native asset and replace the running binary, then return the
/// path to restart. Returns an error string on failure.
pub fn install(asset_url: &str) -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let tmp = exe.with_extension("new");
    let resp = ureq::get(asset_url)
        .set("User-Agent", "EyeCare-native")
        .call()
        .map_err(|e| e.to_string())?;
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    std::io::copy(&mut reader, &mut file).map_err(|e| e.to_string())?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    // Replacing a running binary is allowed on Linux (the old inode lives until
    // exit); the next launch uses the new file.
    std::fs::rename(&tmp, &exe).map_err(|e| format!("replace failed (need write access?): {e}"))?;
    Ok(exe)
}
