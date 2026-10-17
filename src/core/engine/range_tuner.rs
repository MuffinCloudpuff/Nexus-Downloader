use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

pub const DEFAULT_INITIAL_TARGET: usize = 8;
const CONTROLLER_TICK_MS: u64 = 500;
const PHASE_SAMPLE_SECONDS: u64 = 2;
const PHASE_MIN_BYTES: u64 = 512 * 1024;
const THROUGHPUT_GAIN_RATIO: f64 = 1.08;
const THROUGHPUT_DROP_RATIO: f64 = 0.88;
const HISTORY_DEGRADATION_RATIO: f64 = 0.55;
const RAMP_TARGETS: [usize; 6] = [8, 16, 32, 64, 128, 256];

#[derive(Debug, Clone, Copy)]
pub struct RangeTunerHistory {
    pub last_effective_target: usize,
    pub last_effective_speed_bytes_per_sec: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct RangeTunerOutcome {
    pub best_target: usize,
    pub best_speed_bytes_per_sec: u64,
    pub initial_target: usize,
}

#[derive(Clone)]
pub struct RangeTunerRuntime {
    limiter: Arc<AdaptiveChunkLimiter>,
    best_target: Arc<AtomicUsize>,
    best_speed: Arc<AtomicU64>,
    historical_speed: Option<u64>,
    initial_target: usize,
    max_target: usize,
}

impl RangeTunerRuntime {
    pub fn new(max_target: usize, history: Option<RangeTunerHistory>) -> Self {
        let max_target = max_target.max(1);
        let initial_target = initial_target(max_target, history);
        Self {
            limiter: Arc::new(AdaptiveChunkLimiter::new(initial_target, max_target)),
            best_target: Arc::new(AtomicUsize::new(initial_target)),
            best_speed: Arc::new(AtomicU64::new(0)),
            historical_speed: history.map(|record| record.last_effective_speed_bytes_per_sec),
            initial_target,
            max_target,
        }
    }

    pub fn target(&self) -> usize {
        self.limiter.target()
    }

    pub fn active(&self) -> usize {
        self.limiter.active()
    }

    pub async fn acquire(
        &self,
        token: &tokio_util::sync::CancellationToken,
    ) -> Result<AdaptiveChunkPermit> {
        self.limiter.acquire(token).await
    }

    pub fn outcome(&self) -> RangeTunerOutcome {
        RangeTunerOutcome {
            best_target: self.best_target.load(Ordering::Relaxed),
            best_speed_bytes_per_sec: self.best_speed.load(Ordering::Relaxed),
            initial_target: self.initial_target,
        }
    }
}

pub fn initial_target(max_target: usize, history: Option<RangeTunerHistory>) -> usize {
    let max_target = max_target.max(1);
    history
        .map(|record| record.last_effective_target)
        .unwrap_or(DEFAULT_INITIAL_TARGET)
        .clamp(1, max_target)
}

pub fn next_probe_target(current: usize, max_target: usize) -> usize {
    if current >= max_target {
        return current;
    }

    RAMP_TARGETS
        .iter()
        .copied()
        .find(|target| *target > current)
        .unwrap_or(max_target)
        .min(max_target)
        .max(current)
}

pub fn previous_probe_target(current: usize) -> usize {
    RAMP_TARGETS
        .iter()
        .rev()
        .copied()
        .find(|target| *target < current)
        .unwrap_or(1)
}

pub fn choose_connection_target(
    current_target: usize,
    max_target: usize,
    previous_speed: u64,
    best_speed: u64,
    best_target: usize,
    current_speed: u64,
    historical_speed: Option<u64>,
) -> usize {
    if current_target >= max_target
        && historical_speed.is_none_or(|speed| current_speed >= degraded_speed_threshold(speed))
    {
        return current_target;
    }

    if current_speed == 0 {
        return current_target;
    }

    if let Some(speed) = historical_speed
        && current_speed < degraded_speed_threshold(speed)
        && current_target > DEFAULT_INITIAL_TARGET
    {
        return previous_probe_target(current_target).max(1);
    }

    if previous_speed == 0 || best_speed == 0 {
        return next_probe_target(current_target, max_target);
    }

    let current_speed_f = current_speed as f64;
    let previous_speed_f = previous_speed as f64;
    let best_speed_f = best_speed as f64;

    if current_speed_f >= previous_speed_f * THROUGHPUT_GAIN_RATIO
        || current_speed_f >= best_speed_f * THROUGHPUT_GAIN_RATIO
    {
        return next_probe_target(current_target, max_target);
    }

    if current_speed_f < best_speed_f * THROUGHPUT_DROP_RATIO && best_target < current_target {
        return best_target.max(1);
    }

    current_target
}

fn degraded_speed_threshold(speed: u64) -> u64 {
    (speed as f64 * HISTORY_DEGRADATION_RATIO) as u64
}

pub struct AdaptiveChunkLimiter {
    target: AtomicUsize,
    active: AtomicUsize,
    max_target: usize,
    notify: Notify,
}

impl AdaptiveChunkLimiter {
    fn new(initial_target: usize, max_target: usize) -> Self {
        let max_target = max_target.max(1);
        let initial_target = initial_target.clamp(1, max_target);

        Self {
            target: AtomicUsize::new(initial_target),
            active: AtomicUsize::new(0),
            max_target,
            notify: Notify::new(),
        }
    }

