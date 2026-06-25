// EyeBreak — Rust backend.
//
// Owns the authoritative timer (work interval -> break -> repeat), the user
// settings (persisted to a local JSON file), the system-tray icon, and the
// break reminder window. The frontend is a thin view that listens to events
// and calls commands; all timing lives here so it stays precise and survives
// webview reloads.

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_notification::NotificationExt;

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
}

struct AppState {
    settings: Mutex<Settings>,
    timer: Mutex<TimerState>,
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
}

fn snapshot(t: &TimerState, s: &Settings) -> TimerSnapshot {
    let total = match t.phase {
        Phase::Working => s.work_interval_secs,
        Phase::Break => s.break_length_secs,
    };
    TimerSnapshot {
        phase: t.phase,
        remaining: t.remaining,
        total,
        paused: t.paused,
        postpones_used: t.postpones_used,
        max_postpones: s.max_postpones,
    }
}

fn to_working(t: &mut TimerState, s: &Settings) {
    t.phase = Phase::Working;
    t.remaining = s.work_interval_secs;
    t.warned = false;
    t.postpones_used = 0;
}

fn to_break(t: &mut TimerState, s: &Settings) {
    t.phase = Phase::Break;
    t.remaining = s.break_length_secs;
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
// Break window + tray helpers
// ---------------------------------------------------------------------------

/// Open the break reminder window (or fire a notification for "gentle").
fn start_break(app: &AppHandle) {
    let (escalation, sound) = {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap();
        (s.escalation.clone(), s.sound_enabled)
    };

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
        .title("EyeBreak — break time")
        .inner_size(560.0, 380.0)
        .resizable(false)
        .always_on_top(forced)
        .decorations(!forced)
        .skip_taskbar(forced)
        .focused(forced)
        .center();

    if forced {
        builder = builder.fullscreen(true);
    }

    let _ = builder.build();
}

fn close_break_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("break") {
        let _ = w.close();
    }
}

fn update_tray(app: &AppHandle, snap: &TimerSnapshot) {
    if let Some(tray) = app.tray_by_id("main-tray") {
        let mm = snap.remaining / 60;
        let ss = snap.remaining % 60;
        let label = if snap.paused {
            format!("EyeBreak — paused ({:02}:{:02})", mm, ss)
        } else {
            match snap.phase {
                Phase::Working => format!("EyeBreak — next break in {:02}:{:02}", mm, ss),
                Phase::Break => format!("EyeBreak — break: {:02}:{:02} left", mm, ss),
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
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
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
    {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap().clone();
        let mut t = state.timer.lock().unwrap();
        to_working(&mut t, &s);
    }
    close_break_window(app);
    emit_tick(app);
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
fn timer_postpone(app: AppHandle, state: State<AppState>) -> bool {
    let allowed = {
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
        close_break_window(&app);
    }
    emit_tick(&app);
    allowed
}

#[tauri::command]
fn show_main(app: AppHandle) {
    show_main_window(&app);
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

fn run_timer(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        loop {
            ticker.tick().await;

            let action = {
                let state = app.state::<AppState>();
                let s = state.settings.lock().unwrap().clone();
                let mut t = state.timer.lock().unwrap();

                if t.paused {
                    Tick::None
                } else {
                    let mut action = Tick::None;

                    if t.remaining > 0 {
                        t.remaining -= 1;
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

                    action
                }
            };

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
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }));
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
            };
            app.manage(AppState {
                settings: Mutex::new(settings),
                timer: Mutex::new(timer),
            });

            // Tray icon + quick menu.
            let open_i = MenuItemBuilder::with_id("open", "Open EyeBreak").build(app)?;
            let pause_i = MenuItemBuilder::with_id("pause", "Pause / Resume").build(app)?;
            let take_i = MenuItemBuilder::with_id("take", "Take a break now").build(app)?;
            let skip_i = MenuItemBuilder::with_id("skip", "Skip break").build(app)?;
            let quit_i = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&open_i)
                .item(&pause_i)
                .item(&take_i)
                .item(&skip_i)
                .separator()
                .item(&quit_i)
                .build()?;

            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("EyeBreak")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_main_window(app),
                    "pause" => do_toggle_pause(app),
                    "take" => do_take_break(app),
                    "skip" => do_skip(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Closing the main window hides it to the tray instead of quitting.
            if let Some(main) = app.get_webview_window("main") {
                let main_clone = main.clone();
                main.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = main_clone.hide();
                    }
                });
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
            show_main
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
