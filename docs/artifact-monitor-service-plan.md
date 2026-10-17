# Artifact Monitor Service Plan

## Goal

The daemon is the source of truth for completed download artifacts. If a user changes files in Windows Explorer, the daemon should keep the task list consistent with the real disk state.

This is a backend capability. Desktop UI, browser extension, or future clients only display daemon snapshots and do not decide whether a downloaded artifact still exists.

## Phase 1 Semantics

- Watch the configured category directories: video, audio, document, archive, program, and other.
- When a completed task artifact is deleted from a watched directory, remove that task from daemon memory and persistence.
- When a completed task artifact is renamed or moved within watched directories, update `Task.path` so future open/locate actions still work.
- Do not watch the whole disk in phase 1.
- Do not run periodic full-directory scans as the primary mechanism.

## Move Detection Boundary

File system events are reliable for changes inside watched directories. A move can be tracked when the daemon observes both sides of the change:

- rename inside the same watched directory
- move from one watched category directory to another watched category directory

If the user moves a file outside watched directories, the daemon may only see the original path disappear. Phase 1 treats that as removal after a short debounce window. Full-disk relocation requires a later Windows-specific strategy such as file IDs or the USN journal.

## Backend Design

- Add `artifact_monitor_service`.
- Build a path index from completed tasks: normalized artifact path -> task id.
- Listen to category directories recursively with `notify`.
- For remove events:
  - wait briefly so a paired create event can arrive for moves
  - if no same-file candidate appears, remove the task from `TaskService`
- For create/rename events:
  - match by exact rename pair when provided
  - otherwise match a recently removed task by filename and file size
  - update the task path through `TaskService`
- Refresh watched directories when settings are patched by restarting the monitor loop from current settings.

## Task Service Requirements

- `remove_task(task_id)` already removes a task from the snapshot and persistence.
- Add `update_task_output_path(task_id, new_path)` to update persisted task path and emit a task change.
- Keep protocol stable: `TaskSummary.output_path` already carries the updated path to clients.

## Verification

- Unit-test path index behavior and move matching where possible.
- Integration-test task path update and task removal via `TaskService`.
- Run `cargo test` after implementation.
