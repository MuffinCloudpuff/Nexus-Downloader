# Browser Extension and HTTP Download Reliability Plan

## Scope

This plan covers three connected problems:

1. Browser native downloads are not reliably intercepted by the extension.
2. The extension task list does not reflect backend artifact state changes such as `missing`.
3. Large direct HTTP files can be slower than expected even when the server supports Range requests.

The architecture boundary is unchanged: the browser extension and tray UI are replaceable clients, the HTTP API and task events are the stable contract, and the Rust backend owns download scheduling, artifact state, persistence, and file-system truth.

## Root Causes To Address

### Extension auto-intercept

The extension has `chrome.downloads` listeners, but `interceptBrowserDownload` currently exits when the daemon client is not `connected`. That skip path records no telemetry, so a real download event can be lost without a visible reason.

Fix:

- Record every intercept decision as structured telemetry.
- If the daemon client is not connected but has a token, attempt a synchronous reconnect before skipping.
- Record exact skip reasons: disabled, non-http URL, daemon not ready, cancel failure, handoff failure.
- Keep sensitive URL query values out of logs; record host and path kind instead of full signed URLs.

### Extension artifact state

The desktop UI already consumes global task events, including `task_artifact_missing`. The extension still uses per-active-task subscriptions and stops listening once a task is terminal. That means a completed task can become missing in the backend without the extension seeing the event.

Fix:

- Add a global `/api/tasks/stream` client in the extension.
- Keep one background stream alive after daemon connection.
- Apply `task_artifact_available`, `task_artifact_missing`, and `task_artifact_moved` to extension task state.
- Render missing artifacts as gray cards and disable open actions.

### HTTP large-file speed

The direct `.rar` URL supports `Content-Length` and byte ranges. The task execution stage can discover the real size, but task creation persists `file_size=0` and `block_num=1`, so UI and history do not match runtime facts. The observed speed still needs a focused Range concurrency test because NDM reaches much higher throughput on the same link.

Fix:

- Probe direct HTTP files during task creation.
- Persist expected size, Range support, and a planned max connection budget in the execution plan.
- Log actual chunk count, chunk size, max setting, and detected response metadata at runtime.
- Add a small diagnostic path/test harness before changing the download strategy further. Do not “solve” this by only raising connection limits.

## Files Expected To Change

- Extension background:
  - `browser-extension/app/src/background.ts`
  - `browser-extension/app/src/background/daemon-client.ts`
  - `browser-extension/app/src/background/task-event-subscriptions.ts`
  - `browser-extension/app/src/background/task-store.ts`
- Extension protocol/UI:
  - `browser-extension/app/src/shared/protocol/daemon.ts`
  - `browser-extension/app/src/shared/types.ts`
  - `browser-extension/app/src/shared/utils.ts`
  - `browser-extension/app/src/popup/components/TaskCard.tsx`
- Backend HTTP planning/runtime:
  - `src/parsers/protocol/http_file.rs`
  - `src/core/execution_plan.rs`
  - `src/core/engine/downloader.rs`

## Verification

- Extension typecheck/build passes.
- Backend tests pass.
- A deleted completed file emits `task_artifact_missing` and the extension card turns gray without reopening the popup.
- A direct HTTP file task is created with real file size and Range capability in task summary.
- Runtime logs show planned chunk count and actual chunk count for direct HTTP tasks.
