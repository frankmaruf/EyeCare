// Timer engine, ported from `run_timer` / `to_break` / `to_working` in
// eyecare/src-tauri/src/lib.rs. Pure state machine: `tick()` advances one
// second and returns the side-effects to perform (phase event + any wellbeing
// nudges), so the UI/shell layer stays thin and the logic is testable.

use crate::settings::Settings;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Working,
    Break,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    None,
    Prewarn,
    BreakStart,
    BreakEnd,
}

/// Which periodic nudges fired this tick.
#[derive(Clone, Copy, Default, Debug)]
pub struct Nudges {
    pub blink: bool,
    pub hydration: bool,
    pub posture: bool,
    pub eyedrops: bool,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct TickResult {
    pub event: Event,
    pub nudges: Nudges,
}

impl Default for Event {
    fn default() -> Self {
        Event::None
    }
}

/// Count one enabled nudge down; when it reaches zero it fires and re-arms.
fn tick_nudge(remaining: &mut u64, enabled: bool, interval: u64) -> bool {
    if !enabled {
        return false;
    }
    if *remaining > 0 {
        *remaining -= 1;
    }
    if *remaining == 0 {
        *remaining = interval.max(1);
        return true;
    }
    false
}

pub struct Timer {
    pub phase: Phase,
    pub remaining: u64,
    pub total: u64,
    pub paused: bool,
    pub warned: bool,
    pub breaks_done: u32,
    pub postpones_used: u32,
    pub is_long: bool,
    blink_remaining: u64,
    hydration_remaining: u64,
    posture_remaining: u64,
    eyedrops_remaining: u64,
}

impl Timer {
    pub fn new(s: &Settings) -> Self {
        Self {
            phase: Phase::Working,
            remaining: s.work_interval_secs,
            total: s.work_interval_secs,
            paused: false,
            warned: false,
            breaks_done: 0,
            postpones_used: 0,
            is_long: false,
            blink_remaining: s.blink_interval_secs.max(1),
            hydration_remaining: s.hydration_interval_secs.max(1),
            posture_remaining: s.posture_interval_secs.max(1),
            eyedrops_remaining: s.eyedrops_interval_secs.max(1),
        }
    }

    fn to_working(&mut self, s: &Settings) {
        self.phase = Phase::Working;
        self.total = s.work_interval_secs;
        self.remaining = s.work_interval_secs;
        self.warned = false;
        self.postpones_used = 0;
        self.is_long = false;
    }

    fn to_break(&mut self, s: &Settings) {
        self.breaks_done += 1;
        self.is_long =
            s.long_break_enabled && s.long_break_every > 0 && self.breaks_done % s.long_break_every == 0;
        self.phase = Phase::Break;
        self.total = if self.is_long {
            s.long_break_secs
        } else {
            s.break_length_secs
        };
        self.remaining = self.total;
        self.warned = false;
    }

