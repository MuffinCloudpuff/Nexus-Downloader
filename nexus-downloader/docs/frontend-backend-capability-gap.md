# Nexus Downloader Frontend/Backend Capability Gap

## Purpose

This document lists the product capabilities already expressed by the `nexus-downloader` frontend, then checks whether `gd3-daemon` already has a matching protocol/backend module. It is not an implementation patch. It is the development map for connecting frontend clients without mixing UI logic, protocol contracts and backend execution logic.

Terminology:

- "Frontend client" does not only mean the visible `nexus-downloader` desktop UI.
- For most real download creation flows, the always-on frontend client is the browser extension background process, because it captures browser resources and talks to the daemon even when the desktop UI is closed.
- `nexus-downloader` is mainly the desktop management/configuration UI. It can configure user rules, but it should not be assumed to be open when every task is created.

Core rule:

```text
Frontend owns interaction and presentation.
Protocol owns stable commands, state, events, errors and settings contracts.
Backend owns durable settings, task execution, output paths, filesystem facts and external integrations.
```

Implementation ownership rule:

```text
browser extension facts
  -> protocol DTO
  -> daemon classification/path policy
  -> daemon execution
  -> desktop UI observation/configuration
```

The browser extension and desktop UI are both frontend clients. The extension is usually the task-creating frontend because it is always open in the browser. The desktop UI is mainly the management and configuration frontend. Neither frontend should own durable output routing.

Concrete ownership:

- Browser extension: capture observable browser facts and send them through `POST /api/tasks`.
- Protocol: carry factual fields with stable names and explicit types.
- Daemon: classify output category, look up configured category directory, validate paths, persist settings and write files.
- Desktop UI/Tauri: select directories with native shell capability and patch daemon settings; display daemon state.

Replaceable UI constraint:

- Treat visual desktop UI files as replaceable design assets. Keep frontend shell wiring, daemon sync, protocol adapters, daemon API clients and Tauri platform capabilities in separate non-replaceable modules. A pasted/generated UI update should merge into the UI boundary only, then consume shell-facing hooks or props. See `daemon-integration-plan.md` for the directory-level boundary.

Do not implement category routing as a frontend dictionary. The frontend may know a filename or MIME type, but it should pass those facts to the daemon. The daemon should decide whether `.mp4`, `video/mp4`, Bilibili DASH, YouTube, X HLS, PDF, ZIP or any other input maps to `video`, `document`, `archive` or another category.

## Current Frontend Capability Map

