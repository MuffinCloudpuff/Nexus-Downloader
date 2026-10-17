# 前端 10 分架构改造方案

> 当前状态提示：本文是前端改造计划底稿，评分和“当前仍停留在旧协议”的描述已经过期。当前实现状态以 `current-architecture-10-point-audit-and-roadmap.md` 为准：正式扩展已使用 HTTP daemon client、per-task event streaming、`task_updated` 增量消息和可测试 overlay 状态核心。

## 目标

把正式浏览器扩展：

```text
browser-extension/app
```

改造成和 Rust daemon 新后端协议匹配的 10 分前端架构。

目标不是只把 `WebSocket create_task` 换成 `fetch`，而是让扩展前端具备这些正式能力：

- HTTP 只负责低频命令：创建任务、查询任务、暂停、恢复、取消、配对、撤销。
- SSE 或 fetch streaming 只负责观察任务事件：进度、阶段变化、完成、失败。
- daemon 下载任务不依赖 popup、overlay、service worker 或 WebSocket 连接存活。
- popup 关闭、service worker 重启、事件流断开后，前端能通过 `task_id + event_cursor` 恢复。
- 所有跨 daemon 的协议使用 TypeScript 类型、稳定错误码和明确状态枚举。
- 扩展内部保留轻量任务状态缓存，但任务事实来源仍然是 daemon。
- cookies、token、headers 等敏感数据只在必要链路中短暂使用，不进入 UI 状态、普通日志和持久化任务缓存。

---

## 当前前端架构评分

当前扩展已经有较完整的页面媒体捕获、popup、resource bridge 和 WebSocket bridge，但通信主线还停留在旧协议。

当前评分：**6.7/10 到 7.1/10**。

主要优点：

- `background.ts` 已经是扩展内部消息总线。
- `resource-bridge.ts` 已能收集 URL、headers、cookies、video/audio 资源清单。
- `desktop-bridge.ts` 已有连接状态、配对、任务快照缓存和重连逻辑。
- popup UI 已有任务列表、资源列表、设置页和任务操作入口。
- overlay 已能从页面选择媒体并跟踪 `taskId`。

主要扣分：

- `desktop-bridge.ts` 仍然以 WebSocket 作为命令、配对、快照订阅的混合通道。
- `resource-bridge.ts` 仍发送旧 `create_task` 消息，没有构造正式 `POST /api/tasks` 请求。
- `DEFAULT_SERVER_URL` 仍是 `ws://127.0.0.1:14370`，没有区分 HTTP API base URL 和旧 WS URL。
- `manifest.json` 的 CSP 只允许 WS/WSS，本轮要允许本地 HTTP API。
- popup 和 overlay 仍依赖 `task_snapshot`，没有正式维护 `event_cursor`。
- TypeScript 侧没有和 daemon 对齐的 `CreateTaskHttpReq`、`TaskEvent`、`ErrorCode`、`TaskSummary` 类型。
- 任务操作仍通过旧 `task_action` over WebSocket，没有切到 HTTP pause/resume/cancel。
- 自动配对仍通过旧 WS `pair_request`，没有使用 HTTP pairing API。

目标评分：**10/10**。

---

## 10 分目标架构

```text
页面 overlay / popup
  -> background.ts 扩展内部消息总线
  -> resource-bridge 构造 CreateTaskHttpReq
  -> daemon-client HTTP POST /api/tasks
  -> daemon 返回 task_id + event_cursor
  -> task-event-subscriptions 用 fetch streaming 订阅 /api/tasks/{task_id}/stream
  -> task-store 应用事件并维护本地任务视图
  -> background 广播 task_updated / task_snapshot 给 popup 和 overlay
```

核心原则：