    fn target(&self) -> usize {
        self.target.load(Ordering::Relaxed)
    }

    fn active(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }

    fn set_target(&self, target: usize) -> usize {
        let target = target.clamp(1, self.max_target);
        let previous = self.target.swap(target, Ordering::Relaxed);
        if previous != target {
            self.notify.notify_waiters();
        }
        previous
    }

    async fn acquire(
        self: &Arc<Self>,
        token: &tokio_util::sync::CancellationToken,
    ) -> Result<AdaptiveChunkPermit> {
        loop {
            if token.is_cancelled() {
                anyhow::bail!("任务已取消");
            }

            let active = self.active.load(Ordering::Relaxed);
            let target = self.target();
            if active < target {
                if self
                    .active
                    .compare_exchange(active, active + 1, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
                {
                    return Ok(AdaptiveChunkPermit {
                        limiter: self.clone(),
                    });
                }
                continue;
            }

            tokio::select! {
                _ = token.cancelled() => anyhow::bail!("任务已取消"),
                _ = self.notify.notified() => {}
            }
        }
    }
}

pub struct AdaptiveChunkPermit {
    limiter: Arc<AdaptiveChunkLimiter>,
}

impl Drop for AdaptiveChunkPermit {
    fn drop(&mut self) {
        self.limiter.active.fetch_sub(1, Ordering::AcqRel);
        self.limiter.notify.notify_waiters();
    }
}

#[derive(Debug, Clone)]
struct ProbePhase {
    target: usize,
    requested_at: Instant,
    reached_at: Option<Instant>,
    reached_bytes: u64,
}

impl ProbePhase {
    fn new(target: usize, received: u64) -> Self {
        Self {
            target,
            requested_at: Instant::now(),
            reached_at: None,
            reached_bytes: received,
        }
    }

    fn reset(&mut self, target: usize, received: u64) {
        *self = Self::new(target, received);
    }

    fn mark_reached(&mut self, received: u64) {
        if self.reached_at.is_none() {
            self.reached_at = Some(Instant::now());
            self.reached_bytes = received;
        }
    }