    /// Advance one second; returns the phase event + any nudges that fired.
    pub fn tick(&mut self, s: &Settings) -> TickResult {
        if self.paused {
            return TickResult::default();
        }
        if self.remaining > 0 {
            self.remaining -= 1;
        }

        // Periodic nudges only while working.
        let mut nudges = Nudges::default();
        if self.phase == Phase::Working {
            nudges.blink = tick_nudge(&mut self.blink_remaining, s.blink_enabled, s.blink_interval_secs);
            nudges.hydration =
                tick_nudge(&mut self.hydration_remaining, s.hydration_enabled, s.hydration_interval_secs);
            nudges.posture =
                tick_nudge(&mut self.posture_remaining, s.posture_enabled, s.posture_interval_secs);
            nudges.eyedrops =
                tick_nudge(&mut self.eyedrops_remaining, s.eyedrops_enabled, s.eyedrops_interval_secs);
        }

        let event = if self.phase == Phase::Working
            && !self.warned
            && s.pre_break_warning_secs > 0
            && self.remaining == s.pre_break_warning_secs
        {
            self.warned = true;
            Event::Prewarn
        } else if self.remaining == 0 {
            match self.phase {
                Phase::Working => {
                    self.to_break(s);
                    Event::BreakStart
                }
                Phase::Break => {
                    self.to_working(s);
                    Event::BreakEnd
                }
            }
        } else {
            Event::None
        };

        TickResult { event, nudges }
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Re-apply settings after an edit: cap the working countdown and each nudge
    /// to its (possibly shortened) interval so changes take effect now, not next
    /// cycle. Mirrors set_settings() in lib.rs.
    pub fn apply_settings(&mut self, s: &Settings) {
        if self.phase == Phase::Working {
            self.total = s.work_interval_secs;
            self.remaining = self.remaining.min(s.work_interval_secs);
        }
        self.blink_remaining = self.blink_remaining.clamp(1, s.blink_interval_secs.max(1));
        self.hydration_remaining = self.hydration_remaining.clamp(1, s.hydration_interval_secs.max(1));
        self.posture_remaining = self.posture_remaining.clamp(1, s.posture_interval_secs.max(1));
        self.eyedrops_remaining = self.eyedrops_remaining.clamp(1, s.eyedrops_interval_secs.max(1));
    }

    /// Start a break immediately (dashboard "Take a break now").
    pub fn take_break(&mut self, s: &Settings) -> Event {
        if self.phase == Phase::Working {
            self.to_break(s);
            Event::BreakStart
        } else {
            Event::None
        }
    }

    /// Skip: end a break early, or restart the work interval.
    pub fn skip(&mut self, s: &Settings) -> Event {
        let was_break = self.phase == Phase::Break;
        self.to_working(s);
        if was_break {
            Event::BreakEnd
        } else {
            Event::None
        }
    }

    /// Postpone the break by one snooze interval. Returns false if the
    /// max-postpones cap is reached.
    pub fn postpone(&mut self, s: &Settings) -> bool {
        if s.max_postpones > 0 && self.postpones_used >= s.max_postpones {
            return false;
        }
        self.postpones_used += 1;
        if self.phase == Phase::Break {
            self.phase = Phase::Working;
            self.total = s.snooze_secs;
            self.remaining = s.snooze_secs;
            self.warned = false;
            self.is_long = false;
        } else {
            self.remaining = self.remaining.saturating_add(s.snooze_secs);
        }
        true
    }

    /// Remaining fraction for the ring (clamped off a degenerate full circle).
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        (self.remaining as f32 / self.total as f32).clamp(0.0, 0.999)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            work_interval_secs: 5,
            break_length_secs: 3,
            pre_break_warning_secs: 2,
            snooze_secs: 4,
            max_postpones: 1,
            long_break_enabled: true,
            long_break_every: 2,
            long_break_secs: 7,
            ..Settings::default()
        }
    }

    /// Advance `n` seconds, returning the last non-None phase event seen.
    fn run(t: &mut Timer, s: &Settings, n: u64) -> Event {
        let mut last = Event::None;
        for _ in 0..n {
            let e = t.tick(s).event;
            if e != Event::None {
                last = e;
            }
        }
        last
    }

    #[test]
    fn working_to_break_to_working() {
        let s = settings();
        let mut t = Timer::new(&s);
        assert_eq!(t.phase, Phase::Working);
        assert_eq!(run(&mut t, &s, 3), Event::Prewarn);
        assert_eq!(run(&mut t, &s, 2), Event::BreakStart);
        assert_eq!(t.phase, Phase::Break);
        assert_eq!(t.remaining, s.break_length_secs);
        assert_eq!(run(&mut t, &s, 3), Event::BreakEnd);
        assert_eq!(t.phase, Phase::Working);
    }

    #[test]
    fn every_second_break_is_long() {
        let s = settings();
        let mut t = Timer::new(&s);
        t.take_break(&s);
        assert!(!t.is_long);
        t.skip(&s);
        t.take_break(&s);
        assert!(t.is_long);
        assert_eq!(t.total, s.long_break_secs);
    }

    #[test]
    fn pause_freezes_countdown() {
        let s = settings();
        let mut t = Timer::new(&s);
        t.set_paused(true);
        let before = t.remaining;
        run(&mut t, &s, 10);
        assert_eq!(t.remaining, before);
    }

    #[test]
    fn postpone_respects_cap() {
        let s = settings();
        let mut t = Timer::new(&s);
        t.take_break(&s);
        assert!(t.postpone(&s));
        t.take_break(&s);
        assert!(!t.postpone(&s));
    }

    #[test]
    fn blink_nudge_fires_on_interval() {
        let s = Settings {
            blink_enabled: true,
            blink_interval_secs: 3,
            ..settings()
        };
        let mut t = Timer::new(&s);
        assert!(!t.tick(&s).nudges.blink); // 2 left
        assert!(!t.tick(&s).nudges.blink); // 1 left
        assert!(t.tick(&s).nudges.blink); // fires + re-arms
    }
}
