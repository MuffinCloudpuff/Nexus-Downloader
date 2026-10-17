# Nexus Downloader Daemon Integration Plan

## Positioning

`nexus-downloader` should be treated as another frontend client of `gd3-daemon`, not as a new downloader backend.

```text
nexus-downloader React UI
  -> stable HTTP/SSE protocol
  -> gd3-daemon backend core
```

This is the same architectural role as the browser extension. The container capability is different: the extension can capture browser page media, while the Tauri desktop client can own desktop window, tray, local settings, file picker and notifications. Both should remain protocol clients.

Important distinction:

- Browser extension/background is the always-on capture client. It should send browser-side facts to the daemon.
- Desktop UI is the management/configuration client. It may be closed while downloads are created.
- Daemon is the policy and execution owner. It classifies output category, maps category to configured directory, validates paths and writes files.

## Implementation Ownership Decision

This project should be implemented as three stable layers rather than as one UI-driven downloader:

```text
frontend clients
  -> protocol contract
  -> daemon policy/execution backend
```

Frontend clients include both the browser extension and the desktop React/Tauri UI. They have different capabilities, but the same architectural role: they are protocol clients.

### Browser Extension Responsibilities

The extension is the always-on capture frontend. It should stay thin and only send facts it can observe from the browser:

```text
url
title
filename
mime
size
supports_range
captured resources
client context
trace/request ids
```

It must not:

- classify files into video/audio/document/archive/program/other;
- keep a category-to-directory dictionary;
- choose the daemon output path;
- send `output_directory` or any equivalent final path field.

The first `POST /api/tasks` request from the extension should be a factual handoff, not a routing decision.

### Daemon Responsibilities

The daemon owns all durable policy and execution decisions:

```text
CreateTask facts
  -> parser/site resolution
  -> output category classification
  -> settings.category_paths lookup
  -> final output path
  -> downloader writes directly to Task.path
```

The daemon must validate and persist category paths before accepting them as settings. A task's final output path must be decided before the download starts, not by moving files after completion.

The backend work should stay split by responsibility:

```text
src/protocol/settings.rs                 settings DTOs and wire contract
src/services/settings_service.rs         persistence, validation, runtime config update
src/parsers/policy/output_category.rs    file/category classification
src/parsers/policy/output_destination.rs category path lookup and final path construction
src/core/engine/*                        execute using Task.path only
```

### Desktop UI Responsibilities

The desktop UI is a management/configuration frontend. It can be closed while the extension continues creating tasks.

It should:

- display task snapshots, events, progress, speed and errors from the daemon;
- send pause/resume/cancel/create commands through the protocol;
- configure daemon-backed settings through `GET /api/settings` and `PATCH /api/settings`;
- use Tauri only for desktop shell capabilities such as folder picker, tray, notifications and window management.

It must not:

- duplicate daemon file classification rules;
- compute final daemon output paths;
- bypass the daemon by putting download execution into Tauri commands.

Directory selection belongs to the desktop shell boundary, but the selected absolute path is still only a setting value. The daemon remains responsible for validating it and applying it to future tasks.

### Replaceable UI Boundary

The visible React desktop UI may be replaced by a pasted or generated design version. That replacement must not erase the application shell, protocol adapters, daemon synchronization or desktop platform integration.

Keep these responsibilities separated:

```text
src/ui/**             replaceable visual UI and component layout
src/shell/**          frontend host, stores, UI-facing hooks and daemon sync wiring
src/adapters/**       DTO-to-UI and UI-to-DTO mapping
src/api/**            daemon HTTP/SSE client
src/platform/**       Tauri/browser desktop capabilities
src/protocol/**       stable protocol DTOs
```

The UI layer may own presentation, grouping, filters, dialogs and local interaction state. It should receive app capabilities through shell-facing hooks or props such as task snapshots, settings, commands and directory-picking actions.

The shell layer is an integration boundary, not part of the replaceable visual design. It should survive UI replacement and reconnect the UI to daemon protocol, Tauri tray/window behavior, native folder picker, notifications and local UI preferences.

When a new UI version is pasted in, merge it into the replaceable UI boundary first. Do not paste over shell, protocol, API, adapter or platform modules. If the new UI needs data or commands that are not exposed by the shell boundary, add them intentionally to the shell/protocol contract instead of reaching into daemon or Tauri code from visual components.

