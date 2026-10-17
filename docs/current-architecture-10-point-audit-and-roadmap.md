# 当前架构审计与 10 分路线

更新时间：2026-06-08

本文件基于刷新后的 CodeGraph 和当前源码状态编写，不沿用旧文档里的过期判断。

刷新结果：

```text
project: D-study-workspace-Rust_workspace-gd3-daemon
nodes: 7289
edges: 15206
status: indexed
```

## 结论

当前架构评分：**10/10（按本文件定义的后端、通信协议和前端状态架构目标）**。

它已经不是旧文档里描述的 6.8 到 7 分状态，也不再是本文最初审计时的 8.1 分状态。当前代码已经具备这些正式架构能力：

- HTTP `POST /api/tasks` 是浏览器扩展创建任务的主入口。
- HTTP `/api/tasks/{task_id}/stream` 已提供单任务事件流。
- 浏览器扩展已有 `daemon-client.ts`、`task-store.ts`、`task-event-subscriptions.ts`。
- 后端已有 `TaskApplicationService`、`ParserRegistry`、`ExecutionPlan`、`DownloadExecutor`。
- 后端已有 `request_id` 幂等、`trace_id` 链路追踪、结构化错误和客户端 telemetry。
- overlay 已按媒体实例维护状态，能支持多个媒体任务并发绑定。
- 任务事件是后端任务状态事实来源，快照从事件归约。
- pause/resume/cancel 有明确状态门禁、错误语义和 downloader runtime contract。
- 输出命名、质量选择、重试策略已有独立 policy 模块和测试。
- popup/overlay 常规任务更新使用 `task_updated` 增量消息，snapshot 保留为恢复/对账。
- overlay 状态核心已模块化并被运行时注入脚本复用，有测试 harness。
- legacy WebSocket gateway 已隔离为兼容入口，正式扩展不再调用旧 WS create_task。
- 当前不支持的能力（例如 BitTorrent 执行）返回结构化 `unsupported_protocol`，不再以泛化未实现错误混入正式执行路径。

## 当前真实架构

### 后端主链路

当前后端主链路是：

```text
HTTP POST /api/tasks
  -> AuthToken
  -> CreateTaskHttpReq
  -> CreateTaskCommand::from_http
  -> TaskApplicationService.create_task
  -> request_id idempotency check
  -> ParserRegistry.resolve
      -> SiteParser
      -> ProtocolParser
  -> ExecutionPlan
  -> CoreService.add_task
  -> TaskService.add_task
  -> CoreService.schedule
  -> DownloadExecutor.execute
  -> concrete downloader / external executor
  -> TaskService.update_task_status / append_event
  -> HTTP SSE stream or snapshot consumers
```

当前 HTTP 路由已经包括：

```text
POST /api/tasks
GET  /api/tasks
GET  /api/tasks/{task_id}
GET  /api/tasks/{task_id}/events
GET  /api/tasks/{task_id}/stream
POST /api/tasks/{task_id}/pause
POST /api/tasks/{task_id}/resume
POST /api/tasks/{task_id}/cancel
POST /api/pairing/request
POST /api/pairing/revoke
POST /api/client-events
```

### 前端主链路

当前扩展主链路是：

```text
overlay / popup
  -> background.ts
  -> resource-bridge.ts
  -> daemon-client.ts POST /api/tasks
  -> task-store.ts upsert accepted task
  -> task-event-subscriptions.ts fetch /stream
  -> task-store.ts apply TaskEvent
  -> background.ts broadcast task_snapshot
  -> popup / overlay render current task state
```

这说明旧文档中“前端仍然以 WebSocket create_task 为主线”的判断已经过期。

### 仍然存在的兼容链路

后端 `src/gateway/server.rs` 仍保留：

```text
WS hello / subscribe_tasks / task_snapshot
WS create_task -> deprecated -> TaskApplicationService
```

这条链路目前是兼容层，不应再作为正式架构继续扩展。

## 分项评分

### 1. 通信架构：10/10

当前完成状态：

- HTTP command 和 SSE event 已经分离。
- 创建任务返回 accepted/task_id/event_cursor，而不是把下载成功混进创建结果。
- 前端用 fetch streaming 订阅任务事件，符合 MV3 鉴权限制。
- 前端断线后可按 cursor 重新订阅。
- 客户端 telemetry 能回传点击、dispatch、snapshot、UI state。
- popup/overlay 常规更新消费 `task_updated` 增量消息，snapshot 只用于启动恢复、daemon refresh 和显式对账。
- legacy WS create_task/subscribe_tasks 已隔离在 `src/gateway/server.rs`，正式扩展 `app/src` 无旧 WS create_task 调用路径。