1. daemon 是任务生命周期的唯一所有者。
2. 扩展 background 是浏览器侧协调者，不是下载状态事实来源。
3. HTTP 请求的返回值只表示命令是否被接受。
4. 下载状态来自 `GET /api/tasks`、`GET /api/tasks/{task_id}` 和任务事件。
5. popup 和 overlay 都只观察 background 维护的任务视图。
6. 旧 WebSocket 只作为短期兼容入口，不再是正式主线。

---

## 前端模块边界

### 1. `shared/protocol/daemon.ts`

建议新增：

```text
src/shared/protocol/daemon.ts
```

职责：

- 定义 daemon HTTP API 的 TypeScript 类型。
- 定义 `TaskStatus`、`TaskEventType`、`ErrorCode` 枚举或联合类型。
- 定义前端内部展示所需的转换函数。

不负责：

- 发送请求。
- 读写 chrome storage。
- 操作 popup 或 overlay。

核心类型：

```ts
export type TaskStatus =
  | "queued"
  | "parsing"
  | "downloading"
  | "merging"
  | "completed"
  | "failed"
  | "canceled"
  | "paused";

export type ErrorCode =
  | "invalid_request"
  | "token_invalid"
  | "cookie_decryption_failed"
  | "unsupported_media"
  | "idempotency_conflict"
  | "network_timeout"
  | "merge_failed"
  | "internal_error";

export interface ErrorPayload {
  code: ErrorCode;
  message: string;
  retryable: boolean;
  stage?: string;
}

export interface CapturedResourcePayload {
  kind?: "video" | "audio" | "stream" | "subtitle" | "unknown";
  url: string;
  filename?: string;
  mime?: string;
  size?: number;
  headers?: Record<string, string>;
  supports_range?: boolean;
  page_title?: string;
}

export interface CreateTaskPayload {
  url: string;
  headers?: Record<string, string>;
  filename?: string;
  size?: number;
  supports_range?: boolean;
  resources?: CapturedResourcePayload[];
  encrypted_cookies?: string;
  cookie_iv?: string;
}

export interface CreateTaskHttpReq {
  request_id: string;
  source: "page_media" | "resource" | "resource_merge" | "download";
  title?: string;
  payload: CreateTaskPayload;
  client_context: {
    extension_id: string;
    tab_id?: number;
    page_url?: string;
    protocol_version: number;
  };
}

export interface CreateTaskHttpResp {
  request_id: string;
  task_id?: string;
  accepted: boolean;
  initial_status?: TaskStatus;
  event_cursor?: number;
  error?: ErrorPayload;
}

export interface TaskSummary {
  task_id: string;
  title: string;
  status: TaskStatus;
  progress: number;
  received_bytes: number;
  total_bytes: number;
  speed_bytes_per_sec: number;
  created_at: string;
  request_id?: string;
}

export interface TaskEvent {
  event_id: number;
  task_id: string;
  request_id?: string;
  type: string;
  occurred_at: string;
  payload: unknown;
}
```

注意：

- daemon JSON 使用 `snake_case`。
- popup 旧组件可以继续吃 `GenericTaskSummary`，但必须由 mapper 从 daemon `TaskSummary` 转换，不能再把旧 WS 字段当作后端事实协议。
- 不要在协议类型里放 token、cookie 明文或完整敏感 header。

### 2. `background/daemon-client.ts`

建议新增：

```text
src/background/daemon-client.ts
```

职责：

- 封装 HTTP API。
- 统一加 `Authorization: Bearer <token>`。
- 统一超时、错误解析、响应校验。
- 暴露语义明确的方法：

```ts
createTask(req: CreateTaskHttpReq): Promise<CreateTaskHttpResp>
getTasks(): Promise<TaskSummary[]>
getTask(taskId: string): Promise<TaskSummary | null>
getTaskEvents(taskId: string, fromEventId: number): Promise<TaskEvent[]>
pauseTask(taskId: string): Promise<TaskCommandResp>
resumeTask(taskId: string): Promise<TaskCommandResp>
cancelTask(taskId: string): Promise<TaskCommandResp>
requestPairing(): Promise<PairingResp>
revokePairing(): Promise<void>
```