| Frontend capability | Frontend location | Pure frontend? | Backend/protocol status | Notes |
|---|---|---:|---|---|
| Category navigation: all/video/audio/document/archive/program/other | `src/components/Sidebar.tsx`, `src/components/views/*` | Yes | Needs task metadata only | Filtering and grouping can stay frontend-side. |
| Video auto-organize by platform: Bilibili/YouTube/X/other | `src/components/views/VideoListView.tsx` | Yes | Needs `platform` derived from task metadata | Frontend can keep grouping. Backend only needs to expose enough stable metadata. |
| Platform and file-type icons/colors | `src/components/TaskCard.tsx` | Yes | Needs `pack_name`, title, extension or category | UI-only. Do not put icon rules into backend. |
| Search box | `src/components/SearchBar.tsx` | Mostly yes | No backend module | For local task list, frontend can filter in-memory. Backend search only needed later for huge persisted history. |
| Language and theme | `src/components/SettingsModal.tsx`, `src/i18n.ts` | Yes | No backend needed | Local UI preferences. Store in frontend/Tauri local storage. |
| Manual URL download | `src/components/NewTaskModal.tsx` | No | Existing `POST /api/tasks` can support it | Needs frontend submit wiring and daemon client. |
| Pause/resume/cancel buttons | `src/components/TaskCard.tsx` | No | Existing APIs exist | Needs frontend command wiring. Do not mutate status locally as truth. |
| Task list, progress, speed, status | `src/store.tsx`, `src/components/TaskCard.tsx` | No | Existing `GET /api/tasks` and task events exist | Needs adapter from `TaskSummary`/`TaskEvent` to `DownloadTask`. |
| Global speed display | `src/components/GlobalSpeed.tsx` | Mostly yes | Existing task speed fields enough | Sum active task speeds from daemon task snapshot/events. Backend metric endpoint optional later. |
| Active threads display | `src/components/ActiveThreads.tsx` | No | No precise backend runtime metric | Current frontend displays settings value only. If it should mean real active chunk count, backend needs metrics. |
| Storage free space display | `src/components/StorageStats.tsx` | No | No backend API | Needs backend filesystem/disk stats because daemon writes files. |
| Bilibili login QR | `src/components/BilibiliLoginModal.tsx` | No | Existing QR APIs exist | Frontend QR is mock; can wire to daemon auth APIs. |
| Download paths per category | `src/components/SettingsModal.tsx`, `src/store.tsx` | No | Missing backend settings and destination policy | Desktop UI configures category directories; daemon stores them and applies the category-to-path policy. |
| Auto category download destination | `settings.paths` in desktop UI, persisted in daemon settings | No | Missing protocol facts, backend category policy and destination policy | Must be chosen before download starts by daemon, using browser-provided facts plus backend classification rules, not moved after completion. |
| Settings persistence | `src/store.tsx` local state only | Partly | Daemon has in-memory `AppConfig`, no HTTP settings API | Split UI-local settings from daemon-backed settings. |

## Backend Capabilities That Already Exist

These can be connected without inventing new backend modules.

### Task Snapshot And Task Events

Existing protocol/backend:

```text
GET  /api/tasks
GET  /api/tasks/{task_id}
GET  /api/tasks/{task_id}/events
GET  /api/tasks/{task_id}/stream
```

Existing backend types:

```text
TaskSummary
TaskEvent
TaskError
```

Frontend work:

```text
src/protocol/daemon.ts
src/api/daemon-client.ts
src/adapters/task-adapter.ts
src/effects/daemon-task-sync.ts
```

Development rule:

- Components should keep rendering `DownloadTask`.
- Adapter maps daemon `TaskSummary`/`TaskEvent` to `DownloadTask`.
- Do not expose Rust-internal task structs to React.

### Task Creation

Existing endpoint:

```text
POST /api/tasks
```

Frontend work:

- `NewTaskModal` should collect URL and optional title.
- Create request with `source = "desktop_manual"`.
- Include `request_id`, `trace_id`, `client_context.protocol_version`.

Backend gap:

- None for basic URL tasks.
- Category destination policy still affects where the task writes.

### Task Commands

Existing endpoints:

```text
POST /api/tasks/{task_id}/pause
POST /api/tasks/{task_id}/resume
POST /api/tasks/{task_id}/cancel
```

Frontend work:

- `TaskCard` buttons call daemon client.
- UI should show pending command state if needed.
- Final status must come back from task events/snapshot.

### Bilibili QR Auth

Existing endpoints:

```text
POST /api/auth/bilibili/qr_generate
GET  /api/auth/bilibili/qr_status
GET  /api/auth/bilibili/me
```

Frontend work:

- Replace mock QR pattern with daemon QR response.
- Poll status until confirmed/expired.
- Display logged-in account state from `/me`.

Backend gap:

- Optional: expose clearer auth state and expiration errors if current response shape is not enough.

## Backend Gaps To Develop

### 1. Settings API

Frontend capability:

- Settings modal already exposes engine settings and category paths.

Current backend:

- `src/services/config.rs` has `AppConfig`.
- It is currently `OnceLock<AppConfig>`, effectively read-only after init.
- No `GET /api/settings`.
- No `PATCH /api/settings`.
- No category path settings.

Required modules:

```text
src/protocol/settings.rs
src/services/settings_service.rs
src/gateway/http_api.rs
```

Recommended backend contract:

```rust
pub struct AppSettingsDto {
    pub download_folder: String,
    pub max_task_num: u32,
    pub max_chunk_connections: u32,
    pub enable_speed_limitation: bool,
    pub speed_limitation: u64,
    pub proxy_server: String,
    pub category_paths: CategoryDownloadPaths,
}

pub struct CategoryDownloadPaths {
    pub video: String,
    pub audio: String,
    pub document: String,
    pub archive: String,
    pub program: String,
    pub other: String,
}
```

Required endpoints:

```text
GET   /api/settings
PATCH /api/settings
```

Implementation notes:

- Validate paths before accepting, or mark invalid paths clearly in response.
- Settings updates that affect new tasks should not rewrite already running task paths.
- Persist settings to a daemon-owned JSON file, not only memory.
- Keep UI-only settings such as language/theme out of daemon settings.

### 2. Output Category And Directory Mapping

Decision:

- Browser extension/background is a thin always-on capture client.
- It should send browser-side facts, not maintain the category dictionary or choose output paths.
- Daemon owns category classification, category-to-path mapping and final output path selection.
- Desktop UI owns configuration of the category directory dictionary.

Browser extension facts to send:

```text
url
title
filename
mime
size
supports_range
headers
resources[].url
resources[].kind
resources[].mime
resources[].filename
resources[].size
resources[].headers
resources[].page_url
resources[].page_title
client_context.extension_id
client_context.tab_id
client_context.page_url
client_context.trace_id
client_context.protocol_version
```

Daemon-side rule:

```text
browser-provided facts
  -> backend category classification
  -> daemon settings category_paths[category]
  -> final output path before download starts
```

For example:

```text
.mp4 / video/mp4 / Bilibili DASH / YouTube / X HLS -> video    -> category_paths.video
.flac / audio/*                                      -> audio    -> category_paths.audio
.pdf / .docx / .xlsx / .pptx / text/*                -> document -> category_paths.document
.zip / .7z / .rar / .tar / .gz                       -> archive  -> category_paths.archive
.exe / .msi / .deb / .dmg / .apk                     -> program  -> category_paths.program
unknown                                              -> other    -> category_paths.other
```

Current backend:

- `TaskSummary` does not expose an explicit category.
- `CreateTaskHttpReq` has some facts already, but the extension may not populate all filename/MIME/resource facts consistently.
- Backend does not validate or persist category-path settings.
- Backend does not consistently use a single destination policy when creating and executing `ExecutionPlan.output`.

Required browser extension work:

```text
browser-extension/app/src/background/resource-bridge.ts
browser-extension/app/src/background/media-bridge.ts
browser-extension/app/src/shared/protocol/daemon.ts
```

Extension work is limited to completing fact capture and protocol population. Do not add category-path dictionaries or output-directory selection to the extension.

Required protocol additions:

```rust
pub struct CreateTaskPayload {
    // existing fields...
    pub filename: Option<String>,
    pub mime: Option<String>,
    pub size: Option<u64>,
    pub supports_range: Option<bool>,
}
```

`CapturedResource` should also continue carrying per-resource `kind`, `mime`, `filename`, `size`, `headers`, `page_title` and `page_url`.

Required backend modules:

```text
src/parsers/policy/output_category.rs
src/parsers/policy/output_destination.rs
```

Recommended enum:

```rust
#[serde(rename_all = "snake_case")]
pub enum OutputCategory {
    Video,
    Audio,
    Document,
    Archive,
    Program,
    Other,
}
```

Backend classification inputs:

- top-level payload filename
- top-level payload MIME
- URL path extension
- captured resource kind/MIME/filename
- parser/site result
- `ExecutionKind`
- pack name fallback

Task summary protocol addition:

```rust
TaskSummary {
    output_category: Option<OutputCategory>,
    output_path: Option<String>, // optional if safe to expose
}
```

Design rule:

- Extension sends facts only.
- Daemon applies classification and destination policy.
- Desktop UI configures category directories through daemon settings.
- Daemon must decide the final target before download starts.
- Finished downloads should not be moved afterward as the primary path strategy.

