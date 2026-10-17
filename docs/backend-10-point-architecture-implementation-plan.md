# 后端 10 分架构实现方案

## 目标

把 Rust daemon 后端改造成模块边界清晰、按需执行、低常驻资源、可恢复、可观测、可长期扩展的下载服务。

目标形态：

```text
HTTP Gateway
  -> TaskApplicationService
  -> ParserRegistry
       -> SiteParsers
       -> ProtocolParsers
  -> ExecutionPlan
  -> TaskScheduler
  -> DownloadExecutor
       -> HttpDownloader
       -> HlsDownloader
       -> DashDownloader
       -> TorrentExecutor
       -> MergeExecutor
  -> TaskStore + EventStore
  -> Progress/Event Publisher
```

核心原则：

- 常驻层只负责监听、鉴权、任务存储、调度和事件发布。
- 用户创建任务后，才按需调用对应站点解析器、协议解析器和下载执行器。
- Bilibili、YouTube、Twitter/X、BitTorrent、HLS、DASH、HTTP 等能力必须分模块维护。
- 解析器只解析，不下载；下载器只执行计划，不理解站点业务。
- WebSocket/SSE 只是任务事件观察通道，不是任务生命线。
- 下载任务断点恢复、暂停、取消、失败重试都必须是正式能力，不是临时补丁。

---

## 当前后端架构评分

当前 Rust 后端已有较好的雏形：

- `CoreService`
- `TaskService`
- `PackRegistry`
- `FeaturePack`
- `ExecutionKind`
- `DownloadExecutor`
- `UniversalDownloader`
- Bilibili / YouTube / HLS / HTTP / BitTorrent / GenericMerge 等 pack

当前评分：**6.8/10 到 7/10**。

主要优点：

- 已有任务服务和调度服务。
- 已有 pack 注册与匹配机制。
- 已有统一执行器雏形。
- 已有分块下载和部分断点续传基础。

主要扣分：

- HTTP 创建任务还没有成为正式主入口。
- WebSocket gateway 仍承担创建任务。
- 站点解析和协议解析边界不够清晰。
- Twitter/X 目前更像 HLS/twimg 规则兜底，不是独立站点解析器。
- 执行计划、断点恢复、任务事件、错误类型、取消暂停恢复还不够正式。
- 任务运行时资源生命周期没有被统一建模。

目标评分：**10/10**。

---

## 10 分后端的模块边界

### 1. Gateway 层

职责：

- HTTP 路由。
- 鉴权和来源校验。
- 请求体验证。
- 生成或透传 `request_id`。
- 调用应用服务。
- 返回结构化响应。

不负责：

- 站点解析。
- 下载。
- 合并。
- 任务状态拼装。
- 直接操作 pack。

建议模块：

```text
src/gateway/http/
src/gateway/events/
src/gateway/auth/
```

### 2. Protocol 类型层

职责：

- 定义跨前后端请求、响应、事件、错误码。
- 定义任务状态和阶段枚举。
- 定义创建任务 payload。

建议模块：

```text
src/protocol/task.rs
src/protocol/events.rs
src/protocol/errors.rs
```

要求：

- 不用裸 `serde_json::Value` 承载主协议。
- 不用模糊 `ok` / `success` 表示多层语义。
- 对外字段统一命名风格，建议 JSON 使用 `snake_case`。

### 3. TaskApplicationService

职责：

- 创建任务。
- 幂等处理。
- 调用解析器注册表。
- 生成任务和初始事件。
- 调用调度器入队。
- 处理暂停、恢复、取消。

建议模块：

```text
src/services/task_application_service.rs
```

核心接口：

```rust
create_task(request: CreateTaskCommand) -> Result<CreateTaskAccepted, TaskCreateError>
pause_task(task_id: TaskId) -> Result<TaskCommandResult, TaskCommandError>
resume_task(task_id: TaskId) -> Result<TaskCommandResult, TaskCommandError>
cancel_task(task_id: TaskId) -> Result<TaskCommandResult, TaskCommandError>
```

### 4. ParserRegistry

职责：

- 管理站点解析器和协议解析器。
- 按优先级匹配。
- 输出标准 `ExecutionPlan`。
- 记录匹配过程，方便排查。

建议模块：

```text
src/parsers/registry.rs
src/parsers/site/
src/parsers/protocol/
```

### 5. SiteParser

职责：

- 识别具体站点。
- 处理站点 API、页面结构、cookie、referer、签名等站点特有逻辑。
- 输出标准资源候选或直接输出中间解析结果。

建议模块：

```text
src/parsers/site/bilibili.rs
src/parsers/site/youtube.rs
src/parsers/site/twitter.rs
```

要求：