## Current Frontend Shape

CodeGraph scan result:

- Project: `D-study-workspace-Rust_workspace-gd3-daemon-nexus-downloader`
- Nodes: 116
- Edges: 213
- Main layers:
  - `src/App.tsx`: entry/layout.
  - `src/store.tsx`: global state and actions. Current hotspot: `useApp`, fan-in 15.
  - `src/types.ts`: UI task types.
  - `src/components/*`: visual components and modals.
  - `src/components/views/*`: list grouping/filter views.

The current structure is suitable for integration because UI components already consume a shared store instead of owning task state individually.

Important existing hooks:

- `Settings.threads`: already exists as a UI concept, currently local mock state.
- `Settings.paths`: already separates category paths.
- `DownloadTask`: already has `id/name/url/size/downloaded/status/speed/platform/type/createdAt`.
- `NewTaskModal`: already has the manual URL entry surface.
- `TaskCard`: already has pause/resume/cancel/retry button positions.
- `VideoListView`: already supports platform grouping for Bilibili / YouTube / X.

## Current Daemon Protocol Surface

Existing `gd3-daemon` endpoints:

```text
POST /api/tasks
GET  /api/tasks
GET  /api/tasks/{task_id}
GET  /api/tasks/{task_id}/events
GET  /api/tasks/{task_id}/stream
POST /api/tasks/{task_id}/pause
POST /api/tasks/{task_id}/resume
POST /api/tasks/{task_id}/cancel
POST /api/client-events
POST /api/pairing/request
POST /api/pairing/revoke
POST /api/auth/bilibili/qr_generate
GET  /api/auth/bilibili/qr_status
GET  /api/auth/bilibili/me
```

Existing protocol objects that the new frontend should reuse conceptually:

- `CreateTaskHttpReq`
- `CreateTaskHttpResp`
- `TaskSummary`
- `TaskEvent`
- `TaskCommandResp`
- `TaskError`

The browser extension already has TypeScript protocol definitions in:

```text
browser-extension/app/src/shared/protocol/daemon.ts
```

Do not import that file directly from `nexus-downloader` unless the repo is deliberately turned into a shared workspace package. Copying first is acceptable, but the better long-term shape is a shared protocol package.

## Integration Boundary

Add a frontend adapter layer instead of calling `fetch` from components.

Recommended modules:

```text
src/protocol/daemon.ts
src/api/daemon-client.ts
src/adapters/task-adapter.ts
src/effects/daemon-task-sync.ts
src/effects/settings-sync.ts
```

Responsibilities:

- `protocol/daemon.ts`
  - TypeScript copies of stable daemon DTOs.
  - No React, no UI state.

- `api/daemon-client.ts`
  - HTTP command client.
  - Auth token handling.
  - SSE parsing.
  - Pairing request.
  - Task commands.
  - No UI mapping.

- `adapters/task-adapter.ts`
  - `TaskSummary -> DownloadTask`.
  - `TaskEvent -> partial DownloadTask update`.
  - UI category inference: video/audio/document/archive/program/other.
  - Platform inference: Bilibili/YouTube/X/other from `pack_name`, `source`, title, or URL.

- `effects/daemon-task-sync.ts`
  - On app start: connect, fetch snapshot.
  - Subscribe to task streams.
  - Apply task updates to `store`.
  - Reconnect on stream failure.

- `effects/settings-sync.ts`
  - Read and write settings once daemon exposes stable settings endpoints.
  - Until then, keep local UI settings local and mark daemon-backed settings as disabled or pending.

The desktop UI should not duplicate daemon output policy. It can display category grouping and configure category paths, but filename/MIME/category-to-path rules belong in daemon modules.

## Store Changes

Keep the current `useApp` shape, but split local-only UI preferences from daemon-backed runtime settings.

Suggested state split:

```text
uiSettings
  language
  theme
  autoOrganizeVideos

daemonSettings
  downloadFolder
  maxTaskNum
  maxChunkConnections
  speedLimitEnabled
  speedLimitBytesPerSec
  proxyServer
  categoryPaths

connection
  state
  message
  serverUrl
  token
```

The current `settings.threads` should map to daemon `max_chunk_connections`, not raw OS threads. It should mean:

```text
single task max range chunk concurrency
```

Recommended UI range:

