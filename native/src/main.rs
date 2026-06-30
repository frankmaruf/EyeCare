// EyeCare native (Rust + Slint). Dashboard + break overlay + settings + floating
// widget + wellbeing-nudge notifications, driven by the ported timer engine with
// persisted settings. Tray, work-hours/idle/DND suppression, global shortcuts,
// autostart, updater and packaging land in later increments. Mirrors the design
// of the Tauri build (eyecare/src).

mod settings;
mod timer;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_winit::WinitWindowAccessor;
use settings::Settings;
use timer::{Event, Phase, Timer};

slint::include_modules!();

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

/// Parse "#rrggbb" into a Slint color (falls back to the default accent).
fn parse_color(hex: &str) -> slint::Color {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x4cc6c0);
    slint::Color::from_rgb_u8((n >> 16) as u8, (n >> 8) as u8, n as u8)
}

fn populate_settings(w: &SettingsWindow, s: &Settings) {
    w.set_work_min((s.work_interval_secs / 60) as i32);
    w.set_break_sec(s.break_length_secs as i32);
    w.set_prewarn_sec(s.pre_break_warning_secs as i32);
    w.set_escalation_idx(to_idx(&ESCALATIONS, &s.escalation, 1));
    w.set_sound_enabled(s.sound_enabled);
    w.set_accent_idx(to_idx(&ACCENTS, &s.accent, 0));
    w.set_reduce_motion(s.reduce_motion);
    w.set_high_contrast(s.high_contrast);
    w.set_snooze_min((s.snooze_secs / 60) as i32);
    w.set_max_postpones(s.max_postpones as i32);
    w.set_long_enabled(s.long_break_enabled);
    w.set_long_every(s.long_break_every as i32);
    w.set_long_min((s.long_break_secs / 60) as i32);
    w.set_tips_enabled(s.tips_enabled);
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
    w.set_long_hint(long_hint(s).into());
}