- Bilibili 解析器只处理 B 站。
- YouTube 解析器只处理 YouTube。
- Twitter/X 解析器只处理 Twitter/X。
- 不要让站点解析器直接下载最终文件。
- 不要把站点逻辑塞进 HLS 或 HTTP 兜底解析器。

### 6. ProtocolParser

职责：

- 识别资源协议。
- 把 URL 或资源清单转成标准执行阶段。
- 不关心资源来自哪个站点。

建议模块：

```text
src/parsers/protocol/hls.rs
src/parsers/protocol/dash.rs
src/parsers/protocol/http_file.rs
src/parsers/protocol/bittorrent.rs
src/parsers/protocol/captured_merge.rs
```

### 7. ExecutionPlan

职责：

- 作为解析层和执行层之间唯一契约。
- 描述如何下载、如何恢复、如何合并、如何落盘。

建议模块：

```text
src/core/execution_plan.rs
```

建议结构：

```rust
pub struct ExecutionPlan {
    pub task_id: TaskId,
    pub title: String,
    pub output: OutputTarget,
    pub kind: ExecutionKind,
    pub stages: Vec<ExecutionStage>,
    pub resume_policy: ResumePolicy,
    pub merge_policy: MergePolicy,
    pub resource_budget: ResourceBudget,
}
```

### 8. TaskScheduler

职责：

- 等待队列。
- 运行队列。
- 并发限制。
- 任务取消、暂停、恢复。
- 调用 `DownloadExecutor`。
- 任务结束后释放运行资源。

要求：

- 下载任务用受控 tokio task 或 worker 执行。
- 常驻控制面保持轻量。
- 每个运行任务必须有 `CancellationToken`。
- 每个运行任务必须能释放自身资源。

### 9. DownloadExecutor

职责：

- 只接收 `ExecutionPlan`。
- 根据 `ExecutionKind` 分派到具体执行器。
- 将执行过程转成任务事件。
- 不理解 Bilibili、YouTube、Twitter/X 等站点规则。

建议模块：

```text
src/core/engine/executor.rs
src/core/engine/http_downloader.rs
src/core/engine/hls_downloader.rs
src/core/engine/dash_downloader.rs
src/core/engine/torrent_executor.rs
src/core/engine/merge_executor.rs
```

### 10. TaskStore + EventStore

职责：

- 任务当前状态。
- 任务持久化。
- 任务事件追加。
- 断点恢复 manifest。
- 查询任务和事件。

建议模块：

```text
src/services/task_store.rs
src/services/event_store.rs
src/services/resume_store.rs
```

---

## 创建任务主链路

目标流程：

```text
POST /api/tasks
  -> Gateway 验证 token 和请求体
  -> TaskApplicationService.create_task
  -> IdempotencyStore 检查 request_id
  -> ParserRegistry.resolve
  -> SiteParser 可选解析
  -> ProtocolParser 生成 ExecutionPlan
  -> TaskStore 新增 task
  -> EventStore 追加 task_accepted / task_queued
  -> TaskScheduler.enqueue
  -> 返回 task_id + event_cursor
```

注意：

- `create_task` 成功只表示任务被接受，不表示下载成功。
- 解析失败、站点不支持、鉴权缺失、资源缺失必须返回结构化错误。
- 如果解析阶段耗时较长，可以先创建 `parsing` 任务，再异步解析；但状态必须清楚。

---

## 解析器匹配策略

推荐两级匹配：

```text
1. SiteParserRegistry
   - Bilibili
   - YouTube
   - Twitter/X
   - GenericPage

2. ProtocolParserRegistry
   - DASH
   - HLS
   - HTTP file
   - BitTorrent
   - Captured merge
```

匹配规则：

- 每个解析器声明 `id`、`priority`、`capabilities`。
- 高优先级站点解析器先匹配。
- 站点解析器输出真实资源后，再进入协议解析器。
- HTTP 兜底解析器永远最后执行。
- 每次匹配必须记录结构化日志。

示例：

```text
BilibiliSiteParser priority 100
YouTubeSiteParser priority 100
TwitterSiteParser priority 100
CapturedMergeProtocolParser priority 80
DashProtocolParser priority 70
HlsProtocolParser priority 60
BitTorrentProtocolParser priority 50
HttpFileProtocolParser priority 0
```

---

## 执行计划契约

`ExecutionPlan` 是核心边界。解析器只能输出计划，执行器只能消费计划。

建议字段：

```rust
pub enum ExecutionKind {
    HttpFile,
    Hls,
    DashMediaMerge,
    CapturedMediaMerge,
    YouTubeExternal,
    BitTorrent,
}

pub struct ExecutionStage {
    pub stage_id: String,
    pub kind: StageKind,
    pub url: String,
    pub headers: HeaderMap,
    pub expected_size: Option<u64>,
    pub checksum: Option<Checksum>,
    pub output_part: OutputPart,
}

pub enum MergePolicy {
    None,
    FfmpegMux { video_stage: String, audio_stage: String },
    ConcatSegments,
}

pub enum ResumePolicy {
    Disabled,
    Chunked { chunk_size: u64 },
    ManifestBased,
}
```

