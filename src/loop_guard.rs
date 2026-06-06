//! Detects self-inflicted watch loops.
//!
//! On Linux/inotify a step that rewrites watched files (a formatter, a codegen
//! step, anything that bumps an mtime) is re-detected by the watcher, which
//! restarts the pipeline, which rewrites the files again — a tight loop.
//! macOS/FSEvents tends to coalesce or suppress these, so the bug is
//! Linux-only. The guard counts *restart events* (one debounced file-change
//! batch = one event, regardless of how many files it carried) within a
//! sliding window and trips once they cross a threshold.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Sliding-window counter over recent pipeline restarts.
pub struct LoopGuard {
    starts: VecDeque<Instant>,
    window: Duration,
    threshold: usize,
}

impl LoopGuard {
    pub fn new(window: Duration, threshold: usize) -> Self {
        Self {
            starts: VecDeque::new(),
            window,
            threshold,
        }
    }

    /// Records a file-change-triggered restart at `now`, evicts entries older
    /// than `window`, and returns `true` when the number of restarts inside
    /// the window has reached `threshold` — i.e. a loop is suspected.
    pub fn record(&mut self, now: Instant) -> bool {
        self.starts.push_back(now);
        while let Some(&front) = self.starts.front() {
            if now.duration_since(front) > self.window {
                self.starts.pop_front();
            } else {
                break;
            }
        }
        self.starts.len() >= self.threshold
    }

    /// Clears history after a warning so the next trip needs a fresh burst
    /// rather than firing on every subsequent restart.
    pub fn reset(&mut self) {
        self.starts.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> LoopGuard {
        LoopGuard::new(Duration::from_secs(10), 5)
    }

    #[test]
    fn does_not_trip_below_threshold() {
        let mut g = guard();
        let t0 = Instant::now();
        for i in 0..4 {
            assert!(!g.record(t0 + Duration::from_millis(i * 100)));
        }
    }

    #[test]
    fn trips_at_threshold_within_window() {
        let mut g = guard();
        let t0 = Instant::now();
        let mut tripped = false;
        for i in 0..5 {
            tripped = g.record(t0 + Duration::from_millis(i * 100));
        }
        assert!(tripped, "expected the 5th restart in 10s to trip the guard");
    }

    #[test]
    fn does_not_trip_when_spread_beyond_window() {
        let mut g = guard();
        let t0 = Instant::now();
        // One restart every 3s: never 5 inside a 10s window.
        for i in 0..10 {
            assert!(!g.record(t0 + Duration::from_secs(i * 3)));
        }
    }

    #[test]
    fn reset_clears_history() {
        let mut g = guard();
        let t0 = Instant::now();
        for i in 0..4 {
            g.record(t0 + Duration::from_millis(i * 100));
        }
        g.reset();
        // After reset a single restart must not trip even though we were at 4.
        assert!(!g.record(t0 + Duration::from_millis(500)));
    }

    #[test]
    fn evicts_stale_entries_before_counting() {
        let mut g = guard();
        let t0 = Instant::now();
        // Four old restarts, then a long gap, then four fresh ones: the old
        // four fall out of the window, so we stay under threshold.
        for i in 0..4 {
            assert!(!g.record(t0 + Duration::from_millis(i * 100)));
        }
        let later = t0 + Duration::from_secs(30);
        for i in 0..4 {
            assert!(!g.record(later + Duration::from_millis(i * 100)));
        }
    }
}