不负责：

- 缓存任务状态。
- 决定 UI 怎么展示错误。
- 直接广播消息给 popup 或 overlay。

### 3. `background/task-event-subscriptions.ts`

建议新增：

```text
src/background/task-event-subscriptions.ts
```

职责：

- 管理每个运行中任务的事件订阅。
- 从 `event_cursor` 开始补发并继续接收实时事件。
- 事件流断开后按任务状态决定是否重连。
- 终态任务关闭订阅。

推荐不要直接使用浏览器原生 `EventSource` 作为第一选择。

原因：

- daemon 当前 HTTP 鉴权使用 `Authorization` header。
- 原生 `EventSource` 不能设置自定义 `Authorization` header。
- 因此前端应优先用 `fetch` + `ReadableStream` 读取 SSE 文本流，并自己解析 `data:` 行。

流程：

```text
subscribeTask(task_id, from_event_id)
  -> fetch /api/tasks/{task_id}/stream?from_event_id=...
       Authorization: Bearer token
  -> 先收到补发事件
  -> 再收到实时事件
  -> 每条事件交给 task-store.applyEvent
  -> 更新 cursor
  -> 广播本地任务视图变化
```

断线策略：

- `completed`、`failed`、`canceled`：不重连。
- `queued`、`parsing`、`downloading`、`merging`、`paused`：可重连。
- 重连时使用该任务最后持久化的 `event_cursor`。
- 重连退避从 1 秒开始，最多 15 秒。

### 4. `background/task-store.ts`

建议新增：

```text
src/background/task-store.ts
```

职责：

- 维护扩展侧任务缓存。
- 应用 daemon `TaskSummary` 和 `TaskEvent`。
- 记录每个任务最后处理到的 `event_cursor`。
- 输出 popup/overlay 当前需要的 `GenericTaskSummary[]`。
- 持久化非敏感状态到 `chrome.storage.local`。

建议状态：

```ts
interface BrowserTaskState {
  tasks_by_id: Record<string, TaskSummary>;
  event_cursor_by_task_id: Record<string, number>;
  active_task_ids: string[];
  updated_at: number;
}
```

持久化要求：

- 可以持久化 `task_id`、`status`、`progress`、`title`、`event_cursor`。
- 不持久化 token 之外的任何凭证。
- 不持久化 cookie、Authorization、完整 headers。
- token 仍使用现有 `PAIR_TOKEN_KEY`，但设置页展示时要避免误复制日志。

### 5. `background/resource-bridge.ts`

职责保留：

- 捕获资源。
- 收集 headers。
- 收集必要 cookies 并加密。
- 判断单资源、stream、video/audio merge。
- 构造 daemon 创建任务 payload。

本轮必须修改：

- 不再发送 `{ type: "create_task" }` 给 `desktopBridge.sendRequest`。
- 改为构造 `CreateTaskHttpReq` 并调用 `daemonClient.createTask`。
- 每次用户动作都生成稳定 `request_id`，用于 HTTP 重试幂等。
- `encrypted_cookies` 和 `cookie_iv` 字段名要和 daemon 对齐，不再混用 `iv`。
- `resources` 内字段统一转换成 daemon 期望的 `snake_case`。
- 创建成功后返回 `{ ok: true, taskId, eventCursor }` 给 overlay/popup。

### 6. `background.ts`

职责保留：

- 扩展内部消息路由。
- popup、overlay、content scripts 的统一入口。
- task-store 变化后广播 UI 消息。

本轮必须修改：

- 初始化时创建 `daemonClient`、`taskStore`、`taskEventSubscriptions`。
- popup 打开时：
  - 先返回 task-store 缓存。
  - 后台异步 `GET /api/tasks` 刷新 daemon 快照。
  - 对未终态任务恢复订阅。
- overlay 创建任务成功后：
  - 立即把 accepted 任务写入 task-store。
  - 启动事件订阅。
  - 返回 `taskId` 给 overlay。
