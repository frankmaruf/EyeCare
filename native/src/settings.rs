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
    pub escalation: String, // "gentle" | "standard" | "forced"
    pub sound_enabled: bool,
    pub snooze_secs: u64,
    pub max_postpones: u32,
    pub long_break_enabled: bool,
    pub long_break_every: u32,
    pub long_break_secs: u64,
    pub tips_enabled: bool,
    pub exercises_enabled: bool,
    pub calm_visuals_enabled: bool,
    pub stats_enabled: bool,

    // wellbeing nudges (notifications fired while working)
    pub blink_enabled: bool,
    pub blink_interval_secs: u64,
    pub hydration_enabled: bool,
    pub hydration_interval_secs: u64,
    pub posture_enabled: bool,
    pub posture_interval_secs: u64,
    pub eyedrops_enabled: bool,
    pub eyedrops_interval_secs: u64,

    // system
    pub autostart: bool,

    // stay out of the way
    pub idle_pause_enabled: bool,
    pub idle_threshold_secs: u64,
    pub work_hours_enabled: bool,
    pub work_start: String, // "HH:MM"
    pub work_end: String,
    pub work_days: Vec<bool>, // Mon..Sun
    pub evening_nudge_enabled: bool,
    pub evening_hour: u32,

    // appearance
    pub accent: String, // hex "#rrggbb"
    pub reduce_motion: bool,
    pub high_contrast: bool,

    // window & overlay (§4.5 / §4.6)
    pub window_layer: String,      // "above" | "normal" | "below"
    pub allow_forced: bool,        // §4.3 — master switch for the forced overlay
    pub overlay_scope: String,     // "active" | "all" monitors
    pub require_full_break: bool,  // §4.6 — can't dismiss until break done
    pub show_skip_button: bool,    // §4.4
    pub show_postpone_button: bool,
    pub break_end_signal: String,  // "off" | "notification" | "chime" | "both" (§4.7)
    pub sound_volume: u32,         // 0–100 (§4.6)
    pub respect_dnd: bool,         // §5 / §9.9
    pub suppress_presentation: bool, // §9.9 — hush during fullscreen/screen-share
    pub shortcuts_enabled: bool,   // §4.8 — global hotkeys
    pub darkroom_enabled: bool,    // §9.5

    // floating widget
    pub widget_shape: String, // "round" | "squircle" | "square"
    pub widget_opacity: u32,   // 0–100
    pub widget_width: u32,
    pub widget_height: u32,
    pub widget_x: Option<i32>,
    pub widget_y: Option<i32>,
    pub widget_mode: String,       // "off" | "minimized" | "always" (§4.12)
    pub widget_layer: String,      // "above" | "normal" | "below"
    pub widget_bg: String,         // "solid" | "translucent" | "transparent"
    pub widget_click_through: bool, // §4.12 passive mode
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            work_interval_secs: 20 * 60,
            break_length_secs: 20,
            pre_break_warning_secs: 30,
            escalation: "standard".into(),
            sound_enabled: false,
            snooze_secs: 3 * 60,
            max_postpones: 2,
            long_break_enabled: true,
            long_break_every: 3,
            long_break_secs: 5 * 60,
            tips_enabled: true,
            exercises_enabled: false,
            calm_visuals_enabled: true,
            stats_enabled: true,
            blink_enabled: true,
            blink_interval_secs: 2 * 60,
            hydration_enabled: false,
            hydration_interval_secs: 45 * 60,
            posture_enabled: false,
            posture_interval_secs: 30 * 60,
            eyedrops_enabled: false,
            eyedrops_interval_secs: 2 * 60 * 60,
            autostart: false,
            idle_pause_enabled: true,
            idle_threshold_secs: 120,
            work_hours_enabled: false,
            work_start: "09:00".into(),
            work_end: "17:00".into(),
            work_days: vec![true; 7],
            evening_nudge_enabled: false,
            evening_hour: 20,
            accent: "#4cc6c0".into(),
            reduce_motion: false,
            high_contrast: false,
            window_layer: "normal".into(),
            allow_forced: true,
            overlay_scope: "active".into(),
            require_full_break: false,
            show_skip_button: true,
            show_postpone_button: true,
            break_end_signal: "both".into(),
            sound_volume: 70,
            respect_dnd: true,
            suppress_presentation: true,
            shortcuts_enabled: true,
            darkroom_enabled: false,
            widget_shape: "squircle".into(),
            widget_opacity: 95,
            widget_width: 132,
            widget_height: 132,
            widget_x: None,
            widget_y: None,
            widget_mode: "always".into(),
            widget_layer: "above".into(),
            widget_bg: "translucent".into(),
            widget_click_through: false,
        }
    }
}

/// Clamp the widget to a sane size range (mirrors the Tauri 120–480 clamp).
pub const WIDGET_MIN: u32 = 80;
pub const WIDGET_MAX: u32 = 480;

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
