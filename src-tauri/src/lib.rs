// EyeCare — Rust backend.
//
// Owns the authoritative timer (work interval -> break -> repeat), the user
// settings (persisted to a local JSON file), the system-tray icon, and the
// break reminder window. The frontend is a thin view that listens to events
// and calls commands; all timing lives here so it stays precise and survives
// webview reloads.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Set when the user explicitly quits (tray → Quit), so the exit guard knows to
/// let the process exit instead of keeping it alive in the tray.
static QUITTING: AtomicBool = AtomicBool::new(false);

use chrono::{Datelike, Timelike};
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_notification::NotificationExt;

#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;
#[cfg(desktop)]
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

// ---------------------------------------------------------------------------
// Settings (persisted as JSON in the app config dir)
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    /// Length of a work interval, in seconds (default 20 min).
    work_interval_secs: u64,
    /// Length of a break, in seconds (default 20 sec).
    break_length_secs: u64,
    /// Heads-up warning before a break, in seconds (0 = off).
    pre_break_warning_secs: u64,
    /// "gentle" | "standard" | "forced".
    escalation: String,
    /// How long a postpone delays the break, in seconds.
    snooze_secs: u64,
    /// Max postpones per break (0 = unlimited).
    max_postpones: u32,
    /// Play a sound when the break starts (handled in the webview).
    sound_enabled: bool,

    // --- Floating widget (§4.12) ---
    /// "off" | "minimized" | "always".
    #[serde(default = "d_widget_mode")]
    widget_mode: String,
    /// "round" | "squircle" | "square".
    #[serde(default = "d_widget_shape")]
    widget_shape: String,
    /// Widget width, logical px.
    #[serde(default = "d_widget_dim")]
    widget_width: u32,
    /// Widget height, logical px.
    #[serde(default = "d_widget_dim")]
    widget_height: u32,
    /// Fill opacity, 20–100.
    #[serde(default = "d_widget_opacity")]
    widget_opacity: u32,
    /// Last on-screen position (physical px); None = let the OS place it.
    #[serde(default)]
    widget_x: Option<f64>,
    #[serde(default)]
    widget_y: Option<f64>,

    // --- Startup / shortcuts / work hours (§4.8, §4.9, §9.9) ---
    /// Launch at login.
    #[serde(default)]
    autostart: bool,
    /// Master switch for system-wide hotkeys.
    #[serde(default = "d_true")]
    global_shortcuts_enabled: bool,
    #[serde(default = "d_sc_pause")]
    sc_pause: String,
    #[serde(default = "d_sc_skip")]
    sc_skip: String,
    #[serde(default = "d_sc_take")]
    sc_take: String,
    #[serde(default = "d_sc_postpone")]
    sc_postpone: String,
    #[serde(default = "d_sc_toggle_widget")]
    sc_toggle_widget: String,
    /// Only run reminders during the work-hours window.
    #[serde(default)]
    work_hours_enabled: bool,
    /// "HH:MM" local time.
    #[serde(default = "d_work_start")]
    work_start: String,
    #[serde(default = "d_work_end")]
    work_end: String,
    /// Which weekdays reminders run on (Mon..Sun).
    #[serde(default = "d_work_days")]
    work_days: [bool; 7],

    // --- v1.1 ---
    /// Pause the timer when there's been no input for a while.
    #[serde(default = "d_true")]
    idle_pause_enabled: bool,
    /// Idle seconds before pausing.
    #[serde(default = "d_idle_threshold")]
    idle_threshold_secs: u64,
    /// Make every Nth break a longer "stand up & move" break.
    #[serde(default)]
    long_break_enabled: bool,
    /// A long break every N breaks.
    #[serde(default = "d_long_break_every")]
    long_break_every: u32,
    /// Long-break length, seconds.
    #[serde(default = "d_long_break_secs")]
    long_break_secs: u64,
    /// Periodic "blink fully" nudge while working.
    #[serde(default)]
    blink_enabled: bool,
    /// Seconds between blink nudges.
    #[serde(default = "d_blink_interval")]
    blink_interval_secs: u64,
    /// Disable animations.
    #[serde(default)]
    reduce_motion: bool,
    /// High-contrast palette.
    #[serde(default)]
    high_contrast: bool,
    /// Suppress the forced overlay when another app is fullscreen
    /// (presentation / screen-share / video).
    #[serde(default = "d_true")]
    suppress_on_fullscreen: bool,
    /// Respect the OS Do-Not-Disturb state — hide the widget and suppress
    /// breaks while DND is on (KDE turns DND on during screen-share).
    #[serde(default = "d_true")]
    respect_dnd: bool,

    // --- v1.2 eye-health extras ---
    /// Periodic "drink water" nudge while working.
    #[serde(default)]
    hydration_enabled: bool,
    #[serde(default = "d_hydration_interval")]
    hydration_interval_secs: u64,
    /// Periodic posture / screen-distance nudge while working.
    #[serde(default)]
    posture_enabled: bool,
    #[serde(default = "d_posture_interval")]
    posture_interval_secs: u64,
    /// Once-per-evening warm-screen / dark-room nudge.
    #[serde(default)]
    evening_nudge_enabled: bool,
    /// Local hour (0–23) after which the evening nudge fires once.
    #[serde(default = "d_evening_hour")]
    evening_hour: u32,
    /// Show a rotating eye-care tip on the break screen.
    #[serde(default = "d_true")]
    tips_enabled: bool,
    /// Show a guided eye-exercise animation during long breaks.
    #[serde(default)]
    exercises_enabled: bool,
    /// Calming animated visual on the break screen.
    #[serde(default = "d_true")]
    calm_visuals_enabled: bool,
    /// Accent colour (hex).
    #[serde(default = "d_accent")]
    accent: String,
    /// Track local break stats / streaks.
    #[serde(default = "d_true")]
    stats_enabled: bool,
    /// Periodic artificial-tears / eye-drops reminder while working.
    #[serde(default)]
    eyedrops_enabled: bool,
    #[serde(default = "d_eyedrops_interval")]
    eyedrops_interval_secs: u64,
}