- popup 的任务操作改成 HTTP pause/resume/cancel。
- 配对改成 HTTP `/api/pairing/request`。

### 7. popup UI

职责：

- 展示 background 给出的任务视图。
- 发送用户命令。
- 展示明确错误消息。

本轮不要求重做 UI 视觉，但必须保证：

- `queued`、`parsing`、`downloading`、`merging`、`paused`、`completed`、`failed`、`canceled` 都有合理展示。
- 任务命令按钮根据状态启用或禁用。
- token 无效、daemon 未启动、资源不可解析、幂等冲突、cookie 解密失败等错误显示中文文案。
- popup 打开时不要只等事件流，必须先查询或读取当前任务状态。

### 8. overlay

职责：

- 页面内按钮、点击下载、展示当前点击任务的轻量状态。

本轮必须保证：

- 点击后如果 HTTP 创建成功，立即记录 `taskId`。
- 通过 background 广播的任务更新追踪该 `taskId`。
- popup 关闭或后台事件流重连不影响 overlay 的当前状态更新。
- 失败时显示 daemon 返回的错误 message。
- 终态后短暂显示结果，再恢复可点击状态。

---

## 前后端通信链路

### 创建任务

```text
overlay / popup
  -> chrome.runtime.sendMessage(page_download_media / popup_send_resource)
  -> background.ts
  -> resource-bridge 构造 CreateTaskHttpReq
  -> daemon-client POST /api/tasks
  -> task-store 写入 accepted/queued 视图
  -> task-event-subscriptions 订阅事件
  -> 返回 taskId 给调用方
```

请求：

```http
POST http://127.0.0.1:14371/api/tasks
Authorization: Bearer gd3_xxx
Content-Type: application/json
```

响应：

```json
{
  "request_id": "req_xxx",
  "task_id": "task_xxx",
  "accepted": true,
  "initial_status": "queued",
  "event_cursor": 0
}
```

要求：

- `accepted` 只表示 daemon 接受任务，不表示下载成功。
- 相同 `request_id` 的重复请求必须显示为同一个任务。
- 相同 `request_id` 不同 payload 必须展示 `idempotency_conflict`，不能静默创建第二个任务。

### 订阅事件

```text
task-event-subscriptions
  -> GET /api/tasks/{task_id}/stream?from_event_id=cursor
  -> 解析 SSE data
  -> task-store.applyEvent(event)
  -> 持久化 cursor
  -> background 广播任务变化
```

要求：

- 订阅前先有当前任务快照。
- 订阅时带 `from_event_id`。
- 断线重连不丢事件。
- 终态任务关闭订阅。
- 事件流只是观察窗口，不是任务生命线。

### 查询任务

popup 打开或 service worker 恢复时：

```text
GET /api/tasks
  -> task-store.replaceSnapshot
  -> 对未终态任务 subscribeTask(task_id, cursor)
```

单任务需要补状态时：

```text
GET /api/tasks/{task_id}
GET /api/tasks/{task_id}/events?from_event_id=cursor
```

### 任务命令

```text
POST /api/tasks/{task_id}/pause
POST /api/tasks/{task_id}/resume
POST /api/tasks/{task_id}/cancel
```

响应中的字段应使用明确语义：

```json
{
  "task_id": "task_xxx",
  "command_accepted": true,
  "message": "..."
}
```

前端不能再依赖旧 `success` 或 `ok` 表示 daemon 多层状态。扩展内部给 UI 的返回值可以继续用 `{ ok: boolean }`，但必须由 daemon 结构化响应转换而来。

---

## 配置、CSP 和 URL

### URL 存储

旧字段：

```text
desktopServerUrl = ws://127.0.0.1:14370
```

建议迁移成：

```text
daemonHttpBaseUrl = http://127.0.0.1:14371
legacyWsUrl = ws://127.0.0.1:14370
```