```text
8..128, default 8
```

The value is still daemon `max_chunk_connections`, not raw OS threads. It caps the
number of concurrent chunks for one task after the backend computes intelligent
chunks from file size.

## Backend Gaps To Add

The frontend already has a settings UI, but the daemon does not yet expose a stable settings API.

Required daemon additions:

```text
GET   /api/settings
PATCH /api/settings
```

Suggested daemon settings fields:

```text
download_folder: string
max_task_num: number
max_chunk_connections: number
enable_speed_limitation: boolean
speed_limitation: number
proxy_server: string
category_paths:
  video: string
  audio: string
  document: string
  archive: string
  program: string
  other: string
```

Downloader integration:

```text
intelligent_chunks = max(1, total_size / 20MB)
actual_chunks = min(intelligent_chunks, max_chunk_connections)
```

Also update CORS for local desktop dev:

```text
http://localhost:3000
http://127.0.0.1:3000
tauri://localhost if needed by the final Tauri shell
```

Required daemon output policy modules:

```text
src/protocol/settings.rs
src/services/settings_service.rs
src/parsers/policy/output_category.rs
src/parsers/policy/output_destination.rs
```

Output policy ownership:

```text
CreateTaskPayload facts
  -> parser/site resolution facts
  -> daemon output_category classification
  -> daemon settings category_paths lookup
  -> ExecutionPlan.output final path
  -> executor writes to that path
```

The browser extension should not send `output_directory` and should not maintain a category dictionary. It should send richer facts instead:

```text
filename
mime
size
supports_range
resources[].kind
resources[].mime
resources[].filename
resources[].size
resources[].page_url
resources[].page_title
```

The desktop UI should configure `category_paths` through daemon settings. The native folder picker belongs to the Tauri/desktop shell boundary, not the daemon.

## First Implementation Sequence

1. Protocol client foundation
   - Add `src/protocol/daemon.ts`.
   - Add `src/api/daemon-client.ts`.
   - Add connection state to store.
   - Keep mock tasks as fallback only when disconnected.

2. Task snapshot integration
   - `GET /api/tasks` on startup.
   - Map daemon `TaskSummary` into `DownloadTask`.
   - Render real daemon tasks in existing list views.

3. Task command integration
   - Wire `TaskCard` buttons to pause/resume/cancel.
   - Do not let buttons mutate UI state directly; wait for daemon response or task event.

4. Manual create task
   - Wire `NewTaskModal` to `POST /api/tasks`.
   - Generate request id and trace id on client.
   - Use source `desktop_manual`.

5. Live event sync
   - Subscribe to task streams for active tasks.
   - Apply `TaskEvent` to store via adapter.
   - Later consider daemon-wide task event stream to avoid N streams if many tasks are active.

6. Settings integration
   - Add daemon settings API first.
   - Map `threads` UI to `max_chunk_connections`.
   - Use range `8..128`, default `8`.
   - Add category path settings and persist them in daemon settings.
   - Use native desktop folder picker from the Tauri shell when selecting a directory.

7. Tauri shell integration
   - Add Tauri only after web client can talk to daemon cleanly.
   - Use Tauri for tray/window/file picker/notification.
   - Do not move downloader logic into Tauri commands.

## Files Likely To Change First

Frontend:

```text
nexus-downloader/src/types.ts
nexus-downloader/src/store.tsx
nexus-downloader/src/components/NewTaskModal.tsx
nexus-downloader/src/components/TaskCard.tsx
nexus-downloader/src/components/SettingsModal.tsx
nexus-downloader/vite.config.ts
```

New frontend files:

```text
nexus-downloader/src/protocol/daemon.ts
nexus-downloader/src/api/daemon-client.ts
nexus-downloader/src/adapters/task-adapter.ts
nexus-downloader/src/effects/daemon-task-sync.ts
nexus-downloader/src/effects/settings-sync.ts
```

Backend:

```text
src/services/config.rs
src/core/engine/downloader.rs
src/gateway/http_api.rs
src/protocol/settings.rs
src/protocol/mod.rs
```

## Key Rule

`nexus-downloader` should never depend on private Rust implementation details. It should only depend on stable daemon protocol objects and commands.

If a UI feature needs data that is not in the protocol, add it to the protocol intentionally instead of reaching around the protocol boundary.
