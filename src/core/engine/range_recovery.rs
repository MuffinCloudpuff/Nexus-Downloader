//! Progress-aware recovery. Partial writes are persisted before reopening a range.
use std::time::Duration;

pub struct RangeWatchdog {
    idle: Duration,
    detect_slow: bool,
    last_progress: Duration,
    last_received: u64,
    window_start: Duration,
    window_received: u64,
    peak: u64,
}

impl RangeWatchdog {
    pub fn new(detect_slow: bool) -> Self {
        Self {
            idle: Duration::from_secs(if detect_slow { 4 } else { 15 }),
            detect_slow,
            last_progress: Duration::ZERO,
            last_received: 0,
            window_start: Duration::ZERO,
            window_received: 0,
            peak: 0,
        }
    }

    pub fn check(&mut self, elapsed: Duration, received: u64) -> Option<&'static str> {
        if received > self.last_received {
            self.last_progress = elapsed;
            self.last_received = received;
        }
        if elapsed.saturating_sub(self.last_progress) >= self.idle {
            return Some("idle");
        }
        let window = elapsed.saturating_sub(self.window_start);
        if self.detect_slow && window >= Duration::from_secs(2) {
            let speed = (received.saturating_sub(self.window_received) as f64
                / window.as_secs_f64()) as u64;
            let slow = received > 0
                && (speed < 128 * 1024 || (self.peak >= 512 * 1024 && speed < self.peak / 5));
            self.peak = self.peak.max(speed);
            self.window_start = elapsed;
            self.window_received = received;
            if slow {
                return Some("sustained_slow");
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconnects_on_stall_or_sustained_drop_not_normal_transfer() {
        let mut monitor = RangeWatchdog::new(true);
        assert_eq!(monitor.check(Duration::from_secs(2), 4 * 1024 * 1024), None);
        assert_eq!(
            monitor.check(Duration::from_secs(4), 4 * 1024 * 1024 + 64 * 1024),
            Some("sustained_slow")
        );
        assert_eq!(
            RangeWatchdog::new(true).check(Duration::from_secs(4), 0),
            Some("idle")
        );
        let mut normal = RangeWatchdog::new(false);
        assert_eq!(normal.check(Duration::from_secs(4), 1024), None);
        assert_eq!(normal.check(Duration::from_secs(19), 1024), Some("idle"));
        let mut healthy = RangeWatchdog::new(true);
        for second in [2, 4, 6] {
            assert_eq!(
                healthy.check(Duration::from_secs(second), second * 1024 * 1024),
                None
            );
        }
    }
}