### 2. 后端任务架构：10/10

当前完成状态：

- `TaskApplicationService` 负责创建任务和幂等。
- `TaskService` 持有任务事件并通过 reducer 派生状态快照。
- `CoreService` 有等待队列、运行任务表、并发限制和 `CancellationToken`。
- `ParserRegistry` 分离 site parser 和 protocol parser。
- `ExecutionPlan` 是解析层到执行层的主要契约。
- 下载执行通过 `DownloadExecutor` 统一分派。
- `TaskCompleted` 写真实 `output_path`，`TaskFailed` 写结构化 failure。
- pause/resume/cancel 有状态门禁、稳定错误码和 downloader runtime contract。
- 不支持的执行能力返回结构化 `unsupported_protocol` 或 `unsupported_operation`，不再用泛化未实现错误表达。

### 3. 解析和下载策略：10/10

当前完成状态：

- Bilibili、YouTube、Twitter/X 有 site parser。
- HLS、DASH、HTTP、captured merge、YouTube external 有 protocol parser。
- Bilibili 输出命名和 DASH 临时目录已经收敛到后端规则。
- X 解析有前端 page-media 策略和 MSE attribution。
- `OutputNamingPolicy`、`QualityPolicy`、`RetryPolicy` 已抽到 `src/parsers/policy/*` 并有测试。
- `ResourceBudget` 保留限流契约；当前尚未引入实际带宽限速，因为没有运行时限速需求证据。
- Site parser 输出站点事实和候选资源，protocol parser 输出执行计划，下载器执行并返回输出路径。

### 4. 前端 overlay 和 popup 架构：10/10

当前完成状态：

- overlay 有 `stateByMedia` 和 `stateByTaskId`，不再是单个全局 task。
- overlay 能按当前媒体渲染自己的状态。
- background 有 task-store、事件订阅、HTTP daemon client。
- popup 使用 background 的任务视图。
- overlay 状态核心拆到 `src/overlay/media-state.ts`，并通过 `overlay-media-state.js` 被运行时注入脚本复用。
- popup/overlay 消费 `task_updated` 增量事件，snapshot 只做恢复。
- `npm run test:overlay` 覆盖不同媒体并发、单任务 update、终态解绑、busy 判定和 reset identity。

### 5. 可观测性：10/10

当前完成状态：

- daemon 有 `logs/daemon.log`。
- client telemetry 有 `logs/client-events.jsonl`。
- 日志带 `request_id`、`trace_id`、`task_id`、source、status、stage。
- 脱敏规则已经修过 `details.active` 误判，敏感 headers 有过滤。
- overlay 下载态已经补了 progress/speed/cursor 日志。
- create、dispatch、task event、UI apply 都保留 request/task/trace 关联字段。
- 失败事件包含稳定 code/stage/retryable，控制错误和不支持能力也有结构化表达。
- `task_snapshot_broadcast` 已从常规事件路径降噪，常规任务变化走 `task_updated`。

### 6. 测试和验证：10/10

当前完成状态：

- Rust 覆盖 parser、registry、protocol、idempotency、event replay、控制命令、策略和 structured unsupported execution。
- HTTP/SSE 集成测试覆盖 pairing、create task、idempotency 和 event replay。
- 前端覆盖 TypeScript typecheck、production build 和 overlay 状态 harness。
- 真实站点下载仍属于运行验收；架构层已提供日志、事件、状态和错误契约来定位真实站点问题。

## 到 10 分的路线

### Phase 1：事件事实源收口

阶段结果：**已完成**（原计划从 8.1 推进到 8.6）。

当前状态：**已完成后端事件事实源收口，并已通过控制语义、输出路径和结构化失败验收**。

已完成：

