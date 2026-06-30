// EyeCare native (Rust + Slint). Dashboard + break overlay + settings + floating
// widget, driven by the ported timer engine with persisted settings. Tray,
// notifications, nudges, work-hours/idle/DND, shortcuts, autostart, updater and
// packaging land in later increments. Mirrors the Tauri build (eyecare/src).

mod settings;
mod timer;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

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

fn populate_settings(w: &SettingsWindow, s: &Settings) {
    w.set_work_min((s.work_interval_secs / 60) as i32);
    w.set_break_sec(s.break_length_secs as i32);
    w.set_prewarn_sec(s.pre_break_warning_secs as i32);
    w.set_snooze_min((s.snooze_secs / 60) as i32);
    w.set_max_postpones(s.max_postpones as i32);
    w.set_long_enabled(s.long_break_enabled);
    w.set_long_every(s.long_break_every as i32);
    w.set_long_min((s.long_break_secs / 60) as i32);
    w.set_tips_enabled(s.tips_enabled);
    w.set_long_hint(long_hint(s).into());
}

fn read_settings(w: &SettingsWindow) -> Settings {
    Settings {
        work_interval_secs: w.get_work_min().max(1) as u64 * 60,
        break_length_secs: w.get_break_sec().max(5) as u64,
        pre_break_warning_secs: w.get_prewarn_sec().max(0) as u64,
        snooze_secs: w.get_snooze_min().max(1) as u64 * 60,
        max_postpones: w.get_max_postpones().max(0) as u32,
        long_break_enabled: w.get_long_enabled(),
        long_break_every: w.get_long_every().max(1) as u32,
        long_break_secs: w.get_long_min().max(1) as u64 * 60,
        tips_enabled: w.get_tips_enabled(),
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let settings = Rc::new(RefCell::new(Settings::load()));
    let timer = Rc::new(RefCell::new(Timer::new(&settings.borrow())));

    let main_win = MainWindow::new()?;
    let break_win = BreakWindow::new()?;
    let settings_win = SettingsWindow::new()?;
    let widget_win = WidgetWindow::new()?;

    // Single source of truth → every window. One closure keeps them in sync.
    let render = {
        let main_w = main_win.as_weak();
        let break_w = break_win.as_weak();
        let widget_w = widget_win.as_weak();
        let timer = timer.clone();
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
            if t.phase == Phase::Break {
                if settings.borrow().tips_enabled {
                    w.set_tip_text(format!("💡 {}", TIPS[t.breaks_done as usize % TIPS.len()]).into());
                } else {
                    w.set_tip_text("".into());
                }
                w.set_can_postpone(t.postpones_used < settings.borrow().max_postpones);
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
            let ev = timer.borrow_mut().tick(&settings.borrow());
            if matches!(ev, Event::BreakStart | Event::BreakEnd) {
                sync_break();
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
            }
        });
    }
    {
        let settings_w = settings_win.as_weak();
        let settings = settings.clone();
        let timer = timer.clone();
        let render = render.clone();
        settings_win.on_save(move || {
            let Some(w) = settings_w.upgrade() else { return };
            let next = read_settings(&w);
            timer.borrow_mut().apply_settings(&next);
            next.save();
            *settings.borrow_mut() = next;
            w.set_long_hint(long_hint(&settings.borrow()).into());
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

    // Widget: bring the dashboard back to the front.
    {
        let main_w = main_win.as_weak();
        widget_win.on_restore(move || {
            if let Some(w) = main_w.upgrade() {
                let _ = w.show();
                // focus is best-effort across backends
            }
        });
    }

    main_win.run()
}
