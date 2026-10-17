# Adaptive Range Tuner And Site History Plan

## Goal

The Range downloader should keep a global adaptive algorithm while learning a
per-site warm-start point. A site record is not a hard cap and not a permanent
maximum. It is only the next task's starting target.

## Problems To Fix

1. The current tuner starts unknown and known sites the same way.
2. The current ramp is too conservative after 64 connections.
3. A fixed historical maximum is brittle when the user's network changes.
4. A fixed historical low value is also brittle when the user's network later
   improves.

## Target Behavior

- No history: start from the global default target.
- With history: start from `last_effective_target`.
- Every task may still probe upward after the warm start.
- A poor current run may back off from the warm start.
- The next run uses the last effective result, not the all-time maximum.
- Small files still cap the active target by planned Range segments.

## Tuning Model

The tuner must measure connection-target response, not just a fixed wall-clock
window. A sample is valid only after the requested active connection target has
actually been reached.

For each phase:

- target active connections;
- time when that target was requested;
- time when active workers reached that target;
- bytes transferred after the target was reached;
- phase throughput;
- speed delta versus the previous phase;
- gain ratio and marginal gain per added connection.

The ramp ladder is geometric:

```text
8 -> 16 -> 32 -> 64 -> 128 -> 256
```

The tuner continues up the ladder when throughput growth is meaningful. It
stops or backs off when the marginal gain is too small, throughput drops, or the
current run is far below historical speed.

This avoids the previous problem where a 3-second sample could include worker
startup, parsing delay, or an old connection target and then make a decision
from mixed data.

## Site History Model

Persist records under daemon data, keyed by download host for the first
implementation:

```text
site_key
last_effective_target
last_effective_speed_bytes_per_sec
sample_count
last_sample_at_epoch_ms
```

The record controls only the next starting target. It must not prevent upward
probing. `best_observed_*` may be added later for diagnostics, but it should not
drive warm-start behavior.

## Update Rules

Update site history only when the run has enough signal:

- enough bytes were transferred;
- enough time elapsed;
- an effective target and speed were observed;
- the run was not a failed tiny probe.

Canceled/failed tasks should not overwrite history in this implementation.

## Module Boundary

- `src/core/engine/range_tuner.rs`
  - adaptive target decisions;
  - limiter;
  - speed sample tracking.
- `src/services/site_performance_store.rs`
  - JSON persistence;
  - per-site record load/get/update.
- `src/core/engine/downloader.rs`
  - uses the tuner and store;
  - does not own tuning policy details.
