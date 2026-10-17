# Task Ordering And Artifact State Plan

## Goal

Make the download list and completed-file state reflect backend truth instead of UI-local guesses.

This change has two required outcomes:

- The "All downloads" list and type-specific lists sort by the real backend task creation time.
- Completed task cards can show whether the downloaded artifact is still available, missing, or moved.

## Non-Goals

- Do not scan the whole disk.
- Do not periodically scan configured download directories.
- Do not make the browser extension or desktop UI inspect the filesystem directly.
- Do not try to find files moved outside known task-related directories.

## Requirements

1. **Backend task creation time is the source of truth**

   Each task must have a backend-owned `created_at` timestamp recorded when the task is created. It must be persisted with the task and exposed in task snapshots.

   The desktop UI must sort by this backend timestamp, not by `Date.now()` generated during snapshot adaptation.

2. **Frontend sorting must be deterministic**

   `AllListView`, `VideoListView`, and `GenericTypeListView` should sort by backend `created_at` descending. If a legacy task has no timestamp, use a stable fallback such as event history or `0`; do not generate new timestamps that reorder old tasks on every UI reload.

3. **Artifact availability is backend state**

   The backend should expose an artifact state, for example:

   - `available`: the completed artifact is expected to exist at `output_path`.
   - `missing`: the completed artifact is no longer found at its known path.
   - `moved`: the completed artifact was moved or renamed inside a known watched directory and `output_path` was updated.
   - `unknown`: state is not known yet, mainly for legacy or non-completed tasks.

   The frontend only renders this state. It should not invent it.

4. **Missing files stay visible**

   When a completed artifact disappears from disk, the task should not be removed from the daemon snapshot. It should remain visible with `artifact_state = missing` so the UI can render it grey.

   Deleting the task card is a separate user command. Filesystem delete or move-out events must only update artifact state.

5. **Watched directories are task-related directories**

   The artifact monitor must watch a union of:

   - current configured category directories,
   - directories from completed tasks' actual `output_path`,
   - historical directories that are still referenced by persisted tasks,
   - new directories introduced by later settings changes or newly completed tasks.

   This is not full-disk tracking. It is the bounded set of directories the backend already knows are relevant.

6. **Moves inside watched directories can update paths**

   If a file is moved or renamed within the watched directory set and the backend can associate the old path with a completed task, update the task `output_path` and mark the artifact moved.

   If the file is moved outside the watched directory set, the backend should mark the task missing. It should not search the whole disk.

7. **Event-driven monitoring is the primary mechanism**

   Artifact state must be maintained by passive filesystem events, such as Windows `ReadDirectoryChangesW`.

   The monitor is a hook-style listener: it reacts when the filesystem reports create, delete, rename, or move events. It must not periodically walk the directories and must not perform a full-disk search.

## Allowed Bounded Checks

These checks are allowed because they do not scan directories and do not replace event-driven monitoring:

- On daemon startup, check only the known `output_path` values already persisted in tasks. This handles files deleted while the daemon was not running.
- When the user explicitly asks to open/reveal a file, optionally check that exact known path before opening. This is a safety fallback, not the main synchronization mechanism.

## Backend Design

### Task Model

Add fields to `Task`:

- `created_at: String`
- `artifact_state: ArtifactState`

Use an enum for `ArtifactState` with `snake_case` serialization:

- `unknown`
- `available`
- `missing`
- `moved`

New tasks get `created_at = now` and `artifact_state = unknown`.

When a task completes, set `artifact_state = available`.

When a completed artifact is deleted or moved out of known watched directories, set `artifact_state = missing`.

When a completed artifact is moved/renamed within watched directories and the path is updated, set `artifact_state = moved` or `available`. The protocol can expose `moved` to let UI distinguish "path updated" if desired.

### Task Events

Add artifact-related events only if needed by SSE clients:

- `task_artifact_available`
- `task_artifact_missing`
- `task_artifact_moved { old_path, new_path }`

At minimum, task snapshots must expose `artifact_state`.

### Task Summary Protocol

Expose:

- `created_at`
- `artifact_state`

The desktop UI and extension can ignore unsupported fields, so this is backward-compatible for current consumers.

### Artifact Monitor

Replace the current "configured category paths only" watch root calculation with:

- category paths from settings,
- parent directories of completed task output paths,
- any legacy output directories referenced by persisted tasks.

The monitor should restart a generation when:

- settings change,
- a completed task path changes,
- a task completes in a new directory.

Do not periodically rescan directories.

Deletion handling must not call task deletion. It should confirm the exact old path is still absent and then mark only `artifact_state = missing`.

### Startup Reconciliation

On startup, perform a one-time exact-path check for completed tasks:

- If `output_path` exists, mark `available`.
- If `output_path` does not exist, mark `missing`.

This is not directory scanning. It checks only known files.

## Frontend Design

### Types

Add to `TaskSummary` and `DownloadTask`:

- `created_at?: string`
- `artifact_state?: "unknown" | "available" | "missing" | "moved"`

### Adapter

Map `TaskSummary.created_at` into `DownloadTask.createdAt`.

Do not use `Date.now()` as the primary value. For legacy tasks without `created_at`, use `0` or preserve an existing value only if it came from the backend.

### Sorting

Sort descending by backend creation timestamp in:

- `AllListView`
- `VideoListView`
- `GenericTypeListView`

### Missing UI

The UI should render missing artifacts grey/disabled based on `artifact_state === "missing"`.

This visual treatment can be implemented separately by the frontend UI agent, but the field must be available in the task object.

## Verification

Backend:

- Unit test: new tasks have `created_at` and default artifact state.
- Unit test: completed task snapshots expose `artifact_state = available`.
- Unit test: missing artifact update persists and appears in snapshots.
- Artifact monitor test: watched roots include current category paths and completed task output parent directories.

Frontend:

- Type check passes.
- Build passes.
- Sorting uses backend timestamp.
- Task adapter maps `artifact_state`.

Manual:

- Download a file and confirm the card sorts to the top.
- Delete the completed file while daemon is running; backend receives an event and marks it missing.
- Restart UI; the task remains visible and sorted by backend time.
- Restart daemon after deleting a completed file while daemon was offline; startup exact-path check marks it missing.
