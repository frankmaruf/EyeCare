// EyeCare native (Rust + Slint). Dashboard + break overlay + settings + floating
// widget + wellbeing-nudge notifications, driven by the ported timer engine with
// persisted settings. Tray, work-hours/idle/DND suppression, global shortcuts,
// autostart, updater and packaging land in later increments. Mirrors the design
// of the Tauri build (eyecare/src).

mod platform;
mod settings;
mod stats;
mod timer;
mod updater;
#[cfg(target_os = "linux")]
mod tray;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use i_slint_backend_winit::WinitWindowAccessor;
use settings::Settings;
use stats::Stats;
use timer::{Event, Phase, Timer};

slint::include_modules!();

/// Cache "idle detection unavailable" (e.g. no X screensaver extension) so we
/// stop probing and just never gate on idle.
static IDLE_UNAVAIL: AtomicBool = AtomicBool::new(false);

fn idle_seconds() -> u64 {
    if IDLE_UNAVAIL.load(Ordering::Relaxed) {
        return 0;
    }
    match user_idle::UserIdle::get_time() {
        Ok(t) => t.as_seconds(),
        Err(_) => {
            IDLE_UNAVAIL.store(true, Ordering::Relaxed);
            0
        }
    }
}

/// True if now falls inside the configured work hours + weekdays.
fn within_work_hours(s: &Settings) -> bool {
    use chrono::{Datelike, Local, Timelike};
    let now = Local::now();
    let wd = now.weekday().num_days_from_monday() as usize; // Mon=0..Sun=6
    if !s.work_days.get(wd).copied().unwrap_or(true) {
        return false;
    }
    let parse = |t: &str| -> u32 {
        let mut it = t.split(':');
        let h: u32 = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0);
        let m: u32 = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0);
        h * 60 + m
    };
    let cur = now.hour() * 60 + now.minute();
    let (start, end) = (parse(&s.work_start), parse(&s.work_end));
    if start <= end {
        cur >= start && cur < end
    } else {
        cur >= start || cur < end // overnight span
    }
}

const TIPS: &[&str] = &[
    "20-20-20: every 20 min, look ~20 ft away for 20 sec.",
    "Blink fully and often — screens cut your blink rate in half.",
    "Keep your screen about an arm's length away.",
    "Position the top of the monitor at or just below eye level.",
    "Dry eyes? A deliberate slow blink spreads the tear film.",
];

fn fmt(secs: u64) -> slint::SharedString {
    format!("{:02}:{:02}", secs / 60, secs % 60).into()
}

/// Dev-only: dump a rendered window to a PNG so the layout can be inspected
/// headlessly (EYECARE_SNAP=1). Uses the png crate (Linux dep).
#[cfg(target_os = "linux")]
fn save_png(buf: &slint::SharedPixelBuffer<slint::Rgba8Pixel>, path: &str) {
    if let Ok(file) = std::fs::File::create(path) {
        let mut enc = png::Encoder::new(file, buf.width(), buf.height());
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        if let Ok(mut w) = enc.write_header() {
            let _ = w.write_image_data(buf.as_bytes());
        }
    }
}

/// Fire a desktop notification (cross-platform via notify-rust).
fn notify(title: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .appname("EyeCare")
        .show();
}

/// "≈ once an hour" style hint shown under the long-break controls.
fn long_hint(s: &Settings) -> String {
    let gap_min = (s.work_interval_secs / 60).max(1) * s.long_break_every.max(1) as u64;
    let gap = if gap_min >= 60 && gap_min % 60 == 0 {
        format!("~{} h", gap_min / 60)
    } else if gap_min >= 60 {
        format!("~{:.1} h", gap_min as f64 / 60.0)
    } else {
        format!("~{} min", gap_min)
    };
    format!(
        "→ a {}-min long break about every {} (every {} break{}). The others stay short.",
        s.long_break_secs / 60,
        gap,
        s.long_break_every,
        if s.long_break_every == 1 { "" } else { "s" }
    )
}

const SHAPES: [&str; 3] = ["round", "squircle", "square"];
const ESCALATIONS: [&str; 3] = ["gentle", "standard", "forced"];
const ACCENTS: [&str; 6] = ["#4cc6c0", "#34d399", "#5b8def", "#e2725b", "#a78bfa", "#f59e0b"];
const LAYERS: [&str; 3] = ["above", "normal", "below"];
const WIDGET_MODES: [&str; 3] = ["off", "minimized", "always"];
const WIDGET_BGS: [&str; 3] = ["solid", "translucent", "transparent"];
const BREAK_END_SIGNALS: [&str; 4] = ["off", "notification", "chime", "both"];
const OVERLAY_SCOPES: [&str; 2] = ["active", "all"];

/// Index of `val` in `list`, defaulting to `default`. Keeps the combo/swatch
/// <-> string mapping in one place.
fn to_idx(list: &[&str], val: &str, default: i32) -> i32 {
    list.iter().position(|&s| s == val).map(|i| i as i32).unwrap_or(default)
}

fn from_idx(list: &[&str], idx: i32, default: &str) -> String {
    list.get(idx.max(0) as usize).copied().unwrap_or(default).to_string()
}

fn shape_to_idx(shape: &str) -> i32 {
    to_idx(&SHAPES, shape, 1)
}

fn idx_to_shape(idx: i32) -> String {
    from_idx(&SHAPES, idx, "squircle")
}

/// Enable/disable launch-at-login (cross-platform via auto-launch).
fn apply_autostart(enabled: bool) {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(path) = exe.to_str() else { return };
    if let Ok(al) = auto_launch::AutoLaunchBuilder::new()
        .set_app_name("EyeCare")
        .set_app_path(path)
        .build()
    {
        let _ = if enabled { al.enable() } else { al.disable() };
    }
}

/// Parse "HH:MM" into (hour, minute) ints.
fn parse_hm(t: &str) -> (i32, i32) {
    let mut it = t.split(':');
    let h = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(9);
    let m = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0);
    (h, m)
}

