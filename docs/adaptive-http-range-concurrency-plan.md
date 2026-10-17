# Adaptive HTTP Range Concurrency Plan

## Goal

HTTP file downloads should not require users to manually guess the right
connection count. The daemon should treat `max_chunk_connections` as a safety
ceiling and choose the task's active Range worker count from runtime
measurements.

This is different from changing a fixed value from 128 to 256. A fixed higher
value can hide the problem or make some CDNs slower. The root behavior we want
is backend-owned tuning.

## Current State

- `src/core/engine/downloader.rs` plans fine-grained Range segments from file
  size and `max_chunk_connections`.
- The same value is also used as a fixed semaphore size, so every task can run
  up to that many active chunk requests immediately.
- `nexus-downloader` exposes this as `threads`, which makes the user responsible
  for a backend scheduling decision.

## Target Semantics

- `max_chunk_connections`: single-task maximum active Range connection ceiling.
- `initial_active_connections`: conservative startup value, currently 8.
- `adaptive_active_connections`: current daemon-selected active connection
  count for one task.
- `planned_range_segments`: queued logical byte ranges, usually more than the
  active connection count.

The UI may display the maximum or the current active count as diagnostics, but
manual selection should not be required for normal use.

## Backend Design

1. Raise the allowed ceiling from 128 to 256.
2. Keep the minimum at 8.
3. Start each HTTP Range task at 8 active workers or fewer if the task has fewer
   segments.
4. Add a per-task adaptive limiter instead of a fixed semaphore.
5. Every measurement window, compare throughput against the previous and best
   windows:
   - if throughput is still improving, increase the active target;
   - if throughput drops materially after an increase, step back toward the best
     observed target;
   - if the task is almost done or all segments are active, stop tuning.
6. Log each tuning decision with task id, host, old target, new target,
   measured speed, previous speed, and best speed.

## Frontend / Protocol Design

- Keep `max_chunk_connections` in the protocol for compatibility and advanced
  diagnostics.
- Treat it as a ceiling, not as the exact active worker count.
- The desktop React UI should stop presenting this as a required manual
  "thread" choice. If shown, it should be labeled as an advanced maximum.
- Runtime metrics already expose `active_chunk_connections`; that is the value
  users should look at when diagnosing current activity.

## Verification

- Unit tests:
  - clamping accepts 256 and rejects values above it;
  - default ceiling is 256;
  - range segment planning can create enough queued segments for 256 workers;
  - adaptive target selection increases on improvement and backs off on a clear
    regression.
- Manual validation:
  - run the known large HTTP URL;
  - confirm logs show adaptive decisions rather than a fixed connection count;
  - compare frontend speed with backend task speed and logs.