fn d_eyedrops_interval() -> u64 {
    2 * 3600
}

fn d_accent() -> String {
    "#4cc6c0".into()
}

fn d_hydration_interval() -> u64 {
    45 * 60
}
fn d_posture_interval() -> u64 {
    30 * 60
}
fn d_evening_hour() -> u32 {
    20
}

fn d_idle_threshold() -> u64 {
    120
}
fn d_long_break_every() -> u32 {
    3
}
fn d_long_break_secs() -> u64 {
    5 * 60
}
fn d_blink_interval() -> u64 {
    5 * 60
}

fn d_true() -> bool {
    true
}
fn d_sc_pause() -> String {
    "CmdOrControl+Alt+P".into()
}
fn d_sc_skip() -> String {
    "CmdOrControl+Alt+S".into()
}
fn d_sc_take() -> String {
    "CmdOrControl+Alt+B".into()
}
fn d_sc_postpone() -> String {
    "CmdOrControl+Alt+Z".into()
}
fn d_sc_toggle_widget() -> String {
    "CmdOrControl+Alt+W".into()
}
fn d_work_start() -> String {
    "09:00".into()
}
fn d_work_end() -> String {
    "17:00".into()
}
fn d_work_days() -> [bool; 7] {
    [true; 7]
}

fn d_widget_mode() -> String {
    "minimized".into()
}
fn d_widget_shape() -> String {
    "squircle".into()
}
fn d_widget_dim() -> u32 {
    150
}
fn d_widget_opacity() -> u32 {
    95
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            work_interval_secs: 20 * 60,
            break_length_secs: 20,
            pre_break_warning_secs: 30,
            escalation: "standard".into(),
            snooze_secs: 3 * 60,
            max_postpones: 2,
            sound_enabled: false,
            widget_mode: d_widget_mode(),
            widget_shape: d_widget_shape(),
            widget_width: d_widget_dim(),
            widget_height: d_widget_dim(),
            widget_opacity: d_widget_opacity(),
            widget_x: None,
            widget_y: None,
            autostart: false,
            global_shortcuts_enabled: true,
            sc_pause: d_sc_pause(),
            sc_skip: d_sc_skip(),
            sc_take: d_sc_take(),
            sc_postpone: d_sc_postpone(),
            sc_toggle_widget: d_sc_toggle_widget(),
            work_hours_enabled: false,
            work_start: d_work_start(),
            work_end: d_work_end(),
            work_days: d_work_days(),
            idle_pause_enabled: true,
            idle_threshold_secs: d_idle_threshold(),
            long_break_enabled: false,
            long_break_every: d_long_break_every(),
            long_break_secs: d_long_break_secs(),
            blink_enabled: false,
            blink_interval_secs: d_blink_interval(),
            reduce_motion: false,
            high_contrast: false,
            suppress_on_fullscreen: true,
            respect_dnd: true,
            hydration_enabled: false,
            hydration_interval_secs: d_hydration_interval(),
            posture_enabled: false,
            posture_interval_secs: d_posture_interval(),
            evening_nudge_enabled: false,
            evening_hour: d_evening_hour(),
            tips_enabled: true,
            exercises_enabled: false,
            calm_visuals_enabled: true,
            accent: d_accent(),
            stats_enabled: true,
            eyedrops_enabled: false,
            eyedrops_interval_secs: d_eyedrops_interval(),
        }
    }
}

/// Keep every value inside the ranges from the requirements (§6) so a bad
/// value coming from the frontend can never wedge the timer.
fn clamp_settings(mut s: Settings) -> Settings {
    s.work_interval_secs = s.work_interval_secs.clamp(60, 120 * 60);
    s.break_length_secs = s.break_length_secs.clamp(5, 600);
    s.pre_break_warning_secs = s.pre_break_warning_secs.min(120);
    s.snooze_secs = s.snooze_secs.clamp(60, 60 * 60);
    if s.max_postpones > 10 {
        s.max_postpones = 10;
    }
    if !matches!(s.escalation.as_str(), "gentle" | "standard" | "forced") {
        s.escalation = "standard".into();
    }
    s.widget_width = s.widget_width.clamp(120, 480);
    s.widget_height = s.widget_height.clamp(120, 480);
    s.widget_opacity = s.widget_opacity.clamp(20, 100);
    if !matches!(s.widget_mode.as_str(), "off" | "minimized" | "always") {
        s.widget_mode = "minimized".into();
    }
    if !matches!(s.widget_shape.as_str(), "round" | "squircle" | "square") {
        s.widget_shape = "squircle".into();
    }
    s.idle_threshold_secs = s.idle_threshold_secs.clamp(30, 600);
    s.long_break_every = s.long_break_every.clamp(1, 20);
    s.long_break_secs = s.long_break_secs.clamp(60, 3600);
    s.blink_interval_secs = s.blink_interval_secs.clamp(30, 3600);
    s.hydration_interval_secs = s.hydration_interval_secs.clamp(5 * 60, 4 * 60 * 60);
    s.posture_interval_secs = s.posture_interval_secs.clamp(5 * 60, 4 * 60 * 60);
    s.eyedrops_interval_secs = s.eyedrops_interval_secs.clamp(5 * 60, 8 * 60 * 60);
    s.evening_hour = s.evening_hour.min(23);
    s
}

// ---------------------------------------------------------------------------
// Runtime timer state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Working,
    Break,
}