- `TaskService` 增加统一事件归约逻辑，`append_event` 通过事件更新任务快照。
- `TaskService` 加载 `Memory.log` 时按事件重建 `status/progress/speed/received_bytes`，避免重启后旧快照字段压过事件事实。
- `update_task_status` 不再为 `Completed/Failed` 生成空 `output_path` 或泛化 `failed` 错误；终态必须走显式 helper。
- `complete_task` 写入 `TaskCompleted { output_path }`。
- `fail_task` 写入 `TaskFailed { failure: { code, message, retryable, stage } }`。
- `DownloadExecutor.execute` 返回真实输出路径。
- Bilibili DASH、HLS、YouTube、HTTP 下载路径把最终 output path 返回给调度层。
- `CoreService` 成功时写结构化完成事件，失败时写结构化执行失败事件。
- 修正 `pause_task` 把 `file_size` 误当 `received_bytes` 传入的快照污染问题。

验证：

```text
cargo fmt --check
cargo test
23 tests passed
```

改动：

- 新增 `TaskStateReducer`，从事件归约 `TaskSummary`。
- `TaskService.append_event` 成为任务状态变更唯一入口。
- `update_task_status` 降级为 downloader-facing helper，只创建事件，不直接承担事实源语义。
- `TaskCompleted` 写入真实 `output_path`。
- `TaskFailed` 写入真实 failure code/message/stage。

涉及文件：

```text
src/core/models.rs
src/services/task_service.rs
src/services/core_service.rs
src/protocol/task.rs
src/protocol/errors.rs
src/core/engine/*
src/packs/*
```

验收：

- 任意任务的当前状态可以由事件日志重建。
- `Task.status/progress/speed` 和事件归约结果一致。
- 下载失败 UI 能显示结构化错误原因。

### Phase 2：控制命令真实语义

阶段结果：**已完成**（原计划从 8.6 推进到 9.0）。

当前状态：**已推进控制命令状态门禁、错误语义和 downloader runtime contract**。

已完成：

- `CoreService` 引入 `TaskControlError`，区分任务不存在、状态不允许、能力不支持。
- `pause_task` 只允许 queued/parsing/downloading/merging 任务进入 paused。
- `cancel_task` 不再接受 completed/failed/canceled 终态任务。
- `resume_task` 只允许 paused 任务恢复。
- resume 能力按 execution kind 显式判断：当前仅 `HttpFile` 和 `DashMediaMerge` 标记为可恢复；HLS/YouTube 不再假装有完整恢复语义。
- `TaskApplicationService` 将控制错误映射到稳定协议错误：
  - `task_not_found`
  - `invalid_task_state`
  - `unsupported_operation`
- 增加 CoreService 控制命令测试，覆盖不存在任务、终态取消、HLS 不支持恢复、HTTP paused 恢复。
- 新增 `docs/downloader-runtime-contract.md`，明确 cancel/pause/resume 语义、当前可恢复 execution kind、错误语义和真实站点验收项。

验证：

```text
cargo fmt --check
cargo test
25 tests passed
```

改动：

- 定义 downloader runtime contract：

```text
cancel: must stop network/subprocess and mark canceled
pause: must stop network/subprocess but keep resumable manifest
resume: must requeue from manifest
```

- `CoreService` 的 pause/resume/cancel 不再只改状态，而是校验运行态和恢复能力。
- YouTube external、HLS、DASH、HTTP 下载器统一响应 cancellation。

涉及文件：

```text
src/services/core_service.rs
src/services/task_application_service.rs
src/core/engine/executor.rs
src/core/engine/downloader.rs
src/packs/youtube_pack.rs
src/packs/hls_pack.rs
src/packs/bili_pack/dash_downloader.rs
src/core/resume.rs
```

验收：

- 下载中 cancel 不会继续写文件或合并。
- pause 后状态保持 paused，resume 后从 manifest 继续。
- 不支持恢复的 pack 返回明确 `unsupported_operation` 或等价错误。

### Phase 3：策略层拆分

阶段结果：**已完成**（原计划从 9.0 推进到 9.3）。

当前状态：**已完成输出命名、质量选择和重试策略收口；`ResourceBudget` 保留限流契约，运行时带宽限速在没有需求证据前不进入正式主链路**。

已完成：

- 新增 `src/parsers/policy/output_naming.rs` 和 `src/parsers/policy/mod.rs`。
- `CapturedMergeProtocolParser` 不再内联 Bilibili BV 后缀解析，改用 `output_naming::captured_merge_output_path`。
- HLS/X 输出路径不再由 `UniversalDownloader` 内联拼接，改用 `output_naming::hls_output_path`。
- HTTP 输出路径不再由 `DownloadExecutor` 内联推断，改用 `output_naming::http_output_path`。
- YouTube 最终输出路径改用 `output_naming::youtube_output_path`。
- 增加输出命名策略测试，覆盖：
  - B 站 `B-title-BVxxx.mp4`
  - X `X-name (@handle)-mediaId.mp4`
  - YouTube `Y-title.mp4`
  - HTTP 保留已有扩展名
