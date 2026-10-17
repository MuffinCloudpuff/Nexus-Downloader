# ADR 0002: Task Events As Source Of Truth

Date: 2026-06-08

## Status

Accepted

## Context

Tasks used to carry both event history and mutable snapshot fields. That made it possible for `status`, `progress`, `speed`, or terminal failure details to diverge from the event log, especially after throttled progress updates or process restart.

The UI and diagnostics need to reconstruct a task timeline from durable events.

## Decision

Task events are the source of truth for task state transitions:

- `TaskService.append_event` applies a reducer to update snapshot fields.
- `Memory.log` load rebuilds snapshot fields from persisted events.
- Downloaders may call `update_task_status` as a helper for non-terminal progress/stage events.
- Terminal states must use explicit helpers:
  - `complete_task(task_id, output_path)`
  - `fail_task(task_id, failure)`
- `TaskCompleted` must include real `output_path`.
- `TaskFailed` must include stable `code`, `message`, `retryable`, and `stage`.

## Consequences

- Current task state can be rebuilt from the event log.
- Empty completion paths and generic failure events are no longer accepted as formal terminal events.
- Progress throttling must drop small updates instead of silently mutating snapshots without events.