struct TimerState {
    phase: Phase,
    remaining: u64,
    paused: bool,
    postpones_used: u32,
    /// Whether the pre-break warning already fired this work cycle.
    warned: bool,
    /// Count of breaks started (drives the long-break cadence).
    breaks_done: u32,
    /// The current break is a long break.
    is_long: bool,
    /// Countdown to the next blink nudge while working.
    blink_remaining: u64,
    /// Currently frozen because the user is idle.
    idle: bool,
    /// Countdowns to the next hydration / posture / eye-drops nudge.
    hydration_remaining: u64,
    posture_remaining: u64,
    eyedrops_remaining: u64,
    /// Day-of-year the evening nudge last fired (-1 = not yet).
    evening_nudged_day: i32,
}

#[derive(Clone, Copy)]
enum ShortAction {
    Pause,
    Skip,
    Take,
    Postpone,
    ToggleWidget,
}

struct AppState {
    settings: Mutex<Settings>,
    timer: Mutex<TimerState>,
    /// Registered global shortcuts as (accelerator string, action).
    shortcuts: Mutex<Vec<(String, ShortAction)>>,
    /// Manual one-click "hide the widget" (e.g. while screen-recording).
    widget_hidden_override: Mutex<bool>,
    /// Whether the user wants the main window shown. We track this ourselves
    /// instead of polling is_visible(), which is unreliable on some compositors.
    main_shown: Mutex<bool>,
}

/// Snapshot sent to the frontend on every tick.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimerSnapshot {
    phase: Phase,
    remaining: u64,
    total: u64,
    paused: bool,
    postpones_used: u32,
    max_postpones: u32,
    is_long: bool,
    idle: bool,
}

fn snapshot(t: &TimerState, s: &Settings) -> TimerSnapshot {
    let total = match t.phase {
        Phase::Working => s.work_interval_secs,
        Phase::Break if t.is_long => s.long_break_secs,
        Phase::Break => s.break_length_secs,
    };
    TimerSnapshot {
        phase: t.phase,
        remaining: t.remaining,
        total,
        paused: t.paused,
        postpones_used: t.postpones_used,
        max_postpones: s.max_postpones,
        is_long: t.is_long,
        idle: t.idle,
    }
}

fn to_working(t: &mut TimerState, s: &Settings) {
    t.phase = Phase::Working;
    t.remaining = s.work_interval_secs;
    t.warned = false;
    t.postpones_used = 0;
    t.is_long = false;
    t.blink_remaining = s.blink_interval_secs.max(1);
}

fn to_break(t: &mut TimerState, s: &Settings) {
    t.breaks_done = t.breaks_done.wrapping_add(1);
    let long = s.long_break_enabled
        && s.long_break_every > 0
        && t.breaks_done % s.long_break_every == 0;
    t.is_long = long;
    t.phase = Phase::Break;
    t.remaining = if long {
        s.long_break_secs
    } else {
        s.break_length_secs
    };
    t.warned = false;
}

// ---------------------------------------------------------------------------
// Settings persistence
// ---------------------------------------------------------------------------

fn settings_path(app: &AppHandle) -> std::path::PathBuf {
    let dir = app
        .path()
        .app_config_dir()
        .expect("could not resolve app config dir");
    std::fs::create_dir_all(&dir).ok();
    dir.join("settings.json")
}

fn load_settings(app: &AppHandle) -> Settings {
    let path = settings_path(app);
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(parsed) = serde_json::from_str::<Settings>(&text) {
            return clamp_settings(parsed);
        }
    }
    Settings::default()
}

fn save_settings(app: &AppHandle, s: &Settings) {
    let path = settings_path(app);
    if let Ok(text) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(path, text);
    }
}

// ---------------------------------------------------------------------------
// Habit stats (local-only, no telemetry)
// ---------------------------------------------------------------------------

#[derive(Default, Serialize, Deserialize)]
struct Stats {
    days: std::collections::BTreeMap<String, DayStat>,
}

#[derive(Default, Serialize, Deserialize, Clone, Copy)]
struct DayStat {
    taken: u32,
    skipped: u32,
}

fn stats_path(app: &AppHandle) -> std::path::PathBuf {
    let dir = app.path().app_config_dir().expect("config dir");
    std::fs::create_dir_all(&dir).ok();
    dir.join("stats.json")
}