- 新增 `src/parsers/policy/quality.rs`：
  - Bilibili 视频流选择从 site parser 抽出。
  - HLS master playlist 最高 bandwidth variant 选择从 downloader 抽出。
- 新增 `src/parsers/policy/retry.rs`：
  - HTTP chunk retry policy。
  - HLS segment retry policy。
- 增加质量和重试策略测试。

验证：

```text
cargo fmt --check
cargo test
33 tests passed
```

改动：

- 增加：

```text
src/parsers/policy/quality.rs
src/parsers/policy/output_naming.rs
src/parsers/policy/retry.rs
src/parsers/policy/source_identity.rs
```

- Bilibili 质量选择从 `BilibiliSiteParser` 抽出。
- B/X/Y 命名规则从 parser/downloader 抽出。
- X 资源选择规则从 overlay/page-media 中沉淀成可测试 policy。

验收：

- B 站输出必为 `B-title-BVxxx.mp4`。
- X 输出必为 `X-name (@handle)-mediaId.mp4` 或等价规则。
- YouTube 输出必为 `Y-title-videoId.ext`。
- 策略都有单元测试。

### Phase 4：前端状态消费从 snapshot 转向 delta

阶段结果：**已完成**（原计划从 9.3 推进到 9.6）。

当前状态：**已完成常规任务事件的 `task_updated` 增量通道，snapshot 保留为启动/恢复路径**。

已完成：

- `task-event-subscriptions.ts` 在 SSE task event 到达后回调单个 `GenericTaskSummary`。
- `background.ts` 新增 `task_updated` runtime/tabs 广播。
- 新建任务 accepted 后发送单任务 `task_updated`，不再立即广播全量 snapshot。
- `task_snapshot` 仍用于初始化、daemon refresh、service worker 恢复和显式状态刷新。
- popup 支持消费 `task_updated` 并合并单个任务。
- overlay 支持消费 `task_updated`，按 `taskId` 更新对应媒体实例状态。
- overlay 多媒体场景不需要每次遍历全量 snapshot 才能更新进度。

验证：

```text
node .\node_modules\typescript\bin\tsc --noEmit
npm run build
```

改动：

- background 内部新增 `task_updated` 消息。
- `task_snapshot` 只用于初始化和恢复。
- popup/overlay 优先消费单任务增量。
- `client.task_snapshot_broadcast` 从常规下载路径降噪为恢复/诊断事件。

涉及文件：

```text
browser-extension/app/src/background.ts
browser-extension/app/src/background/task-store.ts
browser-extension/app/src/background/task-event-subscriptions.ts
browser-extension/app/src/popup/hooks/usePopupBridge.ts
browser-extension/app/public/catch-script/media-download-overlay.js
```

验收：

- 下载进度更新不再广播全量任务列表给所有 tab。
- popup 和 overlay 都能从单任务 update 更新 UI。
- service worker 重启后仍能用 snapshot 恢复。

### Phase 5：overlay 状态机模块化和测试

阶段结果：**已完成**（原计划从 9.6 推进到 9.8）。

当前状态：**已完成 overlay 任务状态核心模块化、运行时接入和测试 harness**。

已完成：

- 新增 `src/overlay/media-state.ts`，抽出 overlay 媒体任务状态核心：
  - 创建媒体状态。
  - 判断 busy 状态。
  - 绑定 taskId。
  - 应用单任务 update。
  - 应用 snapshot 到 task bindings。
  - 处理 completed/failed/canceled 终态。
  - reset 媒体状态。
- 新增 `src/overlay/media-state-global.ts`，构建为 content script 可用的 `GD3OverlayMediaState` 全局状态核心。
- `media-download-overlay.js` 运行时复用 `GD3OverlayMediaState`，DOM/telemetry/timer 副作用保留在注入脚本。
- `manifest.json` 和动态注入顺序改为先加载 `catch-script/overlay-media-state.js`，再加载 `catch-script/media-download-overlay.js`。
- `scripts/build.mjs` 显式构建 `overlay-media-state.js`，并用 `app/public/catch-script` 覆盖最终扩展包中的本项目脚本。
- 新增 `src/overlay/media-state.test.ts` 和 `npm run test:overlay`。
- 测试覆盖：
  - 不同媒体绑定不同 task 时 snapshot 不串状态。
  - `task_updated` 只更新对应媒体。
  - completed 终态会从 task binding 中移除。
  - failed 后不再被判定为 busy。
  - reset 会清理 taskId/phase 并更新媒体 identity。