/// Parse "#rrggbb" into a Slint color (falls back to the default accent).
fn parse_color(hex: &str) -> slint::Color {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x4cc6c0);
    slint::Color::from_rgb_u8((n >> 16) as u8, (n >> 8) as u8, n as u8)
}

fn populate_settings(w: &MainWindow, s: &Settings) {
    w.set_work_min((s.work_interval_secs / 60) as i32);
    w.set_break_sec(s.break_length_secs as i32);
    w.set_prewarn_sec(s.pre_break_warning_secs as i32);
    w.set_escalation_idx(to_idx(&ESCALATIONS, &s.escalation, 1));
    w.set_sound_enabled(s.sound_enabled);
    w.set_accent_idx(to_idx(&ACCENTS, &s.accent, 0));
    w.set_reduce_motion(s.reduce_motion);
    w.set_high_contrast(s.high_contrast);
    w.set_autostart(s.autostart);
    w.set_idle_enabled(s.idle_pause_enabled);
    w.set_idle_sec(s.idle_threshold_secs as i32);
    w.set_wh_enabled(s.work_hours_enabled);
    let (sh, sm) = parse_hm(&s.work_start);
    let (eh, em) = parse_hm(&s.work_end);
    w.set_wh_start_h(sh);
    w.set_wh_start_m(sm);
    w.set_wh_end_h(eh);
    w.set_wh_end_m(em);
    let day = |i: usize| s.work_days.get(i).copied().unwrap_or(true);
    w.set_wd_mon(day(0));
    w.set_wd_tue(day(1));
    w.set_wd_wed(day(2));
    w.set_wd_thu(day(3));
    w.set_wd_fri(day(4));
    w.set_wd_sat(day(5));
    w.set_wd_sun(day(6));
    w.set_evening_enabled(s.evening_nudge_enabled);
    w.set_evening_hour(s.evening_hour as i32);
    w.set_snooze_min((s.snooze_secs / 60) as i32);
    w.set_max_postpones(s.max_postpones as i32);
    w.set_long_enabled(s.long_break_enabled);
    w.set_long_every(s.long_break_every as i32);
    w.set_long_min((s.long_break_secs / 60) as i32);
    w.set_tips_enabled(s.tips_enabled);
    w.set_exercises_enabled(s.exercises_enabled);
    w.set_calm_enabled(s.calm_visuals_enabled);
    w.set_stats_enabled(s.stats_enabled);
    w.set_blink_enabled(s.blink_enabled);
    w.set_blink_min((s.blink_interval_secs / 60).max(1) as i32);
    w.set_hydration_enabled(s.hydration_enabled);
    w.set_hydration_min((s.hydration_interval_secs / 60).max(1) as i32);
    w.set_posture_enabled(s.posture_enabled);
    w.set_posture_min((s.posture_interval_secs / 60).max(1) as i32);
    w.set_eyedrops_enabled(s.eyedrops_enabled);
    w.set_eyedrops_min((s.eyedrops_interval_secs / 60).max(1) as i32);
    w.set_widget_shape_idx(shape_to_idx(&s.widget_shape));
    w.set_widget_opacity(s.widget_opacity as i32);
    w.set_widget_w(s.widget_width as i32);
    w.set_widget_h(s.widget_height as i32);
    // window & overlay
    w.set_window_layer_idx(to_idx(&LAYERS, &s.window_layer, 1));
    w.set_allow_forced(s.allow_forced);
    w.set_overlay_scope_idx(to_idx(&OVERLAY_SCOPES, &s.overlay_scope, 0));
    w.set_require_full_break(s.require_full_break);
    w.set_show_skip_button(s.show_skip_button);
    w.set_show_postpone_button(s.show_postpone_button);
    w.set_break_end_idx(to_idx(&BREAK_END_SIGNALS, &s.break_end_signal, 3));
    w.set_sound_volume(s.sound_volume as i32);
    w.set_respect_dnd(s.respect_dnd);
    w.set_suppress_presentation(s.suppress_presentation);
    w.set_shortcuts_enabled(s.shortcuts_enabled);
    w.set_darkroom_enabled(s.darkroom_enabled);
    // widget extras
    w.set_widget_mode_idx(to_idx(&WIDGET_MODES, &s.widget_mode, 2));
    w.set_widget_layer_idx(to_idx(&LAYERS, &s.widget_layer, 0));
    w.set_widget_bg_idx(to_idx(&WIDGET_BGS, &s.widget_bg, 1));
    w.set_widget_click_through(s.widget_click_through);
    w.set_long_hint(long_hint(s).into());
}