fn load_stats(app: &AppHandle) -> Stats {
    std::fs::read_to_string(stats_path(app))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn today_key() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn record_break(app: &AppHandle, taken: bool) {
    let enabled = app.state::<AppState>().settings.lock().unwrap().stats_enabled;
    if !enabled {
        return;
    }
    let mut st = load_stats(app);
    let day = st.days.entry(today_key()).or_default();
    if taken {
        day.taken += 1;
    } else {
        day.skipped += 1;
    }
    if let Ok(text) = serde_json::to_string_pretty(&st) {
        let _ = std::fs::write(stats_path(app), text);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DayBar {
    date: String,
    taken: u32,
    skipped: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatsSummary {
    today_taken: u32,
    today_skipped: u32,
    total_taken: u32,
    streak: u32,
    last7: Vec<DayBar>,
}

#[tauri::command]
fn get_stats(app: AppHandle) -> StatsSummary {
    let st = load_stats(&app);
    let total_taken: u32 = st.days.values().map(|d| d.taken).sum();
    let today = today_key();
    let today_stat = st.days.get(&today).copied().unwrap_or_default();
    let base = chrono::Local::now().date_naive();

    let mut last7 = Vec::new();
    for i in (0..7).rev() {
        let date = (base - chrono::Duration::days(i))
            .format("%Y-%m-%d")
            .to_string();
        let d = st.days.get(&date).copied().unwrap_or_default();
        last7.push(DayBar {
            date,
            taken: d.taken,
            skipped: d.skipped,
        });
    }

    // streak: consecutive days with a break taken, up to today (today not yet
    // counted if it has no break, so a fresh morning doesn't reset it).
    let mut streak = 0u32;
    let mut i: i64 = if today_stat.taken == 0 { 1 } else { 0 };
    loop {
        let date = (base - chrono::Duration::days(i))
            .format("%Y-%m-%d")
            .to_string();
        match st.days.get(&date) {
            Some(d) if d.taken > 0 => {
                streak += 1;
                i += 1;
            }
            _ => break,
        }
    }

    StatsSummary {
        today_taken: today_stat.taken,
        today_skipped: today_stat.skipped,
        total_taken,
        streak,
        last7,
    }
}

// ---------------------------------------------------------------------------
// Break window + tray helpers
// ---------------------------------------------------------------------------

/// Open the break reminder window (or fire a notification for "gentle").
fn start_break(app: &AppHandle) {
    let (mut escalation, sound, suppress, respect_dnd) = {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap();
        (
            s.escalation.clone(),
            s.sound_enabled,
            s.suppress_on_fullscreen,
            s.respect_dnd,
        )
    };

    // Don't pop a window over a presentation / screen-share / fullscreen video,
    // or while the OS is in Do-Not-Disturb: downgrade to a gentle notification.
    if escalation != "gentle"
        && ((suppress && another_app_fullscreen()) || (respect_dnd && os_dnd_active()))
    {
        escalation = "gentle".into();
    }

    // Gentle level: just a notification, no window grab.
    if escalation == "gentle" {
        let _ = app
            .notification()
            .builder()
            .title("Time for an eye break")
            .body("Look ~20 feet (6 m) away and relax your eyes.")
            .show();
        return;
    }

    if app.get_webview_window("break").is_some() {
        return; // already open
    }

    let forced = escalation == "forced";
    let url = if sound {
        "index.html#break?sound=1"
    } else {
        "index.html#break"
    };

    let mut builder = WebviewWindowBuilder::new(app, "break", WebviewUrl::App(url.into()))
        .title("EyeCare — break time")
        .inner_size(580.0, 620.0)
        .min_inner_size(420.0, 460.0)
        .resizable(true)
        .always_on_top(forced)
        .decorations(!forced)
        .skip_taskbar(forced)
        .focused(forced)
        .center();

    if forced {
        builder = builder.fullscreen(true);
    }

    if let Ok(w) = builder.build() {
        let _ = w.set_icon(app_image());
    }
    update_widget_visibility(app);
}

fn close_break_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("break") {
        let _ = w.close();
    }
}

/// Apply persisted size/position to the widget window and push the current
/// settings to the widget UI (shape/opacity are CSS-side).
fn apply_widget_config(app: &AppHandle) {
    let s = {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap().clone();
        s
    };
    if let Some(w) = app.get_webview_window("widget") {
        let _ = w.set_size(tauri::LogicalSize::new(
            s.widget_width as f64,
            s.widget_height as f64,
        ));
        if let (Some(x), Some(y)) = (s.widget_x, s.widget_y) {
            let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
        }
    }
    let _ = app.emit("settings:changed", &s);
}

/// Show the widget when it should be visible (mode = always, or mode =
/// minimized and the main window is hidden), hide it otherwise. Suppressed
/// while a break window is open. Idempotent — safe to call every tick.
fn update_widget_visibility(app: &AppHandle) {
    let widget = match app.get_webview_window("widget") {
        Some(w) => w,
        None => return,
    };
    let (mode, suppress, respect_dnd) = {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap();
        (
            s.widget_mode.clone(),
            s.suppress_on_fullscreen,
            s.respect_dnd,
        )
    };
    // Note: we no longer poll is_minimized() to auto-hide the main window. That
    // polling had transient false-positives on some compositors (KDE), which
    // would spuriously hide — or, once the webview is freed on close, destroy —
    // the window. Instead the widget appears when the main window is *closed*
    // (the reliable Close/X event, which also frees its ~200 MB webview); the
    // native minimize button now just minimizes normally.
    let main_shown = *app.state::<AppState>().main_shown.lock().unwrap();
    let break_open = app.get_webview_window("break").is_some();
    let mut want = mode != "off" && !break_open && (mode == "always" || !main_shown);
    // Hide the widget over a fullscreen app (presentation / fullscreen call) so
    // it doesn't show up on a shared/recorded screen.
    if want && suppress && another_app_fullscreen() {
        want = false;
    }
    // Hide while the OS is in DND (KDE enables DND during screen-share).
    if want && respect_dnd && os_dnd_active() {
        want = false;
    }
    if *app.state::<AppState>().widget_hidden_override.lock().unwrap() {
        want = false;
    }
    let is_visible = widget.is_visible().unwrap_or(false);

    if want {
        if !is_visible {
            let _ = widget.show();
        }
        // The WM may minimize the widget together with the app; bring it back,
        // and re-assert keep-above (some compositors drop it on focus change).
        let _ = widget.unminimize();
        let _ = widget.set_always_on_top(true);
    } else if is_visible {
        let _ = widget.hide();
    }
}

fn update_tray(app: &AppHandle, snap: &TimerSnapshot) {
    if let Some(tray) = app.tray_by_id("main-tray") {
        let mm = snap.remaining / 60;
        let ss = snap.remaining % 60;
        let label = if snap.paused {
            format!("EyeCare — paused ({:02}:{:02})", mm, ss)
        } else {
            match snap.phase {
                Phase::Working => format!("EyeCare — next break in {:02}:{:02}", mm, ss),
                Phase::Break => format!("EyeCare — break: {:02}:{:02} left", mm, ss),
            }
        };
        let _ = tray.set_tooltip(Some(&label));
    }
}

/// Build a fresh snapshot, push it to the frontend, and refresh the tray.
fn emit_tick(app: &AppHandle) {
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap().clone();
    let snap = {
        let t = state.timer.lock().unwrap();
        snapshot(&t, &s)
    };
    let _ = app.emit("timer:tick", snap.clone());
    update_tray(app, &snap);
    update_widget_visibility(app);
}

/// Closing/minimizing the main window destroys its webview to free memory (a
/// WebKit/WebView2 renderer is ~200 MB). The app stays alive in the tray, and
/// the window is rebuilt on demand by `show_main_window`. This handler turns a
/// close/minimize into "free the webview + show the widget".
fn attach_main_close_handler(w: &tauri::WebviewWindow) {
    let win = w.clone();
    w.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { .. } = event {
            // Don't prevent the close — letting it through frees the renderer.
            let app = win.app_handle();
            *app.state::<AppState>().main_shown.lock().unwrap() = false;
            update_widget_visibility(app);
        }
    });
}

/// Build the main window (same config as tauri.conf) when it isn't currently
/// alive. Returns the existing one if present.
fn create_main(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window("main") {
        return Some(w);
    }
    let w = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("EyeCare")
        .inner_size(760.0, 680.0)
        .min_inner_size(520.0, 520.0)
        .resizable(true)
        .build()
        .ok()?;
    let _ = w.set_icon(app_image());
    attach_main_close_handler(&w);
    Some(w)
}

fn show_main_window(app: &AppHandle) {
    let w = match create_main(app) {
        Some(w) => w,
        None => return,
    };
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
    *app.state::<AppState>().main_shown.lock().unwrap() = true;
    update_widget_visibility(app);
}

// ---------------------------------------------------------------------------
// Actions shared by commands and the tray menu
// ---------------------------------------------------------------------------

fn do_toggle_pause(app: &AppHandle) {
    {
        let state = app.state::<AppState>();
        let mut t = state.timer.lock().unwrap();
        t.paused = !t.paused;
    }
    emit_tick(app);
}

fn do_toggle_widget(app: &AppHandle) {
    {
        let state = app.state::<AppState>();
        let mut o = state.widget_hidden_override.lock().unwrap();
        *o = !*o;
    }
    update_widget_visibility(app);
}

fn do_take_break(app: &AppHandle) {
    {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap().clone();
        let mut t = state.timer.lock().unwrap();
        to_break(&mut t, &s);
    }
    start_break(app);
    emit_tick(app);
}

fn do_skip(app: &AppHandle) {
    let was_break;
    {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap().clone();
        let mut t = state.timer.lock().unwrap();
        was_break = t.phase == Phase::Break;
        to_working(&mut t, &s);
    }
    if was_break {
        record_break(app, false);
    }
    close_break_window(app);
    emit_tick(app);
}

/// Delay the break by the snooze duration. Returns false if the postpone cap
/// has been reached.
fn do_postpone(app: &AppHandle) -> bool {
    let allowed = {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap().clone();
        let mut t = state.timer.lock().unwrap();
        let unlimited = s.max_postpones == 0;
        if unlimited || t.postpones_used < s.max_postpones {
            t.postpones_used += 1;
            t.phase = Phase::Working;
            t.remaining = s.snooze_secs;
            t.warned = false;
            true
        } else {
            false
        }
    };
    if allowed {
        close_break_window(app);
    }
    emit_tick(app);
    allowed
}

/// Whether reminders should run right now (always true unless work-hours is on
/// and the current local time is outside the configured window).
fn within_work_hours(s: &Settings) -> bool {
    if !s.work_hours_enabled {
        return true;
    }
    let now = chrono::Local::now();
    // Mon = 0 .. Sun = 6
    let weekday = now.weekday().num_days_from_monday() as usize;
    if !s.work_days.get(weekday).copied().unwrap_or(true) {
        return false;
    }
    let now_min = now.hour() * 60 + now.minute();
    let parse = |t: &str| -> Option<u32> {
        let mut parts = t.split(':');
        let h: u32 = parts.next()?.trim().parse().ok()?;
        let m: u32 = parts.next()?.trim().parse().ok()?;
        Some(h * 60 + m)
    };
    let start = parse(&s.work_start).unwrap_or(0);
    let end = parse(&s.work_end).unwrap_or(24 * 60);
    if start <= end {
        now_min >= start && now_min < end
    } else {
        // window spans midnight
        now_min >= start || now_min < end
    }
}

/// Seconds since the last keyboard/mouse input (0 if it can't be read).
/// Idle detection needs the X11 screensaver extension; on setups without it
/// (e.g. some XWayland displays) we disable it after the first failure so it
/// degrades gracefully instead of spamming and re-trying every second.
fn idle_seconds() -> u64 {
    use std::sync::atomic::{AtomicBool, Ordering};
    static UNAVAILABLE: AtomicBool = AtomicBool::new(false);
    if UNAVAILABLE.load(Ordering::Relaxed) {
        return 0;
    }
    match user_idle::UserIdle::get_time() {
        Ok(t) => t.as_seconds(),
        Err(_) => {
            UNAVAILABLE.store(true, Ordering::Relaxed);
            0
        }
    }
}

/// Whether some other app currently has a fullscreen window (a proxy for
/// "presenting / screen-sharing / watching video"). Linux/X11 only.
#[cfg(target_os = "linux")]
fn another_app_fullscreen() -> bool {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    let probe = || -> Result<bool, Box<dyn std::error::Error>> {
        let (conn, screen_num) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen_num].root;
        let atom = |name: &[u8]| -> Result<u32, Box<dyn std::error::Error>> {
            Ok(conn.intern_atom(false, name)?.reply()?.atom)
        };
        let active_atom = atom(b"_NET_ACTIVE_WINDOW")?;
        let state_atom = atom(b"_NET_WM_STATE")?;
        let fs_atom = atom(b"_NET_WM_STATE_FULLSCREEN")?;

        let active = conn
            .get_property(false, root, active_atom, AtomEnum::WINDOW, 0, 1)?
            .reply()?;
        let win = match active.value32().and_then(|mut it| it.next()) {
            Some(w) if w != 0 => w,
            _ => return Ok(false),
        };

        let st = conn
            .get_property(false, win, state_atom, AtomEnum::ATOM, 0, 64)?
            .reply()?;
        let is_fs = st
            .value32()
            .map(|mut it| it.any(|a| a == fs_atom))
            .unwrap_or(false);
        if !is_fs {
            return Ok(false);
        }

        // Exclude our own break overlay (WM_CLASS contains "eyebreak").
        let cls = conn
            .get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)?
            .reply()?;
        let cls_str = String::from_utf8_lossy(&cls.value).to_lowercase();
        Ok(!(cls_str.contains("eyebreak")
            || cls_str.contains("eyecare")
            || cls_str.contains("frankmaruf")))
    };
    probe().unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn another_app_fullscreen() -> bool {
    false
}

