// Local-only habit stats (no telemetry). Mirrors the Tauri build's stats.json:
// one entry per day with breaks taken/skipped, plus a derived streak.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Default, Clone, Serialize, Deserialize)]
struct Day {
    date: String,
    taken: u32,
    skipped: u32,
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Stats {
    days: Vec<Day>,
}

impl Stats {
    fn path() -> Option<PathBuf> {
        let mut p = dirs::config_dir()?;
        p.push("us.frankmaruf.eyecare-native");
        std::fs::create_dir_all(&p).ok()?;
        p.push("stats.json");
        Some(p)
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let (Some(p), Ok(j)) = (Self::path(), serde_json::to_string(self)) {
            let _ = std::fs::write(p, j);
        }
    }

    fn today() -> String {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    }

    /// Record a completed (`taken`) or skipped break for today.
    pub fn record(&mut self, taken: bool) {
        let t = Self::today();
        match self.days.iter_mut().find(|d| d.date == t) {
            Some(d) => {
                if taken {
                    d.taken += 1;
                } else {
                    d.skipped += 1;
                }
            }
            None => self.days.push(Day {
                date: t,
                taken: taken as u32,
                skipped: (!taken) as u32,
            }),
        }
        let n = self.days.len();
        if n > 60 {
            self.days.drain(0..n - 60); // keep ~2 months
        }
        self.save();
    }

    /// (streak in days, today's taken count, all-time taken count)
    pub fn summary(&self) -> (u32, u32, u32) {
        let total = self.days.iter().map(|d| d.taken).sum();
        let today = Self::today();
        let today_taken = self
            .days
            .iter()
            .find(|d| d.date == today)
            .map(|d| d.taken)
            .unwrap_or(0);

        let has = |d: &chrono::NaiveDate| {
            let ds = d.format("%Y-%m-%d").to_string();
            self.days.iter().any(|x| x.date == ds && x.taken > 0)
        };
        let mut cur = chrono::Local::now().date_naive();
        if !has(&cur) {
            // today not done yet — count the run ending yesterday
            match cur.pred_opt() {
                Some(p) => cur = p,
                None => return (0, today_taken, total),
            }
        }
        let mut streak = 0u32;
        while has(&cur) {
            streak += 1;
            match cur.pred_opt() {
                Some(p) => cur = p,
                None => break,
            }
        }
        (streak, today_taken, total)
    }
}