    fn measured_speed(&self, received: u64) -> Option<u64> {
        let reached_at = self.reached_at?;
        let elapsed = reached_at.elapsed();
        if elapsed < Duration::from_secs(PHASE_SAMPLE_SECONDS) {
            return None;
        }

        let bytes_delta = received.saturating_sub(self.reached_bytes);
        if bytes_delta < PHASE_MIN_BYTES {
            return None;
        }

        let elapsed_secs = elapsed.as_secs_f64();
        if elapsed_secs <= 0.0 {
            None
        } else {
            Some((bytes_delta as f64 / elapsed_secs) as u64)
        }
    }
}

pub async fn run_controller(
    tuner: RangeTunerRuntime,
    downloaded_bytes: Arc<AtomicU64>,
    total_size: u64,
    task_id: String,
    url_host: String,
    token: tokio_util::sync::CancellationToken,
) {
    let tick_interval = tokio::time::Duration::from_millis(CONTROLLER_TICK_MS);
    let initial_received = downloaded_bytes.load(Ordering::Relaxed).min(total_size);
    let mut previous_phase_speed = 0_u64;
    let mut phase = ProbePhase::new(tuner.target(), initial_received);

    loop {
        tokio::select! {
            _ = token.cancelled() => break,
            _ = tokio::time::sleep(tick_interval) => {}
        }

        let received = downloaded_bytes.load(Ordering::Relaxed).min(total_size);
        if received >= total_size {
            break;
        }

        let current_target = tuner.target();
        let active_connections = tuner.active();
        if phase.target != current_target {
            phase.reset(current_target, received);
        }

        if active_connections >= current_target {
            let was_unreached = phase.reached_at.is_none();
            phase.mark_reached(received);
            if was_unreached {
                tracing::info!(
                    task_id = %task_id,
                    url_host = %url_host,
                    target = current_target,
                    active_connections,
                    target_wait_ms = phase.requested_at.elapsed().as_millis(),
                    "http range adaptive phase target reached"
                );
            }
        } else {
            tracing::debug!(
                task_id = %task_id,
                url_host = %url_host,
                current_target,
                active_connections,
                target_wait_ms = phase.requested_at.elapsed().as_millis(),
                "http range adaptive phase waiting for active target"
            );
            continue;
        }

        let Some(current_speed) = phase.measured_speed(received) else {
            continue;
        };

        let best_speed = tuner.best_speed.load(Ordering::Relaxed);

        if current_speed > best_speed {
            tuner.best_speed.store(current_speed, Ordering::Relaxed);
            tuner.best_target.store(current_target, Ordering::Relaxed);
        }

        let next_target = choose_connection_target(
            current_target,
            tuner.max_target,
            previous_phase_speed,
            tuner.best_speed.load(Ordering::Relaxed),
            tuner.best_target.load(Ordering::Relaxed),
            current_speed,
            tuner.historical_speed,
        );

        if next_target != current_target {
            let previous_target = tuner.limiter.set_target(next_target);
            tracing::info!(
                task_id = %task_id,
                url_host = %url_host,
                previous_target,
                next_target,
                current_speed,
                previous_phase_speed,
                best_speed = tuner.best_speed.load(Ordering::Relaxed),
                best_target = tuner.best_target.load(Ordering::Relaxed),
                historical_speed = ?tuner.historical_speed,
                active_connections,
                phase_sample_seconds = PHASE_SAMPLE_SECONDS,
                "http range adaptive phase target changed"
            );
            phase.reset(next_target, received);
        } else {
            tracing::debug!(
                task_id = %task_id,
                url_host = %url_host,
                current_target,
                current_speed,
                previous_phase_speed,
                best_speed = tuner.best_speed.load(Ordering::Relaxed),
                best_target = tuner.best_target.load(Ordering::Relaxed),
                historical_speed = ?tuner.historical_speed,
                active_connections,
                phase_sample_seconds = PHASE_SAMPLE_SECONDS,
                "http range adaptive phase target kept"
            );
            phase.reset(current_target, received);
        }

        previous_phase_speed = current_speed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_unknown_site_starts_from_global_default() {
        assert_eq!(initial_target(256, None), 8);
    }

    #[test]
    fn known_site_starts_from_last_effective_target() {
        assert_eq!(
            initial_target(
                256,
                Some(RangeTunerHistory {
                    last_effective_target: 64,
                    last_effective_speed_bytes_per_sec: 100,
                })
            ),
            64
        );
    }

    #[test]
    fn ramp_is_geometric_up_to_256() {
        assert_eq!(next_probe_target(8, 256), 16);
        assert_eq!(next_probe_target(16, 256), 32);
        assert_eq!(next_probe_target(32, 256), 64);
        assert_eq!(next_probe_target(64, 256), 128);
        assert_eq!(next_probe_target(128, 256), 256);
    }

    #[test]
    fn target_increases_when_throughput_improves() {
        let target = choose_connection_target(
            16,
            256,
            10 * 1024 * 1024,
            10 * 1024 * 1024,
            16,
            12 * 1024 * 1024,
            None,
        );

        assert_eq!(target, 32);
    }

    #[test]
    fn target_backs_off_to_best_observed_target() {
        let target = choose_connection_target(
            64,
            256,
            80 * 1024 * 1024,
            90 * 1024 * 1024,
            32,
            70 * 1024 * 1024,
            None,
        );

        assert_eq!(target, 32);
    }

    #[test]
    fn target_never_exceeds_planned_segment_ceiling() {
        let target = choose_connection_target(
            8,
            8,
            10 * 1024 * 1024,
            10 * 1024 * 1024,
            8,
            20 * 1024 * 1024,
            None,
        );

        assert_eq!(target, 8);
    }

    #[test]
    fn target_does_not_increase_without_progress() {
        let target = choose_connection_target(8, 256, 0, 0, 8, 0, None);

        assert_eq!(target, 8);
    }

    #[test]
    fn historical_degradation_steps_down() {
        let target = choose_connection_target(
            128,
            256,
            0,
            0,
            128,
            20 * 1024 * 1024,
            Some(100 * 1024 * 1024),
        );

        assert_eq!(target, 64);
    }
}