fn read_settings(w: &MainWindow, base: &Settings) -> Settings {
    Settings {
        work_interval_secs: w.get_work_min().max(1) as u64 * 60,
        break_length_secs: w.get_break_sec().max(5) as u64,
        pre_break_warning_secs: w.get_prewarn_sec().max(0) as u64,
        escalation: from_idx(&ESCALATIONS, w.get_escalation_idx(), "standard"),
        sound_enabled: w.get_sound_enabled(),
        accent: from_idx(&ACCENTS, w.get_accent_idx(), "#4cc6c0"),
        reduce_motion: w.get_reduce_motion(),
        high_contrast: w.get_high_contrast(),
        autostart: w.get_autostart(),
        idle_pause_enabled: w.get_idle_enabled(),
        idle_threshold_secs: w.get_idle_sec().max(10) as u64,
        work_hours_enabled: w.get_wh_enabled(),
        work_start: format!("{:02}:{:02}", w.get_wh_start_h().clamp(0, 23), w.get_wh_start_m().clamp(0, 59)),
        work_end: format!("{:02}:{:02}", w.get_wh_end_h().clamp(0, 23), w.get_wh_end_m().clamp(0, 59)),
        work_days: vec![
            w.get_wd_mon(),
            w.get_wd_tue(),
            w.get_wd_wed(),
            w.get_wd_thu(),
            w.get_wd_fri(),
            w.get_wd_sat(),
            w.get_wd_sun(),
        ],
        evening_nudge_enabled: w.get_evening_enabled(),
        evening_hour: w.get_evening_hour().clamp(0, 23) as u32,
        snooze_secs: w.get_snooze_min().max(1) as u64 * 60,
        max_postpones: w.get_max_postpones().max(0) as u32,
        long_break_enabled: w.get_long_enabled(),
        long_break_every: w.get_long_every().max(1) as u32,
        long_break_secs: w.get_long_min().max(1) as u64 * 60,
        tips_enabled: w.get_tips_enabled(),
        exercises_enabled: w.get_exercises_enabled(),
        calm_visuals_enabled: w.get_calm_enabled(),
        stats_enabled: w.get_stats_enabled(),
        blink_enabled: w.get_blink_enabled(),
        blink_interval_secs: w.get_blink_min().max(1) as u64 * 60,
        hydration_enabled: w.get_hydration_enabled(),
        hydration_interval_secs: w.get_hydration_min().max(5) as u64 * 60,
        posture_enabled: w.get_posture_enabled(),
        posture_interval_secs: w.get_posture_min().max(5) as u64 * 60,
        eyedrops_enabled: w.get_eyedrops_enabled(),
        eyedrops_interval_secs: w.get_eyedrops_min().max(5) as u64 * 60,
        widget_shape: idx_to_shape(w.get_widget_shape_idx()),
        widget_opacity: w.get_widget_opacity().clamp(20, 100) as u32,
        widget_width: w.get_widget_w().clamp(settings::WIDGET_MIN as i32, settings::WIDGET_MAX as i32) as u32,
        widget_height: w.get_widget_h().clamp(settings::WIDGET_MIN as i32, settings::WIDGET_MAX as i32) as u32,
        window_layer: from_idx(&LAYERS, w.get_window_layer_idx(), "normal"),
        allow_forced: w.get_allow_forced(),
        overlay_scope: from_idx(&OVERLAY_SCOPES, w.get_overlay_scope_idx(), "active"),
        require_full_break: w.get_require_full_break(),
        show_skip_button: w.get_show_skip_button(),
        show_postpone_button: w.get_show_postpone_button(),
        break_end_signal: from_idx(&BREAK_END_SIGNALS, w.get_break_end_idx(), "both"),
        sound_volume: w.get_sound_volume().clamp(0, 100) as u32,
        respect_dnd: w.get_respect_dnd(),
        suppress_presentation: w.get_suppress_presentation(),
        shortcuts_enabled: w.get_shortcuts_enabled(),
        darkroom_enabled: w.get_darkroom_enabled(),
        widget_mode: from_idx(&WIDGET_MODES, w.get_widget_mode_idx(), "always"),
        widget_layer: from_idx(&LAYERS, w.get_widget_layer_idx(), "above"),
        widget_bg: from_idx(&WIDGET_BGS, w.get_widget_bg_idx(), "translucent"),
        widget_click_through: w.get_widget_click_through(),
        // preserve fields not shown in the settings UI (widget position, …)
        ..base.clone()
    }
}

/// X11 window id of a winit window (winit 0.30 exposes it via raw-window-handle).
#[cfg(target_os = "linux")]
fn x11_window_id(win: &i_slint_backend_winit::winit::window::Window) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match win.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(x) => Some(x.window as u32),
        RawWindowHandle::Xcb(x) => Some(x.window.get()),
        _ => None,
    }
}

/// Single-instance guard (spec §5): if a live instance holds the lock, exit.
#[cfg(target_os = "linux")]
fn ensure_single_instance() {
    use std::io::Read;
    let Some(mut p) = dirs::runtime_dir().or_else(dirs::config_dir) else { return };
    p.push("eyecare-native.lock");
    if let Ok(mut f) = std::fs::File::open(&p) {
        let mut s = String::new();
        let _ = f.read_to_string(&mut s);
        if let Ok(pid) = s.trim().parse::<i32>() {
            if std::path::Path::new(&format!("/proc/{pid}")).exists() {
                eprintln!("[eyecare] already running (pid {pid}); exiting");
                std::process::exit(0);
            }
        }
    }
    let _ = std::fs::write(&p, std::process::id().to_string());
}

