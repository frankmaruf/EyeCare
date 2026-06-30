// Timer engine, ported from `run_timer` / `to_break` / `to_working` in
// eyecare/src-tauri/src/lib.rs. Pure state machine: `tick()` advances one
// second and returns the side-effect to perform, so the UI/shell layer stays
// thin and the logic is testable.

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

pub struct Timer {
    pub phase: Phase,
    pub remaining: u64,
    pub total: u64,
    pub paused: bool,
    pub warned: bool,
    pub breaks_done: u32,
    pub postpones_used: u32,
    pub is_long: bool,
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

    /// Advance one second; returns the resulting event (if any).
    pub fn tick(&mut self, s: &Settings) -> Event {
        if self.paused {
            return Event::None;
        }
        if self.remaining > 0 {
            self.remaining -= 1;
        }
        if self.phase == Phase::Working
            && !self.warned
            && s.pre_break_warning_secs > 0
            && self.remaining == s.pre_break_warning_secs
        {
            self.warned = true;
            return Event::Prewarn;
        }
        if self.remaining == 0 {
            return match self.phase {
                Phase::Working => {
                    self.to_break(s);
                    Event::BreakStart
                }
                Phase::Break => {
                    self.to_working(s);
                    Event::BreakEnd
                }
            };
        }
        Event::None
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Re-apply settings after an edit: cap the current working countdown to the
    /// (possibly shortened) interval so a change takes effect now, not next
    /// cycle. Mirrors set_settings() in lib.rs.
    pub fn apply_settings(&mut self, s: &Settings) {
        if self.phase == Phase::Working {
            self.total = s.work_interval_secs;
            if self.remaining > s.work_interval_secs {
                self.remaining = s.work_interval_secs;
            }
        }
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
            // Defer the break: go back to a short (snooze-length) work countdown.
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
            tips_enabled: true,
        }
    }

    /// Advance `n` seconds, returning the last non-None event seen.
    fn run(t: &mut Timer, s: &Settings, n: u64) -> Event {
        let mut last = Event::None;
        for _ in 0..n {
            let e = t.tick(s);
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
        // prewarn fires when remaining hits pre_break_warning_secs (after 3s)
        assert_eq!(run(&mut t, &s, 3), Event::Prewarn);
        // break starts at 0 (5s total)
        assert_eq!(run(&mut t, &s, 2), Event::BreakStart);
        assert_eq!(t.phase, Phase::Break);
        assert_eq!(t.remaining, s.break_length_secs);
        // break ends after break_length_secs
        assert_eq!(run(&mut t, &s, 3), Event::BreakEnd);
        assert_eq!(t.phase, Phase::Working);
    }

    #[test]
    fn every_second_break_is_long() {
        let s = settings();
        let mut t = Timer::new(&s);
        t.take_break(&s); // break #1 -> short
        assert!(!t.is_long);
        t.skip(&s); // back to work
        t.take_break(&s); // break #2 -> long
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
        assert!(t.postpone(&s)); // 1st ok
        t.take_break(&s);
        assert!(!t.postpone(&s)); // cap = 1 -> denied
    }
}