要求：

- `ExecutionPlan` 不包含解密后的 cookies 明文持久化。
- 每个 stage 的 headers 只作用于该资源。
- 输出路径必须提前规范化，防止路径穿越。

---

## 断点恢复设计

断点恢复必须有正式 manifest，而不是只检查 chunk 文件是否存在。

建议 manifest：

```json
{
  "task_id": "task_xxx",
  "source_fingerprint": "sha256:...",
  "output_path": "D:/Downloads/video.mp4",
  "total_size": 123456789,
  "created_at": "...",
  "updated_at": "...",
  "chunks": [
    {
      "index": 0,
      "start": 0,
      "end": 20971519,
      "path": "temp/task_xxx/chunk_000.part",
      "status": "completed",
      "bytes_written": 20971520,
      "checksum": null
    }
  ]
}
```

要求：

- 任务启动时先加载 manifest。
- chunk 完成后原子更新 manifest。
- 应用重启后可扫描未完成任务。
- 资源 URL、headers 指纹或 total size 变化时，不能盲目复用旧 chunk。
- 完成后清理 temp 和 manifest，或归档用于审计。

---

## 任务生命周期和资源释放

每个任务运行时必须有明确生命周期：

```text
created
queued
parsing
downloading
merging
completed / failed / canceled
cleanup
```

运行时资源：

- HTTP client 上下文。
- cookie 解密结果。
- temp chunk 文件。
- ffmpeg / yt-dlp 子进程。
- torrent session。
- event subscription。
- semaphore permit。
- cancellation token。

要求：

- 任务完成、失败、取消后必须释放运行资源。
- 取消任务必须能通知下载器、合并器、外部子进程。
- 失败时要保留足够恢复信息，但不要保留敏感数据。

---

## 调度和并发控制

调度器必须支持：

- 全局最大并发任务数。
- 单任务最大连接数。
- 站点级限流。
- 下载速度限制。
- 暂停。
- 恢复。
- 取消。
- 重试策略。

建议结构：

```rust
pub struct RuntimeTaskHandle {
    pub task_id: TaskId,
    pub cancellation_token: CancellationToken,
    pub pause_token: PauseToken,
    pub resource_budget: ResourceBudget,
}
```

要求：

- 不要无限 `tokio::spawn`。
- 每个 spawn 必须能被追踪、取消和回收。
- 运行队列移除后要触发下一次调度。

---

## 错误模型

必须定义结构化错误类型。

建议错误码：

```text
unsupported_site
unsupported_protocol
parse_failed
auth_required
token_invalid
cookie_decryption_failed
http_403
http_404
range_not_supported
content_length_missing
network_timeout
retry_exhausted
disk_full
write_failed
merge_failed
external_tool_missing
external_tool_failed
torrent_metadata_failed
canceled
paused
internal_error
```

要求：

- 对用户展示 `message`。
- 对程序判断使用稳定 `code`。
- 对重试判断使用 `retryable`。
- 对定位问题记录 `stage`。
- 日志中不记录 cookies、token、密码、敏感 headers。

---

## 任务事件

后端应以事件驱动状态同步。

推荐事件：

```text
task_accepted
task_queued
task_parsing_started
task_plan_created
task_download_started
task_progress_updated
task_merge_started
task_completed
task_failed
task_canceled
task_paused
task_resumed
```

事件字段：

```text
event_id
task_id
request_id
type
occurred_at
stage
progress
failure_code
message
```

要求：

- `event_id` 单任务内递增。
- 下载进度事件要节流。
- 终态事件发送后，前端订阅可以关闭。

---

## 低内存常驻设计

正确目标不是“物理上只有一个线程”，而是“常驻资源少、任务资源按需创建和释放”。

常驻资源：

- HTTP listener。
- event listener。
- scheduler。
- task store。
- event store。
- parser registry 中的轻量 parser 对象。

按任务创建的资源：

- 下载连接。
- 大 buffer。
- ffmpeg / yt-dlp 进程。
- torrent session。
- chunk manifest 写入上下文。
- cookie 解密结果。

说明：

- Rust 中轻量 parser 常驻通常不是内存瓶颈。
- 真正需要严格生命周期管理的是下载 buffer、并发连接、临时文件、外部进程和 torrent session。

---

## 实施阶段

### Phase 1：定义正式协议和后端类型

输出：

- `CreateTaskCommand`
- `CreateTaskAccepted`
- `TaskStatus`
- `TaskEvent`
- `TaskError`
- `ExecutionPlan`
- `ExecutionStage`
- `ResumePolicy`
- `MergePolicy`