/// Whether the OS is in Do-Not-Disturb. On Linux this reads the freedesktop
/// Notifications "Inhibited" property over D-Bus — KDE Plasma turns this on
/// during screen-sharing / recording / presentations.
#[cfg(target_os = "linux")]
fn os_dnd_active() -> bool {
    use zbus::blocking::Connection;
    let probe = || -> zbus::Result<bool> {
        let conn = Connection::session()?;
        let reply = conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.Notifications", "Inhibited"),
        )?;
        let value: zbus::zvariant::OwnedValue = reply.body().deserialize()?;
        Ok(bool::try_from(value).unwrap_or(false))
    };
    probe().unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn os_dnd_active() -> bool {
    false
}

/// Enable/disable launch-at-login to match the setting.
#[cfg(desktop)]
fn apply_autostart(app: &AppHandle) {
    let want = app.state::<AppState>().settings.lock().unwrap().autostart;
    let mgr = app.autolaunch();
    let is = mgr.is_enabled().unwrap_or(false);
    if want && !is {
        let _ = mgr.enable();
    } else if !want && is {
        let _ = mgr.disable();
    }
}

/// (Re)register the global hotkeys from the current settings.
#[cfg(desktop)]
fn register_shortcuts(app: &AppHandle) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();

    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap().clone();
    let mut map = state.shortcuts.lock().unwrap();
    map.clear();
    if !s.global_shortcuts_enabled {
        return;
    }
    for (accel, action) in [
        (s.sc_pause.clone(), ShortAction::Pause),
        (s.sc_skip.clone(), ShortAction::Skip),
        (s.sc_take.clone(), ShortAction::Take),
        (s.sc_postpone.clone(), ShortAction::Postpone),
        (s.sc_toggle_widget.clone(), ShortAction::ToggleWidget),
    ] {
        let accel = accel.trim().to_string();
        if accel.is_empty() {
            continue;
        }
        if let Ok(sc) = accel.parse::<Shortcut>() {
            if gs.register(sc).is_ok() {
                map.push((accel, action));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Commands (callable from the webview)
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn set_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Settings {
    let cleaned = clamp_settings(settings);
    {
        let mut s = state.settings.lock().unwrap();
        *s = cleaned.clone();
        save_settings(&app, &s);
        // If we shortened the work interval below the time already counted,
        // cap the remaining time so the next break isn't pushed out.
        let mut t = state.timer.lock().unwrap();
        if t.phase == Phase::Working && t.remaining > s.work_interval_secs {
            t.remaining = s.work_interval_secs;
        }
        // Likewise cap each wellbeing-nudge countdown to its (possibly new)
        // interval, so lowering an interval — or enabling a nudge — takes
        // effect within that interval instead of waiting out the old one.
        t.blink_remaining = t.blink_remaining.clamp(1, s.blink_interval_secs.max(1));
        t.hydration_remaining = t
            .hydration_remaining
            .clamp(1, s.hydration_interval_secs.max(1));
        t.posture_remaining = t
            .posture_remaining
            .clamp(1, s.posture_interval_secs.max(1));
        t.eyedrops_remaining = t
            .eyedrops_remaining
            .clamp(1, s.eyedrops_interval_secs.max(1));
    }
    apply_widget_config(&app);
    #[cfg(desktop)]
    {
        apply_autostart(&app);
        register_shortcuts(&app);
    }
    emit_tick(&app);
    cleaned
}

#[tauri::command]
fn get_timer(state: State<AppState>) -> TimerSnapshot {
    let s = state.settings.lock().unwrap().clone();
    let t = state.timer.lock().unwrap();
    snapshot(&t, &s)
}

#[tauri::command]
fn timer_set_paused(app: AppHandle, state: State<AppState>, paused: bool) {
    {
        state.timer.lock().unwrap().paused = paused;
    }
    emit_tick(&app);
}

#[tauri::command]
fn timer_skip(app: AppHandle) {
    do_skip(&app);
}

#[tauri::command]
fn timer_take_break(app: AppHandle) {
    do_take_break(&app);
}

/// Delay the break by the snooze duration. Returns false if the postpone cap
/// has been reached (the break is then enforced).
#[tauri::command]
fn timer_postpone(app: AppHandle) -> bool {
    do_postpone(&app)
}

#[tauri::command]
fn show_main(app: AppHandle) {
    show_main_window(&app);
}

/// Check the update endpoint. Returns the new version ("x.y.z") if one is
/// available, an empty string if up to date, or an error string.
#[tauri::command]
async fn check_update(app: AppHandle) -> Result<String, String> {
    #[cfg(desktop)]
    {
        use tauri_plugin_updater::UpdaterExt;
        let updater = app.updater().map_err(|e| e.to_string())?;
        return match updater.check().await {
            Ok(Some(update)) => Ok(update.version),
            Ok(None) => Ok(String::new()),
            Err(e) => Err(e.to_string()),
        };
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err("updates not supported on this platform".into())
    }
}

/// Download + install the available update, then restart.
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    #[cfg(desktop)]
    {
        use tauri_plugin_updater::UpdaterExt;
        let updater = app.updater().map_err(|e| e.to_string())?;
        if let Some(update) = updater.check().await.map_err(|e| e.to_string())? {
            update
                .download_and_install(|_, _| {}, || {})
                .await
                .map_err(|e| e.to_string())?;
            app.restart();
        }
        Ok(())
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err("updates not supported on this platform".into())
    }
}

// ---------------------------------------------------------------------------
// The 1-second timer loop
// ---------------------------------------------------------------------------

enum Tick {
    None,
    Prewarn(u64),
    BreakStart,
    BreakEnd,
}

#[derive(Default)]
struct Nudges {
    blink: bool,
    hydration: bool,
    posture: bool,
    eyedrops: bool,
    evening: bool,
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// The real app icon, embedded from the icon file (Tauri's
/// default_window_icon falls back to its own logo, so we set ours explicitly).
fn app_image() -> tauri::image::Image<'static> {
    tauri::image::Image::from_bytes(include_bytes!("../icons/128x128.png"))
        .expect("embedded app icon")
}

fn run_timer(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        // After a suspend the future is paused; don't fire a backlog of ticks.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_wall = chrono::Local::now();

        loop {
            ticker.tick().await;

            // Suspend/resume or clock-jump detection: if real time skipped
            // ahead, freeze this tick instead of decrementing / firing.
            let now_wall = chrono::Local::now();
            let slept = (now_wall - last_wall).num_seconds() > 3;
            last_wall = now_wall;

            let (action, nudges) = {
                let state = app.state::<AppState>();
                let s = state.settings.lock().unwrap().clone();
                let mut t = state.timer.lock().unwrap();

                let idle_now =
                    s.idle_pause_enabled && idle_seconds() >= s.idle_threshold_secs;
                t.idle = idle_now;

                let mut nudges = Nudges::default();

                // Evening warm-screen / dark-room nudge — once per day, any state.
                if s.evening_nudge_enabled {
                    let today = now_wall.ordinal() as i32;
                    if now_wall.hour() >= s.evening_hour && t.evening_nudged_day != today {
                        t.evening_nudged_day = today;
                        nudges.evening = true;
                    }
                }

                // Freeze while suspended, paused, idle, or outside work hours.
                if slept || t.paused || idle_now || !within_work_hours(&s) {
                    (Tick::None, nudges)
                } else {
                    let mut action = Tick::None;

                    if t.remaining > 0 {
                        t.remaining -= 1;
                    }

                    // Periodic nudges while working.
                    if t.phase == Phase::Working {
                        if s.blink_enabled {
                            if t.blink_remaining > 0 {
                                t.blink_remaining -= 1;
                            }
                            if t.blink_remaining == 0 {
                                t.blink_remaining = s.blink_interval_secs.max(1);
                                nudges.blink = true;
                            }
                        }
                        if s.hydration_enabled {
                            if t.hydration_remaining > 0 {
                                t.hydration_remaining -= 1;
                            }
                            if t.hydration_remaining == 0 {
                                t.hydration_remaining = s.hydration_interval_secs.max(1);
                                nudges.hydration = true;
                            }
                        }
                        if s.posture_enabled {
                            if t.posture_remaining > 0 {
                                t.posture_remaining -= 1;
                            }
                            if t.posture_remaining == 0 {
                                t.posture_remaining = s.posture_interval_secs.max(1);
                                nudges.posture = true;
                            }
                        }
                        if s.eyedrops_enabled {
                            if t.eyedrops_remaining > 0 {
                                t.eyedrops_remaining -= 1;
                            }
                            if t.eyedrops_remaining == 0 {
                                t.eyedrops_remaining = s.eyedrops_interval_secs.max(1);
                                nudges.eyedrops = true;
                            }
                        }
                    }

                    // Pre-break heads-up while still working.
                    if t.phase == Phase::Working
                        && !t.warned
                        && s.pre_break_warning_secs > 0
                        && t.remaining == s.pre_break_warning_secs
                    {
                        t.warned = true;
                        action = Tick::Prewarn(s.pre_break_warning_secs);
                    }

                    // Phase transition when the countdown hits zero.
                    if t.remaining == 0 {
                        match t.phase {
                            Phase::Working => {
                                to_break(&mut t, &s);
                                action = Tick::BreakStart;
                            }
                            Phase::Break => {
                                to_working(&mut t, &s);
                                action = Tick::BreakEnd;
                            }
                        }
                    }

                    (action, nudges)
                }
            };

            if nudges.blink {
                notify(&app, "Blink 👀", "Blink slowly and fully a few times.");
                let _ = app.emit("timer:blink", ());
            }
            if nudges.hydration {
                notify(&app, "Hydrate 💧", "Take a sip of water — dry eyes thank you.");
            }
            if nudges.posture {
                notify(
                    &app,
                    "Posture check",
                    "Sit back, relax your shoulders, screen about an arm's length away.",
                );
            }
            if nudges.eyedrops {
                notify(
                    &app,
                    "Eye drops 💧",
                    "Time for artificial tears / eye drops.",
                );
            }
            if nudges.evening {
                notify(
                    &app,
                    "Evening eyes",
                    "Warm your screen (night mode) and avoid using it in the dark.",
                );
            }

            match action {
                Tick::Prewarn(secs) => {
                    let _ = app
                        .notification()
                        .builder()
                        .title("Eye break soon")
                        .body(&format!("Break starting in {} seconds.", secs))
                        .show();
                    let _ = app.emit("timer:prewarn", secs);
                }
                Tick::BreakStart => {
                    start_break(&app);
                    let _ = app.emit("timer:break-start", ());
                }
                Tick::BreakEnd => {
                    record_break(&app, true);
                    close_break_window(&app);
                    let escalation = app
                        .state::<AppState>()
                        .settings
                        .lock()
                        .unwrap()
                        .escalation
                        .clone();
                    if escalation != "gentle" {
                        let _ = app
                            .notification()
                            .builder()
                            .title("Break over")
                            .body("Back to work — your eyes thank you.")
                            .show();
                    }
                    let _ = app.emit("timer:break-end", ());
                }
                Tick::None => {}
            }

            emit_tick(&app);
        }
    });
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    // Single instance must be registered first so a second launch focuses the
    // running copy instead of starting a new one.
    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                show_main_window(app);
            }))
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                None,
            ))
            .plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(|app, shortcut, event| {
                        if event.state() != ShortcutState::Pressed {
                            return;
                        }
                        let action = {
                            let state = app.state::<AppState>();
                            let map = state.shortcuts.lock().unwrap();
                            map.iter().find_map(|(accel, act)| {
                                accel
                                    .parse::<Shortcut>()
                                    .ok()
                                    .filter(|sc| sc == shortcut)
                                    .map(|_| *act)
                            })
                        };
                        match action {
                            Some(ShortAction::Pause) => do_toggle_pause(app),
                            Some(ShortAction::Skip) => do_skip(app),
                            Some(ShortAction::Take) => do_take_break(app),
                            Some(ShortAction::Postpone) => {
                                let _ = do_postpone(app);
                            }
                            Some(ShortAction::ToggleWidget) => do_toggle_widget(app),
                            None => {}
                        }
                    })
                    .build(),
            )
            .plugin(tauri_plugin_updater::Builder::new().build());
    }

    builder
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // Load settings and seed the timer.
            let settings = load_settings(&handle);
            let timer = TimerState {
                phase: Phase::Working,
                remaining: settings.work_interval_secs,
                paused: false,
                postpones_used: 0,
                warned: false,
                breaks_done: 0,
                is_long: false,
                blink_remaining: settings.blink_interval_secs.max(1),
                idle: false,
                hydration_remaining: settings.hydration_interval_secs.max(1),
                posture_remaining: settings.posture_interval_secs.max(1),
                eyedrops_remaining: settings.eyedrops_interval_secs.max(1),
                evening_nudged_day: -1,
            };
            app.manage(AppState {
                settings: Mutex::new(settings),
                timer: Mutex::new(timer),
                main_shown: Mutex::new(true),
                shortcuts: Mutex::new(Vec::new()),
                widget_hidden_override: Mutex::new(false),
            });

            // Tray icon + quick menu.
            let open_i = MenuItemBuilder::with_id("open", "Open EyeCare").build(app)?;
            let pause_i = MenuItemBuilder::with_id("pause", "Pause / Resume").build(app)?;
            let take_i = MenuItemBuilder::with_id("take", "Take a break now").build(app)?;
            let skip_i = MenuItemBuilder::with_id("skip", "Skip break").build(app)?;
            let widget_i =
                MenuItemBuilder::with_id("widget", "Hide / show widget").build(app)?;
            let quit_i = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&open_i)
                .item(&pause_i)
                .item(&take_i)
                .item(&skip_i)
                .item(&widget_i)
                .separator()
                .item(&quit_i)
                .build()?;

            TrayIconBuilder::with_id("main-tray")
                .icon(app_image())
                .tooltip("EyeCare")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_main_window(app),
                    "pause" => do_toggle_pause(app),
                    "take" => do_take_break(app),
                    "skip" => do_skip(app),
                    "widget" => do_toggle_widget(app),
                    "quit" => {
                        QUITTING.store(true, Ordering::SeqCst);
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // Set the real window icon (default_window_icon is Tauri's fallback).
            for label in ["main", "widget"] {
                if let Some(w) = app.get_webview_window(label) {
                    let _ = w.set_icon(app_image());
                }
            }

            // Closing/minimizing the main window destroys its webview to free
            // memory (the app stays alive in the tray); the floating widget
            // appears in its place. The window is rebuilt on restore.
            if let Some(main) = app.get_webview_window("main") {
                attach_main_close_handler(&main);
            }

            // Floating widget: apply saved size/position and remember where the
            // user drags it.
            apply_widget_config(&handle);
            if let Some(widget) = app.get_webview_window("widget") {
                // Exclude the widget from screen capture (Windows/macOS; no-op
                // on Linux/X11). Combined with fullscreen-suppression this keeps
                // it off shared/recorded screens.
                let _ = widget.set_content_protected(true);
                let wh = handle.clone();
                let w_scale = widget.clone();
                widget.on_window_event(move |event| match event {
                    WindowEvent::Moved(pos) => {
                        let state = wh.state::<AppState>();
                        let mut s = state.settings.lock().unwrap();
                        s.widget_x = Some(pos.x as f64);
                        s.widget_y = Some(pos.y as f64);
                        save_settings(&wh, &s);
                    }
                    WindowEvent::Resized(size) => {
                        // Persist the dragged size (store logical px so it matches
                        // the values shown in Settings).
                        let sf = w_scale.scale_factor().unwrap_or(1.0);
                        let logical = size.to_logical::<f64>(sf);
                        if logical.width < 1.0 || logical.height < 1.0 {
                            return; // ignore the minimize-to-zero event
                        }
                        let state = wh.state::<AppState>();
                        let mut s = state.settings.lock().unwrap();
                        s.widget_width = (logical.width.round() as u32).clamp(120, 480);
                        s.widget_height = (logical.height.round() as u32).clamp(120, 480);
                        save_settings(&wh, &s);
                    }
                    _ => {}
                });
            }
            update_widget_visibility(&handle);

            // Apply startup + global-hotkey settings.
            #[cfg(desktop)]
            {
                apply_autostart(&handle);
                register_shortcuts(&handle);
            }

            // Start the authoritative 1-second timer.
            run_timer(handle);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_settings,
            get_timer,
            timer_set_paused,
            timer_skip,
            timer_take_break,
            timer_postpone,
            show_main,
            check_update,
            install_update,
            get_stats
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            // Keep running in the tray when the last window closes (e.g. the
            // main window was minimized away). Only a real Quit exits.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !QUITTING.load(Ordering::SeqCst) {
                    api.prevent_exit();
                }
            }
        });
}