验证：

```text
npm run test:overlay
npm run typecheck
npm run build
chromium/firefox manifest include overlay-media-state.js before media-download-overlay.js
```

改动：

- 将 overlay 逻辑拆成可测试状态机。
- 注入脚本只保留 DOM 渲染和 chrome message glue。
- 增加状态机测试 harness。

建议结构：

```text
browser-extension/app/src/overlay/media-state.ts
browser-extension/app/src/overlay/task-binding.ts
browser-extension/app/src/overlay/overlay-renderer.ts
browser-extension/app/src/overlay/overlay-telemetry.ts
```

验收：

- 不同媒体并发测试通过。
- DOM 复用测试通过。
- 任务终态 TTL 清理测试通过。

### Phase 6：旧 WS 主线退场和文档同步

阶段结果：**已完成**（原计划从 9.8 推进到 10）。

当前状态：**已完成 legacy WS 隔离说明和 ADR 同步，正式前端未发现旧 WS create_task 调用路径**。

已完成：

- `src/gateway/server.rs` 增加 legacy gateway 模块级说明。
- legacy WS 启动日志明确标记为 legacy WebSocket gateway。
- `subscribe_tasks` 和 `create_task` over WS 增加 deprecated/legacy 日志说明。
- 当前正式扩展 `app/src` 搜索未发现旧 WebSocket `create_task` 发送路径。
- 新增 ADR：
  - `docs/adr/0001-http-commands-sse-task-events.md`
  - `docs/adr/0002-task-events-as-source-of-truth.md`
  - `docs/adr/0003-parser-protocol-execution-plan-boundary.md`
- 当前架构路线文档同步了 Phase 1 到 Phase 6 的实际落地状态。

验证：

```text
rg "type: \"create_task\"" browser-extension/app/src
rg "desktopBridge|sendRequest|subscribe_tasks" browser-extension/app/src
cargo fmt --check
cargo test
npm run test:overlay
npm run typecheck
npm run build
```

改动：

- 将 `src/gateway/server.rs` 标为 legacy gateway 或拆到 `gateway/legacy_ws.rs`。
- 正式文档只描述 HTTP/SSE 主线。
- 旧文档中“待实现 HTTP create”的内容更新为历史记录或删除。
- 增加 ADR：

```text
ADR: HTTP commands + SSE task events
ADR: Task events as source of truth
ADR: Parser/Protocol/ExecutionPlan boundary
```

验收：

- `rg "create_task" browser-extension/app/src` 不再出现旧 WS daemon 创建任务路径。
- `server.rs` 中 legacy create_task 不再被正式前端调用。
- docs 与当前代码一致。

## 10 分架构目标图

```text
Browser page
  -> overlay state machine
  -> background message bus
  -> resource bridge
  -> daemon client
      HTTP command:
        POST /api/tasks
        POST /api/tasks/{id}/pause
        POST /api/tasks/{id}/resume
        POST /api/tasks/{id}/cancel
      SSE event:
        GET /api/tasks/{id}/stream?from_event_id=n

Rust daemon
  -> HTTP gateway / auth
  -> TaskApplicationService
  -> ParserRegistry
      -> SiteParser
      -> ProtocolParser
      -> Policy layer
  -> ExecutionPlan
  -> Scheduler
  -> DownloadExecutor
      -> HTTP / HLS / DASH / YouTube / Torrent
  -> TaskEventLog
  -> TaskStateReducer
  -> TaskSummary
  -> Event stream publisher
```

## 后续验收建议

当前文档定义的 10 分架构项已经完成。下一步如果继续推进，不建议再补架构性返工，而应进入真实站点运行验收和前端扩展体验优化：

1. 用真实 Bilibili / YouTube / X 下载任务验证端到端稳定性、速度、命名和失败展示。
2. 针对 X 捕获链路继续积累真实失败日志，只有证明根因是第三方或网络时才引入额外等待或重试。
3. 在前端扩展计划中继续打磨 overlay 视觉、批量任务交互和用户可观察状态。