### 3. Output Destination Policy

Frontend capability:

- User can set different directories per file category.

Current backend:

- Output path is scattered:
  - Some protocol parsers write `./data/{task_id}.mp4`.
  - Some protocol parsers write `./downloads/{task_id}`.
  - `output_naming` writes `downloads/...`.
  - Some downloaders recompute final output path during execution.
- `ExecutionPlan.output` exists, but is not consistently the final path source of truth.

Required module:

```text
src/parsers/policy/output_destination.rs
```

Required design:

```text
CreateTaskCommand
  -> SiteResolution
  -> ProtocolParser creates ExecutionPlan from observed facts
  -> daemon output category classification
  -> daemon category path lookup
  -> daemon path validation
  -> ExecutionPlan.output = final file path
  -> Task.path = final file path
  -> Downloader writes directly to Task.path
```

The previous wording "frontend output intent" is intentionally avoided here. Frontend clients may send facts such as filename, MIME, size and range support, but they should not send final output directories or category routing decisions.

Required runtime flow:

```text
CreateTaskCommand factual fields
  -> SiteResolution enriches facts when a site parser can resolve more metadata
  -> ProtocolParser creates ExecutionPlan without choosing a final directory
  -> OutputCategoryPolicy classifies category from facts and parser result
  -> OutputDestinationPolicy joins daemon-owned category path with safe filename
  -> ExecutionPlan.output = final file path
  -> Task.path = final file path
  -> Downloader writes directly to Task.path
```

Important rule:

Do not implement this as:

```text
download to default folder -> move after completion
```

Reason:

- Resume manifests would point to the wrong final path.
- `TaskCompleted.output_path` would be harder to keep correct.
- Disk-space checks would be wrong.
- Partial failures would leave files in surprising places.
- It creates a temporary architecture that must be removed later.

Required cleanup:

- `output_naming` should produce filenames or relative file names, not hard-coded root directories.
- `output_destination` should join the selected category directory with the safe filename.
- Downloaders should use `task.path` or `ExecutionPlan.output`, not recompute hard-coded `downloads/...`.

Affected current files:

```text
src/parsers/policy/output_naming.rs
src/parsers/protocol/http_file.rs
src/parsers/protocol/hls.rs
src/parsers/protocol/dash.rs
src/parsers/protocol/youtube.rs
src/parsers/protocol/captured_merge.rs
src/core/execution_plan.rs
src/core/engine/executor.rs
src/core/engine/downloader.rs
src/packs/youtube_pack.rs
src/packs/bili_pack/dash_downloader.rs
```

### 4. Disk/Storage Stats API

Frontend capability:

- `StorageStats` displays free space for configured download path.

Current backend:

- No disk stats endpoint.
- Frontend currently mocks free space.

Required modules:

```text
src/protocol/storage.rs
src/services/storage_service.rs
src/gateway/http_api.rs
```

Recommended endpoints:

```text
GET /api/storage/paths
POST /api/storage/check
```

Possible DTO:

```rust
pub struct StoragePathStatus {
    pub path: String,
    pub exists: bool,
    pub writable: bool,
    pub available_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub error: Option<TaskError>,
}
```

Implementation notes:

- This is platform-sensitive; isolate filesystem calls inside `storage_service`.
- Do not put filesystem probing inside React.
- Do not log full sensitive paths unless needed; prefer sanitized logs.

### 5. Runtime Metrics API

Frontend capability:

- `ActiveThreads` and `GlobalSpeed` imply runtime metrics.

Current backend:

- Task speed exists per task.
- `max_task_num` exists.
- No API for current active chunk count, running task count, queue length, or total speed.

Required module:

```text
src/protocol/metrics.rs
src/services/runtime_metrics_service.rs
```

Recommended endpoint:

```text
GET /api/runtime/metrics
```

Possible DTO:

