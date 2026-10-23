//! Progress reporting: every long operation reports a percentage and,
//! when a total is known, a simple ETA derived from elapsed time.
//! (Standing rule: Progress % + ETA whenever they can be estimated
//! responsibly; when no total exists, percent is `None`, never invented.)

use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Progress {
    started: Instant,
    done: u64,
    total: Option<u64>,
    label: String,
}

impl Progress {
    pub fn new(label: impl Into<String>, total: Option<u64>) -> Self {
        Self {
            started: Instant::now(),
            done: 0,
            total,
            label: label.into(),
        }
    }

    pub fn advance(&mut self, n: u64) {
        self.done += n;
    }

    pub fn done(&self) -> u64 {
        self.done
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Percentage complete, or `None` when no total is known.
    pub fn percent(&self) -> Option<f64> {
        let total = self.total?;
        if total == 0 {
            return Some(100.0);
        }
        Some((self.done as f64 / total as f64 * 100.0).min(100.0))
    }

    /// Estimated remaining time, or `None` when it cannot be estimated
    /// responsibly (no total, or nothing done yet).
    pub fn eta(&self) -> Option<Duration> {
        let total = self.total?;
        if self.done == 0 || self.done >= total {
            return if self.done >= total {
                Some(Duration::ZERO)
            } else {
                None
            };
        }
        let elapsed = self.started.elapsed().as_secs_f64();
        let per_item = elapsed / self.done as f64;
        Some(Duration::from_secs_f64(
            per_item * (total - self.done) as f64,
        ))
    }

    /// One-line human status, e.g. `Scanning files — 42% (ETA 0:13)`.
    pub fn status_line(&self) -> String {
        match (self.percent(), self.eta()) {
            (Some(p), Some(eta)) => {
                format!("{} — {:.0}% (ETA {})", self.label, p, fmt_duration(eta))
            }
            (Some(p), None) => format!("{} — {:.0}%", self.label, p),
            _ => format!("{} — {} items", self.label, self.done),
        }
    }
}

pub fn fmt_duration(d: Duration) -> String {
    let secs = d.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_and_eta_follow_done_total() {
        let mut p = Progress::new("Scanning", Some(200));
        assert_eq!(p.percent(), Some(0.0));
        assert!(p.eta().is_none()); // nothing done yet: no honest ETA
        p.advance(50);
        assert_eq!(p.percent(), Some(25.0));
        assert!(p.eta().is_some());
        p.advance(150);
        assert_eq!(p.percent(), Some(100.0));
        assert_eq!(p.eta(), Some(Duration::ZERO));
        assert!(p.status_line().contains("100%"));
    }

    #[test]
    fn unknown_total_never_invents_a_percent() {
        let mut p = Progress::new("Watching", None);
        p.advance(7);
        assert_eq!(p.percent(), None);
        assert_eq!(p.eta(), None);
        assert!(p.status_line().contains("7 items"));
    }
}