验收：

- 类型可序列化。
- 单元测试覆盖 JSON round-trip。
- 不再为主链路新增裸 `serde_json::Value`。

### Phase 2：引入 TaskApplicationService

输出：

- `create_task`
- `pause_task`
- `resume_task`
- `cancel_task`

验收：

- HTTP gateway 只调用 service。
- WebSocket 旧入口如保留，必须转调同一 service，并标记 deprecated。

### Phase 3：拆分 ParserRegistry

输出：

- `SiteParser` trait。
- `ProtocolParser` trait。
- Bilibili site parser。
- YouTube site parser。
- Twitter/X site parser。
- HLS protocol parser。
- DASH protocol parser。
- HTTP file protocol parser。
- BitTorrent protocol parser。

验收：

- Twitter/X 不再只是 HLS pack 中的字符串匹配。
- 每个 parser 有独立单元测试。
- 匹配顺序可预测、可日志追踪。

### Phase 4：ExecutionPlan 成为唯一执行契约

输出：

- 所有 parser 输出 `ExecutionPlan`。
- `DownloadExecutor` 只接收 `ExecutionPlan`。

验收：

- 执行器不依赖站点名称判断下载方式。
- Bilibili、YouTube、Twitter/X、HLS、HTTP、Torrent 都能映射到明确 `ExecutionKind`。

### Phase 5：正式断点恢复

输出：

- `ResumeManifest`。
- `ResumeStore`。
- chunk 原子更新。
- 启动恢复扫描。

验收：

- 中断下载后重启 daemon 能恢复或明确标记为可恢复失败。
- URL/headers/size 不匹配时不会错误复用旧 chunk。

### Phase 6：任务控制能力

输出：

- pause。
- resume。
- cancel。
- cancellation token。
- 外部子进程终止。
- 调度器运行句柄。

验收：

- 下载中取消能停止网络请求和合并进程。
- 暂停后任务不继续写入。
- 恢复后从 manifest 继续。

### Phase 7：事件存储和发布

输出：

- `EventStore`。
- `TaskEventPublisher`。
- 任务状态变化生成事件。

验收：

- 前端可按 `task_id` 订阅事件。
- 后端可按 `from_event_id` 补发。
- 高频进度事件有节流。

### Phase 8：可观测性和安全收口

输出：

- 结构化日志。
- 脱敏策略。
- request/task/event 关联。

验收：

- 能从日志追踪完整链路：

```text
request_id -> task_id -> parser -> execution_plan -> downloader -> merge -> output
```

- 日志不包含 cookies、token、密码。

---

## 测试策略

### 单元测试

- parser 匹配优先级。
- Bilibili URL 解析。
- YouTube URL 规范化。
- Twitter/X URL 和 twimg 资源归因。
- HLS manifest 解析。
- HTTP range 分块计划。
- BitTorrent magnet/torrent 识别。
- ExecutionPlan JSON round-trip。
- 错误码映射。

### 集成测试

- HTTP 创建任务成功。
- HTTP 创建任务失败。
- 同 `request_id` 幂等返回同一任务。
- 同 `request_id` 不同 payload 返回冲突。
- 任务入队后调度执行。
- 任务完成生成完成事件。
- 任务失败生成失败事件。
- 取消任务能停止执行。
- 断点恢复能继续下载。

### 回归验证命令

```powershell
cargo check
cargo test
```

如涉及扩展联调：

```powershell
cd browser-extension/app
npm run typecheck
npm run build
```

---

## 完成标准

后端达到 10 分的标准：

1. HTTP 创建任务是正式主入口。
2. Gateway 不包含站点解析和下载逻辑。
3. 站点解析器和协议解析器分层。
4. Bilibili、YouTube、Twitter/X、BitTorrent、HLS、DASH、HTTP 能独立演进。
5. 每次任务只调用匹配到的解析器和执行器。
6. 所有解析结果都收敛为 `ExecutionPlan`。
7. 下载器只消费 `ExecutionPlan`，不理解站点业务。
8. 下载支持正式断点恢复 manifest。
9. 暂停、恢复、取消是调度器正式能力。
10. 任务事件、错误、日志都结构化。
11. WebSocket/SSE 断开不影响任务执行。
12. 任务结束后运行资源被释放，daemon 回到低常驻资源状态。

---

## 给实现者的核心判断

不要为了“低内存”把所有 parser 都做成复杂动态加载。先保证边界正确：

- parser 常驻可以接受，只要它是轻量对象。
- 大资源必须按任务创建和释放。
- 站点逻辑不能泄漏到下载器。
- 下载器不能反向理解站点。
- 任务状态和事件必须由后端统一维护。

这套架构的关键不是线程数量，而是职责边界、资源生命周期和失败恢复能力。