```rust
pub struct RuntimeMetrics {
    pub running_tasks: usize,
    pub queued_tasks: usize,
    pub total_speed_bytes_per_sec: u64,
    pub active_chunk_connections: Option<u32>,
    pub max_task_num: u32,
    pub max_chunk_connections: u32,
}
```

Development note:

- `GlobalSpeed` can initially sum task speeds from `GET /api/tasks`; that does not require this endpoint.
- `ActiveThreads` should only show true active threads/chunks after backend instrumentation exists.

### 6. Directory Picker / Tauri Boundary

Frontend capability:

- Settings modal currently uses browser directory picker fallback.

Current backend:

- No directory picker. That is fine; directory picking belongs to desktop shell capability, not daemon.

Required frontend/Tauri module later:

```text
src/platform/desktop-file-dialog.ts
```

Rule:

- Tauri can choose a directory.
- React sends the chosen path to daemon through `PATCH /api/settings`.
- Daemon validates and persists it.
- Daemon uses it for future task output paths.

Do not put download execution into Tauri commands.

## Recommended Development Order

### Phase 1: Protocol Client Without New Backend Features

Goal: connect what already exists.

Frontend:

```text
src/protocol/daemon.ts
src/api/daemon-client.ts
src/adapters/task-adapter.ts
src/effects/daemon-task-sync.ts
```

Wire:

- `GET /api/tasks`
- `POST /api/tasks`
- pause/resume/cancel
- task stream
- Bilibili QR APIs

Backend:

- Add CORS dev origins for `nexus-downloader` if needed.

### Phase 2: Settings API And Persistent Config

Goal: make settings real, but not yet change output destinations.

Backend:

```text
src/protocol/settings.rs
src/services/settings_service.rs
src/gateway/http_api.rs
```

Frontend:

```text
src/effects/settings-sync.ts
SettingsModal -> daemon settings
```

### Phase 3: Category And Destination Policy

Goal: user-selected category directories become actual first-write destinations.

Backend:

```text
src/parsers/policy/output_category.rs
src/parsers/policy/output_destination.rs
src/parsers/policy/output_naming.rs
```

Then update parsers and downloaders so `ExecutionPlan.output` / `Task.path` is the single final output path.

Verification:

- Bilibili task writes directly to video directory.
- YouTube task writes directly to video directory.
- X/HLS task writes directly to video directory.
- PDF writes directly to document directory.
- ZIP writes directly to archive directory.
- `TaskCompleted.output_path` equals the real final path.
- Resume manifest still points to the correct final output.

### Phase 4: Storage Stats And Runtime Metrics

Goal: remove remaining mock dashboard data.

Backend:

```text
src/services/storage_service.rs
src/services/runtime_metrics_service.rs
src/protocol/storage.rs
src/protocol/metrics.rs
```

Frontend:

```text
StorageStats -> storage API
ActiveThreads -> runtime metrics
GlobalSpeed -> either task sum or runtime metrics
```

### Phase 5: Tauri Desktop Capabilities

Goal: add desktop shell features after protocol connection is stable.

Frontend/Tauri:

- Directory picker.
- Tray.
- Native notifications.
- Open file/folder commands if desired.

Rule:

All durable state still goes through daemon protocol unless it is purely UI-local.

## What Not To Do

- Do not let React decide final daemon output paths by itself.
- Do not download to a temporary common folder and move by category after completion.
- Do not make Tauri commands bypass daemon for task creation or download execution.
- Do not store daemon-backed settings only in frontend local storage.
- Do not duplicate backend classification rules in multiple components.
- Do not expose private Rust task structs directly to the frontend.

## Summary

The frontend already contains a good product skeleton. The missing backend work is not "more UI"; it is mainly:

1. persistent settings API,
2. output category classification,
3. output destination policy,
4. storage stats,
5. optional runtime metrics.

The highest-priority backend architecture fix is making final output path a single source of truth before download starts. That is what makes per-category download directories clean instead of becoming a later file-moving workaround.
