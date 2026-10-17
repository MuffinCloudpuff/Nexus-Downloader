# Task Card Operations Plan

## Scope

Task cards expose three user intents:

- Open artifact: locate the completed file from the desktop client.
- Retry task: ask the daemon to rerun an existing terminal task.
- Remove task: delete the task artifact and remove the task from the daemon task list.

The desktop UI must not implement file deletion or task recreation itself. It maps clicks to daemon protocol commands and refreshes task state from the daemon.

## Backend Responsibilities

- Add a daemon command for task artifact deletion: `POST /api/tasks/{task_id}/delete`.
- If the task is not completed, the daemon cancels the running or queued task first, waits for the worker to stop touching files, then deletes artifacts.
- If the task is completed, the daemon deletes the completed output file.
- In both cases, the daemon also removes known residual artifacts:
  - `Task.path` final output file.
  - `temp/{task_id}` runtime work directory.
  - `{output_parent}/.{task_id}.parts` YouTube stream work directory.
  - `data/resumes/{task_id}.manifest.json` resume manifest.
- After successful cleanup, remove the task from persisted task memory so later snapshots do not show a deleted task.
- Add a daemon command for retry: `POST /api/tasks/{task_id}/retry`.
- Retry is allowed only for terminal tasks: `completed`, `failed`, or `canceled`.
- Retry cleans stale artifacts first, appends a structured retry event, resets progress counters, requeues the existing task definition, and schedules it again.

## Frontend Responsibilities

- Task title and leading icon locate the completed artifact through the desktop shell adapter.
- The task-card `X` button calls the daemon delete command, not plain cancel.
- The completed/error retry button calls the daemon retry command, not resume.
- Paused tasks still use resume.
- After delete or retry, refresh the daemon snapshot so UI state remains owned by the backend.

## Verification

- Backend unit tests cover artifact cleanup and retry state reset.
- `cargo test` verifies daemon behavior.
- `npm run lint` and `npm run build` verify the React/Tauri frontend wiring.