迁移规则：

- 如果旧 `desktopServerUrl` 是 `ws://127.0.0.1:14370`，自动推导 `daemonHttpBaseUrl = http://127.0.0.1:14371`。
- 设置页优先展示 HTTP API 地址。
- 旧 WS URL 只在兼容或诊断区域保留。

### Manifest CSP

当前 CSP 只允许 WS/WSS。前端改造必须允许本地 HTTP API：

```json
{
  "content_security_policy": {
    "extension_pages": "default-src 'self'; script-src 'self'; object-src 'self'; style-src 'self' 'unsafe-inline'; connect-src http://127.0.0.1:* http://localhost:* ws://127.0.0.1:* ws://localhost:* wss://127.0.0.1:* wss://localhost:*;"
  }
}
```

说明：

- HTTP 本地 API 是正式主线。
- WS 只为短期兼容保留。
- 不要开放任意远程 origin。

---

## 错误映射

建议新增：

```text
src/shared/protocol/error-messages.ts
```

示例：

```ts
export function messageForDaemonError(error: ErrorPayload): string {
  switch (error.code) {
    case "token_invalid":
      return "配对令牌无效，请重新配对";
    case "cookie_decryption_failed":
      return "页面凭据解密失败，请刷新页面后重试";
    case "unsupported_media":
      return "当前媒体资源暂时无法解析";
    case "idempotency_conflict":
      return "这次请求和已提交任务不一致，请重新点击下载";
    case "network_timeout":
      return "网络请求超时，请稍后重试";
    default:
      return error.message || "任务创建失败";
  }
}
```

要求：

- UI 展示中文 message。
- 程序判断使用稳定 `code`。
- 日志不输出 token、cookies、Authorization。
- `retryable` 可以用于决定是否显示“重试”按钮。

---

## 实施阶段

### Phase 1：冻结前端协议类型

输出：

- `src/shared/protocol/daemon.ts`
- `src/shared/protocol/error-messages.ts`
- daemon `snake_case` 到 popup `GenericTaskSummary` 的 mapper。

验收：

- 所有 daemon API 类型集中定义。
- `resource-bridge.ts` 不再散落临时 JSON 字段名。
- `npm run typecheck` 通过。

### Phase 2：新增 HTTP daemon client

输出：

- `src/background/daemon-client.ts`
- HTTP timeout。
- token header。
- 结构化错误解析。
- HTTP pairing。

验收：

- `POST /api/pairing/request` 可拿到 token。
- `GET /api/tasks` 可拿到任务列表。
- token 无效时连接状态显示 `unauthorized`。

### Phase 3：新增 task-store 和事件订阅管理

输出：

- `src/background/task-store.ts`
- `src/background/task-event-subscriptions.ts`
- `fetch` streaming SSE parser。
- cursor 持久化。

验收：

- popup 打开能看到已有任务。
- 事件流断开后能从 cursor 恢复。
- 终态任务不继续保持订阅。

### Phase 4：改造 resource-bridge 创建任务

输出：

- `sendHttpResourceToDesktop` 改为 HTTP create task。
- `sendPageMediaResourceToDesktop` 改为 HTTP create task。
- `dispatchMergeResources` 改为 HTTP create task。
- browser download handoff 改为 HTTP create task。

验收：

- 创建任务请求包含 `request_id`、`source`、`payload`、`client_context`。
- cookies 字段使用 `encrypted_cookies` + `cookie_iv`。
- 创建成功返回 `taskId` 和 `eventCursor`。
- 创建失败返回明确中文错误。

### Phase 5：改造 popup 和 overlay 状态来源

输出：

- background 广播由 task-store 生成的任务更新。
- popup 任务命令改为 HTTP pause/resume/cancel。
- overlay 继续用 `taskId` 追踪，但状态来自 task-store。

验收：

