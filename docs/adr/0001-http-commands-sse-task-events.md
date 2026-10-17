# ADR 0001: HTTP Commands + SSE Task Events

Date: 2026-06-08

## Status

Accepted

## Context

The browser extension previously relied on a mixed WebSocket channel for task creation, task snapshots, and compatibility messages. That made command acceptance, task execution, UI state updates, and connection lifetime too tightly coupled.

The current extension needs a daemon task to keep running even when popup, overlay, service worker, or a streaming connection is interrupted.

## Decision

Use HTTP for commands and SSE for per-task event observation:

- `POST /api/tasks` creates a task and returns acceptance metadata.
- `POST /api/tasks/{id}/pause|resume|cancel` sends control commands.
- `GET /api/tasks/{id}/stream?from_event_id=n` streams task events.
- Snapshot APIs remain for startup recovery and reconciliation.
- The WebSocket gateway is legacy-only and must not receive new product behavior.

## Consequences

- A successful create command means the task was accepted, not completed.
- Download progress, completion, and failure are observed through task events.
- Service worker or popup restarts do not define task lifetime.
- Legacy WebSocket `create_task` may remain as a deprecated compatibility adapter, but formal clients must use HTTP/SSE.
