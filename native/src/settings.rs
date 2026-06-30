// Persisted settings. Field names mirror the Tauri build's Settings (camelCase
// JSON) so the two can converge. `#[serde(default)]` at the struct level means
// missing keys fall back to `Default` — one place to define defaults, no
// per-field default functions (DRY vs the original lib.rs).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub work_interval_secs: u64,
    pub break_length_secs: u64,
    pub pre_break_warning_secs: u64,
    pub snooze_secs: u64,
    pub max_postpones: u32,
    pub long_break_enabled: bool,
    pub long_break_every: u32,
    pub long_break_secs: u64,
    pub tips_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            work_interval_secs: 20 * 60,
            break_length_secs: 20,
            pre_break_warning_secs: 30,
            snooze_secs: 3 * 60,
            max_postpones: 2,
            long_break_enabled: true,
            long_break_every: 3,
            long_break_secs: 5 * 60,
            tips_enabled: true,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        let mut p = dirs::config_dir()?;
        p.push("us.frankmaruf.eyecare-native");
        std::fs::create_dir_all(&p).ok()?;
        p.push("settings.json");
        Some(p)
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let (Some(p), Ok(json)) = (Self::path(), serde_json::to_string_pretty(self)) {
            let _ = std::fs::write(p, json);
        }
    }
}