- popup 关闭后任务继续。
- popup 重开后先显示快照，再恢复事件订阅。
- overlay 点击后能看到 queued/parsing/downloading/merging/completed/failed。

### Phase 6：降低旧 WebSocket 主线

输出：

- `desktop-bridge.ts` 删除或降级为 `legacy-ws-bridge.ts`。
- 旧 `create_task` over WebSocket 不再由正式前端调用。
- 设置页说明改成 HTTP API 地址。

验收：

- `rg "type: \"create_task\"" app/src` 不再出现在正式发送 daemon 请求的路径里。
- `DEFAULT_SERVER_URL` 不再是正式 daemon HTTP 地址。
- WS 只作为 deprecated fallback 或诊断使用。

### Phase 7：验证和回归

命令验证：

```powershell
cd browser-extension/app
npm run typecheck
npm run build
```

后端联调：

```powershell
cargo check
cargo test
```

页面验证：

- 普通 HTML5 视频页面：overlay 出现，点击后 HTTP 创建任务。
- Bilibili：能创建任务，进度事件更新。
- YouTube：能创建任务，错误或外部下载状态明确显示。
- X/Twitter：能创建 stream 或 HLS 任务。
- popup 关闭后任务继续。
- popup 重开后任务状态恢复。
- daemon 未启动时显示明确错误。
- token 无效时提示重新配对。
- 同一点击请求重试不会创建重复任务。

---

## 深水区和处理策略

### 1. MV3 service worker 生命周期

风险：

- background service worker 可能休眠，长连接事件流可能断开。

策略：

- 不把事件流当任务生命线。
- cursor 持久化。
- service worker 恢复后先 `GET /api/tasks`，再按未终态任务重建订阅。
- alarm 只做低频恢复兜底，不做进度轮询主链路。

### 2. SSE 鉴权

风险：

- 原生 `EventSource` 不能设置 `Authorization` header。

策略：

- 使用 `fetch` streaming 订阅 SSE。
- 所有请求统一走 `daemon-client` 设置 token。
- 如果未来后端引入一次性订阅 token，再考虑原生 EventSource。

### 3. 任务视图一致性

风险：

- popup、overlay、background 缓存、daemon 快照可能短时间不一致。

策略：

- daemon 是事实来源。
- task-store 只维护派生视图。
- 每个事件按 `event_id` 单调应用。
- 发现 cursor 缺口时，先 `GET /api/tasks/{task_id}` 和 `/events` 修复，再继续订阅。

---

## 完成标准

前端达到 10 分的标准：

1. 扩展正式主线通过 HTTP `POST /api/tasks` 创建任务。
2. 创建任务返回值只表示 accepted 和 `task_id`，不混淆下载成功。
3. 下载进度通过 `/api/tasks/{task_id}/stream` 或 fetch streaming 事件更新。
4. popup 关闭、overlay 消失、事件流断开都不影响 daemon 下载。
5. popup 重新打开后能查询当前任务并从 `event_cursor` 继续订阅。
6. 重复点击或 HTTP 重试不会创建重复任务。
7. pause/resume/cancel 通过 HTTP 命令通道发送。
8. 配对和撤销通过 HTTP pairing API 完成。
9. TypeScript 侧有正式协议类型、错误码和状态枚举。
10. CSP、CORS、token、cookies、headers 都有明确安全边界。
11. popup 和 overlay 都能显示 queued/parsing/downloading/merging/paused/completed/failed/canceled。
12. 旧 WebSocket create_task 不再是正式前端调用路径。

---

## 给实现者的核心判断

前端这轮改造的关键不是“换请求库”，而是把浏览器侧拆成清楚的三件事：

- resource bridge 负责收集和构造创建任务请求。
- daemon client 负责 HTTP 命令和事件订阅。
- task store 负责把 daemon 快照和事件变成 UI 可读状态。

只要这三个边界稳住，popup、overlay、资源列表和后续扩展功能都能继续演进，而不会再被旧 WebSocket 快照模型绑住。