fn main() -> Result<(), slint::PlatformError> {
    // Force XWayland (X11). Native Wayland (KWin) restricts a client moving /
    // resizing / always-on-topping / re-showing its own windows — which breaks
    // widget drag/resize, tray "Open", and keep-above. X11 restores all of it
    // (spec §7.8). The Tauri build did the same via GDK_BACKEND=x11.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        std::env::set_var("WINIT_UNIX_BACKEND", "x11");
        std::env::remove_var("WAYLAND_DISPLAY");
    }

    #[cfg(target_os = "linux")]
    ensure_single_instance();

    // Set a stable Wayland app_id / X11 WM_CLASS so the compositor can match the
    // installed .desktop and show the EyeCare icon (instead of a generic one).
    #[cfg(target_os = "linux")]
    {
        use i_slint_backend_winit::winit::platform::wayland::WindowAttributesExtWayland;
        if let Ok(backend) = i_slint_backend_winit::Backend::builder()
            .with_window_attributes_hook(|attrs| {
                attrs.with_name("us.frankmaruf.eyecare-native", "EyeCare")
            })
            .build()
        {
            let _ = slint::platform::set_platform(Box::new(backend));
        }
    }

    let settings = Rc::new(RefCell::new(Settings::load()));
    let timer = Rc::new(RefCell::new(Timer::new(&settings.borrow())));
    let stats = Rc::new(RefCell::new(Stats::load()));
    // 0 = running, 1 = idle-paused, 2 = outside work hours
    let gate = Rc::new(Cell::new(0u8));
    let evening_day = Rc::new(Cell::new(-1i64));
    let darkroom_day = Rc::new(Cell::new(-1i64));

    let main_win = MainWindow::new()?;
    let break_win = BreakWindow::new()?;
    let widget_win = WidgetWindow::new()?;

    // Closing the dashboard hides it (so the tray "Open" can re-show it) instead
    // of destroying the surface — re-showing a destroyed Wayland window fails.
    main_win
        .window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);

    // require-full-break: refuse to close the break overlay until the timer ends
    // (§4.6); otherwise just hide it.
    {
        let settings = settings.clone();
        break_win.window().on_close_requested(move || {
            if settings.borrow().require_full_break {
                slint::CloseRequestResponse::KeepWindowShown
            } else {
                slint::CloseRequestResponse::HideWindow
            }
        });
    }

    // Push appearance settings onto a window's Theme global (each top-level
    // window owns its own global instance, so we apply to all of them).
    macro_rules! set_theme {
        ($w:expr, $accent:expr, $rm:expr, $hc:expr) => {{
            let g = $w.global::<Theme>();
            g.set_accent($accent);
            g.set_reduce_motion($rm);
            g.set_high_contrast($hc);
        }};
    }
    {
        let s = settings.borrow();
        let c = parse_color(&s.accent);
        set_theme!(main_win, c, s.reduce_motion, s.high_contrast);
        set_theme!(break_win, c, s.reduce_motion, s.high_contrast);
        set_theme!(widget_win, c, s.reduce_motion, s.high_contrast);
        apply_autostart(s.autostart);
    }

    // Apply window stacking (above/normal/below) + widget click-through. Called
    // at startup and after Save.
    let apply_chrome: Rc<dyn Fn()> = Rc::new({
        let main_w = main_win.as_weak();
        let widget_w = widget_win.as_weak();
        let settings = settings.clone();
        move || {
            use i_slint_backend_winit::winit::window::WindowLevel;
            let level = |l: &str| match l {
                "above" => WindowLevel::AlwaysOnTop,
                "below" => WindowLevel::AlwaysOnBottom,
                _ => WindowLevel::Normal,
            };
            let s = settings.borrow();
            if let Some(m) = main_w.upgrade() {
                m.window()
                    .with_winit_window(|win| win.set_window_level(level(&s.window_layer)));
            }
            if let Some(wd) = widget_w.upgrade() {
                wd.window().with_winit_window(|win| {
                    win.set_window_level(level(&s.widget_layer));
                    let _ = win.set_cursor_hittest(!s.widget_click_through);
                });
            }
        }
    });

    // Single source of truth → every window. One closure keeps them in sync.
    let render = {
        let main_w = main_win.as_weak();
        let break_w = break_win.as_weak();
        let widget_w = widget_win.as_weak();
        let timer = timer.clone();
        let settings = settings.clone();
        let gate = gate.clone();
        move || {
            let t = timer.borrow();
            let time = fmt(t.remaining);
            if let Some(w) = main_w.upgrade() {
                w.set_progress(t.fraction());
                w.set_time_text(time.clone());
                let (tag, label) = match gate.get() {
                    1 => ("idle", "paused — you're away"),
                    2 => ("paused", "outside work hours"),
                    _ => match (t.paused, t.phase) {
                        (true, _) => ("paused", "paused"),
                        (_, Phase::Break) => ("break", "break time left"),
                        (_, Phase::Working) => ("working", "until next break"),
                    },
                };
                w.set_phase_tag(tag.into());
                w.set_label_text(label.into());
                w.set_pause_text(if t.paused { "Resume".into() } else { "Pause".into() });
            }
            if let Some(w) = widget_w.upgrade() {
                w.set_progress(t.fraction());
                w.set_time_text(time.clone());
                w.set_pause_icon(if t.paused { "▶".into() } else { "⏸".into() });
                let s = settings.borrow();
                w.set_shape(s.widget_shape.clone().into());
                w.set_phase_state(if t.paused {
                    2
                } else if t.phase == Phase::Break {
                    1
                } else {
                    0
                });
                // background mode → effective card opacity
                let base = (s.widget_opacity as f32 / 100.0).clamp(0.0, 1.0);
                w.set_opacity_frac(match s.widget_bg.as_str() {
                    "solid" => 1.0,
                    "transparent" => 0.0,
                    _ => base.max(0.2),
                });
                // show/hide per mode (off / when-minimized / always), hidden during
                // a forced overlay or a fullscreen presentation
                let main_min = main_w
                    .upgrade()
                    .map(|m| {
                        let vis = m.window().is_visible();
                        let min = m
                            .window()
                            .with_winit_window(|win| win.is_minimized().unwrap_or(false))
                            .unwrap_or(false);
                        !vis || min
                    })
                    .unwrap_or(true);
                let mut show = match s.widget_mode.as_str() {
                    "off" => false,
                    "minimized" => main_min,
                    _ => true,
                };
                if t.phase == Phase::Break && s.escalation == "forced" {
                    show = false;
                }
                if show && s.suppress_presentation && platform::another_app_fullscreen() {
                    show = false;
                }
                if show != w.window().is_visible() {
                    if show {
                        let _ = w.show();
                    } else {
                        let _ = w.hide();
                    }
                }
            }
            if let Some(w) = break_w.upgrade() {
                w.set_time_text(time);
                let bs = settings.borrow();
                w.set_calm_on(bs.calm_visuals_enabled);
                w.set_exercise_on(bs.exercises_enabled);
                w.set_is_long(t.is_long);
                // require-full-break hides Skip + Postpone until the break is done
                let req = bs.require_full_break;
                w.set_show_skip(bs.show_skip_button && !req);
                w.set_show_postpone(bs.show_postpone_button && !req);
                drop(bs);
                w.set_title_text(if t.is_long {
                    "Stand up & move".into()
                } else {
                    "Look ~20 feet away".into()
                });
                w.set_sub_text(if t.is_long {
                    "Longer break — stretch, walk, and look far away.".into()
                } else {
                    "Relax your eyes — let your focus drift to the distance.".into()
                });
            }
        }
    };

    // Show/hide the break overlay to match the current phase.
    let sync_break = {
        let break_w = break_win.as_weak();
        let timer = timer.clone();
        let settings = settings.clone();
        move || {
            let Some(w) = break_w.upgrade() else { return };
            let t = timer.borrow();
            let s = settings.borrow();
            // "forced" needs the allow-forced master switch; otherwise it falls
            // back to a standard window (spec §4.3).
            let forced = s.escalation == "forced" && s.allow_forced;
            // "gentle" intensity uses a notification only — no window grab.
            if t.phase == Phase::Break && s.escalation != "gentle" {
                // hush a non-forced overlay during a fullscreen presentation /
                // screen-share — the break-start notification already fired (§9.9).
                if !forced && s.suppress_presentation && platform::another_app_fullscreen() {
                    let _ = w.hide();
                    return;
                }
                if s.tips_enabled {
                    w.set_tip_text(format!("💡 {}", TIPS[t.breaks_done as usize % TIPS.len()]).into());
                } else {
                    w.set_tip_text("".into());
                }
                w.set_can_postpone(t.postpones_used < s.max_postpones);
                w.window().with_winit_window(|win| {
                    use i_slint_backend_winit::winit::window::{Fullscreen, WindowLevel};
                    win.set_fullscreen(if forced {
                        Some(Fullscreen::Borderless(None))
                    } else {
                        None
                    });
                    win.set_window_level(WindowLevel::AlwaysOnTop);
                });
                let _ = w.show();
                // bring the break overlay to the front + focus it (and keep it
                // out of the taskbar)
                w.window().with_winit_window(|win| {
                    win.set_visible(true);
                    win.focus_window();
                    #[cfg(target_os = "linux")]
                    if let Some(xid) = x11_window_id(win) {
                        platform::x11_skip_taskbar(xid);
                    }
                });
            } else {
                let _ = w.hide();
            }
        }
    };

    render();
    let _ = break_win.hide();
    let _ = widget_win.show();
    apply_chrome();
    // keep the widget out of the taskbar — it lives in the tray (X11)
    #[cfg(target_os = "linux")]
    {
        widget_win.window().with_winit_window(|win| {
            if let Some(xid) = x11_window_id(win) {
                platform::x11_skip_taskbar(xid);
            }
        });
        // re-apply once the window is surely mapped (the WM ignores state changes
        // sent before map)
        let ww = widget_win.as_weak();
        slint::Timer::single_shot(Duration::from_millis(700), move || {
            if let Some(wd) = ww.upgrade() {
                wd.window().with_winit_window(|win| {
                    if let Some(xid) = x11_window_id(win) {
                        platform::x11_skip_taskbar(xid);
                    }
                });
            }
        });
    }

    // 1-second tick.
    let ticker = slint::Timer::default();
    {
        let timer = timer.clone();
        let settings = settings.clone();
        let render = render.clone();
        let sync_break = sync_break.clone();
        let gate = gate.clone();
        let evening_day = evening_day.clone();
        let darkroom_day = darkroom_day.clone();
        let stats = stats.clone();
        ticker.start(slint::TimerMode::Repeated, Duration::from_secs(1), move || {
            // Evening warm-screen nudge: once per day at the chosen hour.
            {
                let s = settings.borrow();
                if s.evening_nudge_enabled {
                    use chrono::{Datelike, Local, Timelike};
                    let now = Local::now();
                    let ord = now.num_days_from_ce() as i64;
                    if now.hour() == s.evening_hour && evening_day.get() != ord {
                        evening_day.set(ord);
                        notify(
                            "Evening eyes",
                            "Warm your screen (night mode) and avoid using it in the dark.",
                        );
                    }
                    // dark-room warning: once per evening, an hour after the warm
                    // nudge (§9.5)
                    if s.darkroom_enabled
                        && now.hour() == (s.evening_hour + 1) % 24
                        && darkroom_day.get() != ord
                    {
                        darkroom_day.set(ord);
                        notify(
                            "Room too dark?",
                            "Match screen brightness to the room — a bright screen in the dark strains the eyes.",
                        );
                    }
                }
            }
            // Gate the countdown: idle, or outside work hours.
            let g = {
                let s = settings.borrow();
                if s.idle_pause_enabled && idle_seconds() >= s.idle_threshold_secs {
                    1u8
                } else if s.work_hours_enabled && !within_work_hours(&s) {
                    2
                } else {
                    0
                }
            };
            gate.set(g);
            if g != 0 {
                render();
                return;
            }
            let res = timer.borrow_mut().tick(&settings.borrow());
            match res.event {
                Event::Prewarn => notify("Eye break soon", "A break is coming up — finish your thought."),
                Event::BreakStart => {
                    notify("Time for an eye break", "Look ~20 feet (6 m) away and relax your eyes.");
                    let s = settings.borrow();
                    if s.sound_enabled {
                        platform::play_sound(s.sound_volume);
                    }
                    drop(s);
                    sync_break();
                }
                Event::BreakEnd => {
                    let s = settings.borrow();
                    if s.stats_enabled {
                        stats.borrow_mut().record(true);
                    }
                    // break-over signal: off / notification / chime / both (§4.7)
                    let sig = s.break_end_signal.clone();
                    if sig == "notification" || sig == "both" {
                        notify("Break over", "Welcome back — your eyes thank you.");
                    }
                    if sig == "chime" || sig == "both" {
                        platform::play_sound(s.sound_volume);
                    }
                    drop(s);
                    sync_break();
                }
                Event::None => {}
            }
            let n = res.nudges;
            if n.blink {
                notify("Blink 👀", "Blink slowly and fully a few times.");
            }
            if n.hydration {
                notify("Hydrate 💧", "Take a sip of water — dry eyes thank you.");
            }
            if n.posture {
                notify("Posture check", "Sit back, relax your shoulders, screen an arm's length away.");
            }
            if n.eyedrops {
                notify("Eye drops 💧", "Time for artificial tears / eye drops.");
            }
            render();
        });
    }

    // Wire one action to its timer method + a UI refresh. Keeps callback bodies
    // one line each instead of repeating the clone/borrow boilerplate.
    macro_rules! action {
        ($win:ident . $cb:ident, |$t:ident, $s:ident| $body:expr) => {{
            let timer = timer.clone();
            let settings = settings.clone();
            let render = render.clone();
            let sync_break = sync_break.clone();
            $win.$cb(move || {
                {
                    let mut $t = timer.borrow_mut();
                    let $s = settings.borrow();
                    let _ = $body;
                }
                sync_break();
                render();
            });
        }};
    }

    action!(main_win.on_pause, |t, _s| {
        let p = !t.paused;
        t.set_paused(p)
    });
    action!(main_win.on_take_break, |t, s| t.take_break(&s));
    action!(main_win.on_skip, |t, s| t.skip(&s));
    action!(break_win.on_postpone, |t, s| t.postpone(&s));
    {
        let timer = timer.clone();
        let settings = settings.clone();
        let render = render.clone();
        let sync_break = sync_break.clone();
        let stats = stats.clone();
        break_win.on_skip(move || {
            let was_break = timer.borrow().phase == Phase::Break;
            timer.borrow_mut().skip(&settings.borrow());
            if was_break && settings.borrow().stats_enabled {
                stats.borrow_mut().record(false);
            }
            sync_break();
            render();
        });
    }
    action!(widget_win.on_pause, |t, _s| {
        let p = !t.paused;
        t.set_paused(p)
    });
    action!(widget_win.on_take_break, |t, s| t.take_break(&s));
    action!(widget_win.on_skip, |t, s| t.skip(&s));
    action!(widget_win.on_postpone, |t, s| t.postpone(&s));
    widget_win.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    {
        let mw = main_win.as_weak();
        let settings = settings.clone();
        widget_win.on_open_settings(move || {
            let Some(w) = mw.upgrade() else { return };
            populate_settings(&w, &settings.borrow());
            w.set_show_settings(true);
            let _ = w.show();
            w.window().with_winit_window(|win| {
                win.set_minimized(false);
                win.set_visible(true);
                win.focus_window();
            });
        });
    }

    // Settings page (rendered inside the dashboard window) — open / save / back
    // / export / import. Page-swap via `show-settings` (no separate window).
    {
        let mw = main_win.as_weak();
        let settings = settings.clone();
        let stats = stats.clone();
        main_win.on_open_settings(move || {
            let Some(w) = mw.upgrade() else { return };
            populate_settings(&w, &settings.borrow());
            let (streak, today, total) = stats.borrow().summary();
            w.set_stat_streak(streak as i32);
            w.set_stat_today(today as i32);
            w.set_stat_total(total as i32);
            w.set_app_version(updater::current_version().into());
            w.set_show_settings(true);
        });
    }
    {
        let mw = main_win.as_weak();
        let widget_w = widget_win.as_weak();
        let break_w = break_win.as_weak();
        let settings = settings.clone();
        let timer = timer.clone();
        let render = render.clone();
        let apply_chrome = apply_chrome.clone();
        main_win.on_save(move || {
            let Some(w) = mw.upgrade() else { return };
            let next = read_settings(&w, &settings.borrow());
            timer.borrow_mut().apply_settings(&next);
            next.save();
            *settings.borrow_mut() = next;
            let s = settings.borrow();
            let c = parse_color(&s.accent);
            set_theme!(w, c, s.reduce_motion, s.high_contrast);
            if let Some(b) = break_w.upgrade() {
                set_theme!(b, c, s.reduce_motion, s.high_contrast);
            }
            if let Some(wd) = widget_w.upgrade() {
                set_theme!(wd, c, s.reduce_motion, s.high_contrast);
                wd.window()
                    .set_size(slint::LogicalSize::new(s.widget_width as f32, s.widget_height as f32));
            }
            apply_autostart(s.autostart);
            w.set_long_hint(long_hint(&s).into());
            drop(s);
            apply_chrome();
            w.set_show_settings(false);
            render();
        });
    }
    {
        let mw = main_win.as_weak();
        main_win.on_back(move || {
            if let Some(w) = mw.upgrade() {
                w.set_show_settings(false);
            }
        });
    }

    // ---- global keyboard shortcuts (§4.8): Ctrl+Alt+ P/B/K/O ----
    let _hotkeys = {
        use global_hotkey::hotkey::{Code, HotKey, Modifiers};
        use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
        match GlobalHotKeyManager::new() {
            Err(e) => {
                eprintln!("[eyecare] global shortcuts unavailable: {e}");
                None
            }
            Ok(mgr) => {
                let m = Modifiers::CONTROL | Modifiers::ALT;
                let hk_pause = HotKey::new(Some(m), Code::KeyP);
                let hk_take = HotKey::new(Some(m), Code::KeyB);
                let hk_skip = HotKey::new(Some(m), Code::KeyK);
                let hk_post = HotKey::new(Some(m), Code::KeyO);
                for hk in [hk_pause, hk_take, hk_skip, hk_post] {
                    let _ = mgr.register(hk);
                }
                let (ip, it, ik, io) =
                    (hk_pause.id(), hk_take.id(), hk_skip.id(), hk_post.id());
                let rx = GlobalHotKeyEvent::receiver();
                let poll = slint::Timer::default();
                let timer = timer.clone();
                let settings = settings.clone();
                let render = render.clone();
                let sync_break = sync_break.clone();
                poll.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
                    let mut acted = false;
                    while let Ok(ev) = rx.try_recv() {
                        if ev.state != HotKeyState::Pressed {
                            continue;
                        }
                        if !settings.borrow().shortcuts_enabled {
                            continue;
                        }
                        {
                            let mut t = timer.borrow_mut();
                            let s = settings.borrow();
                            if ev.id == ip {
                                let p = !t.paused;
                                t.set_paused(p);
                            } else if ev.id == it {
                                t.take_break(&s);
                            } else if ev.id == ik {
                                t.skip(&s);
                            } else if ev.id == io {
                                t.postpone(&s);
                            }
                        }
                        acted = true;
                    }
                    if acted {
                        sync_break();
                        render();
                    }
                });
                Some((mgr, poll))
            }
        }
    };
    {
        let mw = main_win.as_weak();
        let settings = settings.clone();
        main_win.on_export_settings(move || {
            let Some(w) = mw.upgrade() else { return };
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("JSON", &["json"])
                .set_file_name("eyecare-settings.json")
                .save_file()
            {
                let ok = serde_json::to_string_pretty(&*settings.borrow())
                    .ok()
                    .and_then(|j| std::fs::write(&path, j).ok())
                    .is_some();
                w.set_backup_msg(if ok { "Exported ✓".into() } else { "Export failed".into() });
            }
        });
    }
    {
        let mw = main_win.as_weak();
        let widget_w = widget_win.as_weak();
        let break_w = break_win.as_weak();
        let settings = settings.clone();
        let timer = timer.clone();
        let render = render.clone();
        main_win.on_import_settings(move || {
            let Some(w) = mw.upgrade() else { return };
            let Some(path) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() else {
                return;
            };
            let next = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<Settings>(&t).ok());
            let Some(next) = next else {
                w.set_backup_msg("Import failed".into());
                return;
            };
            timer.borrow_mut().apply_settings(&next);
            next.save();
            *settings.borrow_mut() = next;
            let s = settings.borrow();
            let c = parse_color(&s.accent);
            set_theme!(w, c, s.reduce_motion, s.high_contrast);
            if let Some(b) = break_w.upgrade() {
                set_theme!(b, c, s.reduce_motion, s.high_contrast);
            }
            if let Some(wd) = widget_w.upgrade() {
                set_theme!(wd, c, s.reduce_motion, s.high_contrast);
                wd.window()
                    .set_size(slint::LogicalSize::new(s.widget_width as f32, s.widget_height as f32));
            }
            apply_autostart(s.autostart);
            populate_settings(&w, &s);
            drop(s);
            w.set_backup_msg("Imported ✓".into());
            render();
        });
    }

    // ---- updater: check GitHub releases for a newer native build ----
    let upd_asset: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let (upd_tx, upd_rx) = std::sync::mpsc::channel::<(String, Option<String>, bool)>();
    {
        let mw = main_win.as_weak();
        let upd_tx = upd_tx.clone();
        main_win.on_check_update(move || {
            if let Some(w) = mw.upgrade() {
                w.set_update_status("Checking…".into());
            }
            let tx = upd_tx.clone();
            std::thread::spawn(move || {
                let (status, asset, avail) = updater::evaluate();
                let _ = tx.send((status, asset, avail));
            });
        });
    }
    {
        let mw = main_win.as_weak();
        let upd_asset = upd_asset.clone();
        let upd_tx = upd_tx.clone();
        main_win.on_install_update(move || {
            let Some(url) = upd_asset.borrow().clone() else { return };
            if let Some(w) = mw.upgrade() {
                w.set_update_status("Downloading…".into());
            }
            let tx = upd_tx.clone();
            std::thread::spawn(move || match updater::install(&url) {
                Ok(exe) => {
                    let _ = tx.send(("Installed — restarting…".into(), None, false));
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    let _ = std::process::Command::new(exe).spawn();
                    std::process::exit(0);
                }
                Err(e) => {
                    let _ = tx.send((format!("Install failed: {e}"), None, false));
                }
            });
        });
    }
    let upd_timer = slint::Timer::default();
    {
        let mw = main_win.as_weak();
        let upd_asset = upd_asset.clone();
        upd_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(250),
            move || {
                while let Ok((status, asset, avail)) = upd_rx.try_recv() {
                    *upd_asset.borrow_mut() = asset;
                    if let Some(w) = mw.upgrade() {
                        w.set_update_status(status.into());
                        w.set_update_available(avail);
                    }
                }
            },
        );
    }

    // ---- floating widget: restore, geometry, drag, resize, persistence ----
    {
        let main_w = main_win.as_weak();
        widget_win.on_restore(move || {
            if let Some(w) = main_w.upgrade() {
                let _ = w.show();
            }
        });
    }
    // apply saved size/position + clamp the resizable range (120–480, as Tauri)
    {
        let s = settings.borrow();
        widget_win
            .window()
            .set_size(slint::LogicalSize::new(s.widget_width as f32, s.widget_height as f32));
        if let (Some(x), Some(y)) = (s.widget_x, s.widget_y) {
            widget_win
                .window()
                .set_position(slint::LogicalPosition::new(x as f32, y as f32));
        }
        widget_win.window().with_winit_window(|win| {
            use i_slint_backend_winit::winit::dpi::LogicalSize;
            use i_slint_backend_winit::winit::window::WindowLevel;
            win.set_resizable(true);
            win.set_window_level(WindowLevel::AlwaysOnTop);
            win.set_min_inner_size(Some(LogicalSize::new(
                settings::WIDGET_MIN as f64,
                settings::WIDGET_MIN as f64,
            )));
            win.set_max_inner_size(Some(LogicalSize::new(
                settings::WIDGET_MAX as f64,
                settings::WIDGET_MAX as f64,
            )));
        });
    }
    // press-drag to move; corner grip to resize (native interactive drags)
    {
        let widget_w = widget_win.as_weak();
        widget_win.on_start_drag(move || {
            if let Some(w) = widget_w.upgrade() {
                let r = w.window().with_winit_window(|win| win.drag_window());
                eprintln!("[eyecare] drag_window -> {:?}", r);
            }
        });
    }
    {
        let widget_w = widget_win.as_weak();
        widget_win.on_start_resize(move || {
            if let Some(w) = widget_w.upgrade() {
                use i_slint_backend_winit::winit::window::ResizeDirection;
                let r = w
                    .window()
                    .with_winit_window(|win| win.drag_resize_window(ResizeDirection::SouthEast));
                eprintln!("[eyecare] drag_resize_window -> {:?}", r);
            }
        });
    }
    // persist widget geometry (logical px) whenever it changes
    let geom_timer = slint::Timer::default();
    {
        let widget_w = widget_win.as_weak();
        let settings = settings.clone();
        geom_timer.start(slint::TimerMode::Repeated, Duration::from_millis(800), move || {
            let Some(w) = widget_w.upgrade() else { return };
            let win = w.window();
            let scale = win.scale_factor().max(0.1);
            let size = win.size();
            let pos = win.position();
            let (lw, lh) = (
                (size.width as f32 / scale).round() as u32,
                (size.height as f32 / scale).round() as u32,
            );
            let (lx, ly) = (
                (pos.x as f32 / scale).round() as i32,
                (pos.y as f32 / scale).round() as i32,
            );
            if lw < 1 || lh < 1 {
                return;
            }
            // Keep the widget square so the round dial always fills it. Snap
            // whenever the live window isn't square (resizing one edge), then
            // persist.
            let sq = lw.min(lh).clamp(settings::WIDGET_MIN, settings::WIDGET_MAX);
            if lw != lh || lw != sq {
                win.set_size(slint::LogicalSize::new(sq as f32, sq as f32));
            }
            let mut s = settings.borrow_mut();
            if s.widget_width != sq || s.widget_x != Some(lx) || s.widget_y != Some(ly) {
                s.widget_width = sq;
                s.widget_height = sq;
                s.widget_x = Some(lx);
                s.widget_y = Some(ly);
                s.save();
            }
        });
    }

    // ---- system tray (Linux; StatusNotifierItem) ----
    #[cfg(target_os = "linux")]
    let _tray = {
        let (tx, rx) = std::sync::mpsc::channel::<tray::Action>();
        let handle = tray::spawn(tx);
        let dispatch = slint::Timer::default();
        let main_w = main_win.as_weak();
        let widget_w = widget_win.as_weak();
        let timer = timer.clone();
        let settings = settings.clone();
        let render = render.clone();
        let sync_break = sync_break.clone();
        let widget_visible = Rc::new(Cell::new(true));
        let tip_handle = handle.clone();
        let last_tip = Rc::new(RefCell::new(String::new()));
        dispatch.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(150),
            move || {
                // Minimize → hide to the tray (not the taskbar): when the
                // dashboard gets minimized, un-minimize it and hide so it leaves
                // the taskbar entirely. Re-open via the tray / widget.
                if let Some(m) = main_w.upgrade() {
                    let min = m
                        .window()
                        .with_winit_window(|w| w.is_minimized().unwrap_or(false))
                        .unwrap_or(false);
                    if min {
                        m.window().with_winit_window(|w| w.set_minimized(false));
                        let _ = m.hide();
                    }
                }
                // live countdown in the tray tooltip (§4.11)
                {
                    let t = timer.borrow();
                    let state = if t.paused {
                        "paused"
                    } else if t.phase == Phase::Break {
                        "break"
                    } else {
                        "until next break"
                    };
                    let txt = format!("{} — {}", fmt(t.remaining), state);
                    drop(t);
                    if *last_tip.borrow() != txt {
                        *last_tip.borrow_mut() = txt.clone();
                        tip_handle.update(move |tr| tr.status = txt.clone());
                    }
                }
                while let Ok(action) = rx.try_recv() {
                    use tray::Action::*;
                    match action {
                        Open => {
                            if let Some(m) = main_w.upgrade() {
                                m.set_show_settings(false);
                                let _ = m.show();
                                m.window().with_winit_window(|win| {
                                    win.set_minimized(false);
                                    win.set_visible(true);
                                    win.focus_window();
                                    win.request_redraw();
                                });
                            }
                        }
                        Settings => {
                            if let Some(m) = main_w.upgrade() {
                                populate_settings(&m, &settings.borrow());
                                m.set_show_settings(true);
                                let _ = m.show();
                                m.window().with_winit_window(|win| {
                                    win.set_minimized(false);
                                    win.set_visible(true);
                                    win.focus_window();
                                });
                            }
                        }
                        Pause => {
                            let p = !timer.borrow().paused;
                            timer.borrow_mut().set_paused(p);
                            render();
                        }
                        Take => {
                            timer.borrow_mut().take_break(&settings.borrow());
                            sync_break();
                            render();
                        }
                        Skip => {
                            timer.borrow_mut().skip(&settings.borrow());
                            sync_break();
                            render();
                        }
                        ToggleWidget => {
                            let v = !widget_visible.get();
                            widget_visible.set(v);
                            if let Some(wd) = widget_w.upgrade() {
                                if v {
                                    let _ = wd.show();
                                } else {
                                    let _ = wd.hide();
                                }
                            }
                        }
                        Quit => {
                            let _ = slint::quit_event_loop();
                        }
                    }
                }
            },
        );
        (handle, dispatch)
    };

    // Dev: dump dashboard / widget / settings renders to /tmp for inspection.
    #[cfg(target_os = "linux")]
    if std::env::var("EYECARE_SNAP").is_ok() {
        let mw = main_win.as_weak();
        let ww = widget_win.as_weak();
        let bw = break_win.as_weak();
        let settings_c = settings.clone();
        slint::Timer::single_shot(Duration::from_millis(1500), move || {
            if let Some(w) = mw.upgrade() {
                if let Ok(b) = w.window().take_snapshot() {
                    save_png(&b, "/tmp/ec-dash.png");
                }
                w.set_app_version(updater::current_version().into());
                populate_settings(&w, &settings_c.borrow());
                w.set_show_settings(true);
            }
            if let Some(wd) = ww.upgrade() {
                if let Ok(b) = wd.window().take_snapshot() {
                    save_png(&b, "/tmp/ec-widget.png");
                }
            }
            let _ = &bw;
            let mw2 = mw.clone();
            slint::Timer::single_shot(Duration::from_millis(800), move || {
                if let Some(w) = mw2.upgrade() {
                    if let Ok(b) = w.window().take_snapshot() {
                        save_png(&b, "/tmp/ec-settings.png");
                    }
                }
            });
        });
    }

    // Tray-first: the dashboard stays hidden (lives in the tray) unless launched
    // with --show (the app-menu entry passes it). Autostart/boot has no flag, so
    // it comes up silently in the tray. Open later via the tray or the widget.
    if std::env::args().any(|a| a == "--show") {
        let _ = main_win.show();
    }
    slint::run_event_loop()
}