fn read_settings(w: &SettingsWindow, base: &Settings) -> Settings {
    Settings {
        work_interval_secs: w.get_work_min().max(1) as u64 * 60,
        break_length_secs: w.get_break_sec().max(5) as u64,
        pre_break_warning_secs: w.get_prewarn_sec().max(0) as u64,
        escalation: from_idx(&ESCALATIONS, w.get_escalation_idx(), "standard"),
        sound_enabled: w.get_sound_enabled(),
        accent: from_idx(&ACCENTS, w.get_accent_idx(), "#4cc6c0"),
        reduce_motion: w.get_reduce_motion(),
        high_contrast: w.get_high_contrast(),
        snooze_secs: w.get_snooze_min().max(1) as u64 * 60,
        max_postpones: w.get_max_postpones().max(0) as u32,
        long_break_enabled: w.get_long_enabled(),
        long_break_every: w.get_long_every().max(1) as u32,
        long_break_secs: w.get_long_min().max(1) as u64 * 60,
        tips_enabled: w.get_tips_enabled(),
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
        widget_width: w.get_widget_w().clamp(120, 480) as u32,
        widget_height: w.get_widget_h().clamp(120, 480) as u32,
        // preserve fields not shown in the settings UI (widget position, …)
        ..base.clone()
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let settings = Rc::new(RefCell::new(Settings::load()));
    let timer = Rc::new(RefCell::new(Timer::new(&settings.borrow())));

    let main_win = MainWindow::new()?;
    let break_win = BreakWindow::new()?;
    let settings_win = SettingsWindow::new()?;
    let widget_win = WidgetWindow::new()?;

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
        set_theme!(settings_win, c, s.reduce_motion, s.high_contrast);
        set_theme!(widget_win, c, s.reduce_motion, s.high_contrast);
    }

    // Single source of truth → every window. One closure keeps them in sync.
    let render = {
        let main_w = main_win.as_weak();
        let break_w = break_win.as_weak();
        let widget_w = widget_win.as_weak();
        let timer = timer.clone();
        let settings = settings.clone();
        move || {
            let t = timer.borrow();
            let time = fmt(t.remaining);
            if let Some(w) = main_w.upgrade() {
                w.set_progress(t.fraction());
                w.set_time_text(time.clone());
                let (tag, label) = match (t.paused, t.phase) {
                    (true, _) => ("paused", "paused"),
                    (_, Phase::Break) => ("break", "break time left"),
                    (_, Phase::Working) => ("working", "until next break"),
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
                w.set_opacity_frac((s.widget_opacity as f32 / 100.0).clamp(0.2, 1.0));
            }
            if let Some(w) = break_w.upgrade() {
                w.set_time_text(time);
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
            // "gentle" intensity uses a notification only — no window grab.
            if t.phase == Phase::Break && s.escalation != "gentle" {
                if s.tips_enabled {
                    w.set_tip_text(format!("💡 {}", TIPS[t.breaks_done as usize % TIPS.len()]).into());
                } else {
                    w.set_tip_text("".into());
                }
                w.set_can_postpone(t.postpones_used < s.max_postpones);
                // "forced" goes fullscreen + always-on-top; "standard" is a window.
                let forced = s.escalation == "forced";
                w.window().with_winit_window(|win| {
                    use i_slint_backend_winit::winit::window::{Fullscreen, WindowLevel};
                    win.set_fullscreen(if forced {
                        Some(Fullscreen::Borderless(None))
                    } else {
                        None
                    });
                    win.set_window_level(if forced {
                        WindowLevel::AlwaysOnTop
                    } else {
                        WindowLevel::Normal
                    });
                });
                let _ = w.show();
            } else {
                let _ = w.hide();
            }
        }
    };

    render();
    let _ = break_win.hide();
    let _ = widget_win.show();

    // 1-second tick.
    let ticker = slint::Timer::default();
    {
        let timer = timer.clone();
        let settings = settings.clone();
        let render = render.clone();
        let sync_break = sync_break.clone();
        ticker.start(slint::TimerMode::Repeated, Duration::from_secs(1), move || {
            let res = timer.borrow_mut().tick(&settings.borrow());
            match res.event {
                Event::Prewarn => notify("Eye break soon", "A break is coming up — finish your thought."),
                Event::BreakStart => {
                    notify("Time for an eye break", "Look ~20 feet (6 m) away and relax your eyes.");
                    sync_break();
                }
                Event::BreakEnd => sync_break(),
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
    action!(break_win.on_skip, |t, s| t.skip(&s));
    action!(break_win.on_postpone, |t, s| t.postpone(&s));
    action!(widget_win.on_pause, |t, _s| {
        let p = !t.paused;
        t.set_paused(p)
    });
    action!(widget_win.on_take_break, |t, s| t.take_break(&s));
    action!(widget_win.on_skip, |t, s| t.skip(&s));

    // Settings: open (populate) / save (persist + apply) / back.
    {
        let settings_w = settings_win.as_weak();
        let settings = settings.clone();
        main_win.on_open_settings(move || {
            if let Some(w) = settings_w.upgrade() {
                populate_settings(&w, &settings.borrow());
                let _ = w.show();
                // bring it to the front + focus (a hidden window can otherwise
                // re-appear behind the dashboard, looking like nothing happened)
                w.window().with_winit_window(|win| {
                    win.set_visible(true);
                    win.focus_window();
                });
            }
        });
    }
    {
        let settings_w = settings_win.as_weak();
        let widget_w = widget_win.as_weak();
        let main_w = main_win.as_weak();
        let break_w = break_win.as_weak();
        let settings = settings.clone();
        let timer = timer.clone();
        let render = render.clone();
        settings_win.on_save(move || {
            let Some(w) = settings_w.upgrade() else { return };
            let next = read_settings(&w, &settings.borrow());
            timer.borrow_mut().apply_settings(&next);
            next.save();
            *settings.borrow_mut() = next;
            let s = settings.borrow();
            let c = parse_color(&s.accent);
            // live-apply appearance to every window, and the widget size
            set_theme!(w, c, s.reduce_motion, s.high_contrast);
            if let Some(m) = main_w.upgrade() {
                set_theme!(m, c, s.reduce_motion, s.high_contrast);
            }
            if let Some(b) = break_w.upgrade() {
                set_theme!(b, c, s.reduce_motion, s.high_contrast);
            }
            if let Some(wd) = widget_w.upgrade() {
                set_theme!(wd, c, s.reduce_motion, s.high_contrast);
                wd.window()
                    .set_size(slint::LogicalSize::new(s.widget_width as f32, s.widget_height as f32));
            }
            w.set_long_hint(long_hint(&s).into());
            drop(s);
            let _ = w.hide();
            render();
        });
    }
    {
        let settings_w = settings_win.as_weak();
        settings_win.on_back(move || {
            if let Some(w) = settings_w.upgrade() {
                let _ = w.hide();
            }
        });
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
                w.window().with_winit_window(|win| {
                    let _ = win.drag_window();
                });
            }
        });
    }
    {
        let widget_w = widget_win.as_weak();
        widget_win.on_start_resize(move || {
            if let Some(w) = widget_w.upgrade() {
                use i_slint_backend_winit::winit::window::ResizeDirection;
                w.window().with_winit_window(|win| {
                    let _ = win.drag_resize_window(ResizeDirection::SouthEast);
                });
            }
        });
    }
    // persist widget geometry (logical px) whenever it changes
    let geom_timer = slint::Timer::default();
    {
        let widget_w = widget_win.as_weak();
        let settings = settings.clone();
        geom_timer.start(slint::TimerMode::Repeated, Duration::from_secs(2), move || {
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
            let mut s = settings.borrow_mut();
            if s.widget_width != lw || s.widget_height != lh || s.widget_x != Some(lx) || s.widget_y != Some(ly) {
                s.widget_width = lw.clamp(settings::WIDGET_MIN, settings::WIDGET_MAX);
                s.widget_height = lh.clamp(settings::WIDGET_MIN, settings::WIDGET_MAX);
                s.widget_x = Some(lx);
                s.widget_y = Some(ly);
                s.save();
            }
        });
    }

    main_win.run()
}
