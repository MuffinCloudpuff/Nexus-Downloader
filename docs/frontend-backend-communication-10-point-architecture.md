# 前后端通信 10 分架构改造方案

> 当前状态提示：本文是通信改造设计底稿，部分“当前架构问题”描述已经完成整改。当前实现状态以 `current-architecture-10-point-audit-and-roadmap.md` 和 `docs/adr/0001-http-commands-sse-task-events.md` 为准：HTTP commands + SSE task events 已成为正式通信主线，WebSocket `create_task` 仅为 deprecated legacy adapter。

## 目标

把浏览器扩展和 Rust daemon 的通信改造成职责清晰、可恢复、可观测、可长期维护的正式架构：

- HTTP/HTTPS 只负责低频命令：创建任务、查询任务、暂停、恢复、取消。
- WebSocket 或 SSE 只负责任务运行期间的事件订阅：进度、阶段变化、完成、失败。
- WebSocket/SSE 断开不影响后端下载任务。
- 任务结束后订阅连接可以关闭，下一次用户点击下载时重新通过 HTTP 创建新任务。
- 前后端协议必须结构化、可版本化、可测试，避免临时字符串 JSON 和模糊布尔字段。

这份文档用于指导后续实现。目标不是堆复杂度，而是让异常情况下也不乱。

---

## 当前架构问题

当前 Rust daemon 已经有 WebSocket gateway，浏览器扩展通过 WebSocket 做这些事：

1. `hello` / `hello_ack` 连接握手。
2. `pair_request` 配对。
3. `subscribe_tasks` 订阅任务快照。
4. `create_task` 创建任务。
5. `task_snapshot` 推送任务列表。

主要问题：

1. **命令和订阅混在同一条 WebSocket 里**
   创建任务是低频命令，进度同步是高频事件，两者通信特征不同，不应该长期混在同一协议入口里。

2. **WebSocket 被误用成任务生命线**
   正确边界应是：WebSocket 只是观察通道。前端断开、popup 关闭、浏览器 service worker 被暂停，后端下载必须继续。

3. **协议结构不够正式**
   后端直接读取 `serde_json::Value` 中的字符串字段，前端也靠散落的对象字面量发送消息。跨模块、跨进程协议应使用明确类型、枚举状态和错误码。

4. **创建结果和任务状态边界不清**
   `create_task_result` 应只表示“任务是否被接受并入队”。下载中、合并中、完成、失败都应该通过任务事件或任务状态查询表达。

5. **鉴权和来源控制不足**
   本地 daemon 即使监听 `127.0.0.1`，也不能无条件信任所有请求。必须有扩展配对 token、来源校验、token 撤销和日志脱敏。

6. **任务快照过重**
   高频推完整 `task_snapshot` 可以作为兼容兜底，但正式实时通道应优先推单任务事件，降低 UI 状态同步复杂度和消息体积。

---

## 架构评分标准

| 方案 | 分数 | 评价 |
|---|---:|---|
| WebSocket 同时负责创建任务、订阅、控制、推快照 | 6.5 | 能跑，但协议职责混杂，异常恢复弱 |
| HTTP 创建任务 + WebSocket/SSE 下载期推事件 + 任务结束断开订阅 | 9 | 主链路清楚，复杂度合适 |
| 9 分方案 + 幂等、事件补发、断线恢复、正式鉴权、结构化协议、完整可观测性 | 10 | 可长期维护，异常情况下行为稳定 |

---

## 10 分目标架构

```text
用户点击页面悬浮下载按钮
  -> content script 捕获点击和媒体选择
  -> background/service worker 收集 URL、headers、cookies、资源清单
  -> HTTP POST /api/tasks 创建任务
  -> daemon 返回 task_id、accepted 状态、event_cursor
  -> 前端按需打开 WebSocket/SSE 订阅 task_id
  -> daemon 推送 task_progress / task_stage_changed / task_completed / task_failed
  -> 任务结束后前端关闭订阅连接
  -> 后端清理订阅关系，但任务记录保留
```

核心原则：

1. HTTP 是命令通道。
2. WebSocket/SSE 是观察通道。
3. 任务生命周期属于 daemon，不属于前端连接。
4. 所有跨边界数据必须有稳定契约。
5. 所有关键链路必须能通过 `request_id` 和 `task_id` 追踪。

---

## 通信边界

### HTTP/HTTPS 负责的低频命令

这些 API 是请求-响应语义，适合 HTTP：

- 创建任务。
- 查询任务当前状态。
- 暂停任务。
- 恢复任务。
- 取消任务。
- 查询任务历史事件。
- 配对、刷新 token、撤销 token。

推荐接口：

```text
POST   /api/tasks
GET    /api/tasks/{task_id}
GET    /api/tasks/{task_id}/events?from_event_id=123
POST   /api/tasks/{task_id}/pause
POST   /api/tasks/{task_id}/resume
POST   /api/tasks/{task_id}/cancel
POST   /api/pairing/request
POST   /api/pairing/revoke
```

### WebSocket/SSE 负责的高频事件

这些是服务端到前端的事件流，适合 WebSocket 或 SSE：

- 任务开始。
- 解析阶段变化。
- 下载进度。
- 速度和已下载字节变化。
- 合并阶段变化。
- 完成。
- 失败。
- 取消。

推荐订阅方式：

```text
GET /api/tasks/{task_id}/stream?from_event_id=123
```

如果使用 WebSocket：

```json
{
  "type": "subscribe_task",
  "task_id": "task_xxx",
  "from_event_id": 123
}
```

如果只需要单向推送，优先考虑 SSE。若后续要在同一连接内做多任务订阅、取消订阅、心跳、批量事件，使用 WebSocket。

---

## 创建任务协议

### 请求

```json
{
  "request_id": "req_20260605_abcdef",
  "source": "page_media",
  "title": "video title",
  "payload": {
    "url": "https://example.com/video.m3u8",
    "headers": {
      "referer": "https://example.com/page",
      "user-agent": "..."
    },
    "resources": [
      {
        "kind": "video",
        "url": "https://example.com/video.mp4",
        "mime": "video/mp4",
        "size": 123456,
        "headers": {}
      }
    ],
    "encrypted_cookies": "...",
    "cookie_iv": "..."
  },
  "client_context": {
    "extension_id": "chrome-extension-id",
    "tab_id": 123,
    "page_url": "https://example.com/page",
    "protocol_version": 1
  }
}
```

### 响应

```json
{
  "request_id": "req_20260605_abcdef",
  "task_id": "task_20260605_abcdef",
  "accepted": true,
  "initial_status": "queued",
  "event_cursor": 0
}
```

失败响应：

```json
{
  "request_id": "req_20260605_abcdef",
  "accepted": false,
  "error": {
    "code": "unsupported_media",
    "message": "当前媒体资源无法解析",
    "retryable": false
  }
}
```

要求：

- `request_id` 必填，用于幂等和日志追踪。
- `task_id` 只在任务被接受后返回。
- `accepted` 只表示后端是否接受并入队，不表示下载成功。
- 不要使用模糊的 `ok` / `success` 作为跨层关键语义字段。

---

## 幂等要求

创建任务必须支持幂等。

场景：

- 用户连续点击下载按钮。
- HTTP 请求超时后前端重试。
- Chrome extension service worker 重启后重新发送。
- 网络抖动导致前端无法确认第一次请求是否成功。

规则：

1. 同一个授权客户端在幂等窗口内重复发送相同 `request_id`，后端必须返回同一个 `task_id`。
2. 如果第一次请求已经创建任务，第二次不能重复创建下载任务。
3. 幂等记录应至少保存到任务完成后一段时间。
4. 如果相同 `request_id` 携带不同 payload，后端应返回明确错误，例如 `idempotency_conflict`。

---

## 任务事件协议

所有事件必须包含：

- `event_id`
- `task_id`
- `type`
- `occurred_at`
- `request_id` 或 `causation_id`

推荐事件类型：

```ts
type TaskEvent =
  | TaskAccepted
  | TaskQueued
  | TaskParsingStarted
  | TaskStageChanged
  | TaskDownloadProgressUpdated
  | TaskMergeStarted
  | TaskCompleted
  | TaskFailed
  | TaskCanceled;
```

示例：

```json
{
  "event_id": 42,
  "task_id": "task_20260605_abcdef",
  "type": "task_download_progress_updated",
  "occurred_at": "2026-06-05T20:30:00+08:00",
  "stage": "downloading",
  "progress": 0.42,
  "received_bytes": 104857600,
  "total_bytes": 249561088,
  "speed_bytes_per_sec": 3145728
}
```

完成事件：

```json
{
  "event_id": 88,
  "task_id": "task_20260605_abcdef",
  "type": "task_completed",
  "occurred_at": "2026-06-05T20:35:00+08:00",
  "output_path": "D:/Downloads/video.mp4"
}
```

失败事件：

```json
{
  "event_id": 89,
  "task_id": "task_20260605_abcdef",
  "type": "task_failed",
  "occurred_at": "2026-06-05T20:35:10+08:00",
  "failure": {
    "code": "http_403",
    "message": "下载资源被服务器拒绝",
    "retryable": false,
    "stage": "downloading"
  }
}
```

要求：

- 进度统一使用 `0.0 ~ 1.0`。
- 阶段状态使用枚举，不使用自由字符串。
- 错误必须有稳定 `code`，UI 展示用 `message`。
- 不要在事件中包含 cookies、token、完整敏感 headers。

---

## 断线恢复和事件补发

HTTP 创建任务成功后，WebSocket/SSE 可能还没连上，任务已经开始执行。因此必须支持事件补发。

流程：

```text
POST /api/tasks
  <- task_id, event_cursor

GET /api/tasks/{task_id}/stream?from_event_id=event_cursor
  <- 补发 event_cursor 之后的事件
  <- 继续推送实时事件
```

规则：

1. 后端为每个任务保留最近事件日志。
2. 前端订阅时带 `from_event_id`。
3. 后端先补发缺失事件，再推实时事件。
4. 如果事件日志已过期，后端返回当前任务快照和新的 cursor。
5. 前端连接断开后，不影响任务执行。
6. 前端重新打开 popup 或 overlay 时，先 `GET /api/tasks/{task_id}` 获取当前状态，再订阅后续事件。

---

## 任务状态模型

推荐任务状态：

```text
queued
parsing
downloading
merging
completed
failed
canceled
paused
```

状态含义：

| 状态 | 含义 |
|---|---|
| `queued` | 已接受，等待调度 |
| `parsing` | 正在解析页面、manifest、资源列表 |
| `downloading` | 正在下载实际资源 |
| `merging` | 正在合并音视频或分片 |
| `completed` | 成功完成 |
| `failed` | 失败且不会自动继续 |
| `canceled` | 用户取消 |
| `paused` | 用户暂停 |

注意：

- `accepted` 是创建请求结果，不是任务状态。
- `completed` / `failed` / `canceled` 是终态。
- WebSocket/SSE 关闭不是任务状态。

---

## 前端模块边界

正式扩展主线应保持在：

```text
browser-extension/app
```

建议边界：

1. **content script / overlay**
   - 只负责页面按钮、用户点击、当前视频选择。
   - 不直接理解 daemon 协议。
   - 不保存长期任务状态。

2. **page-media**
   - 负责识别页面媒体、MSE 归因、video/audio 选择。
   - 输出结构化媒体选择结果。

3. **background/service worker**
   - 是扩展内部消息总线。
   - 负责调用资源桥接、daemon client、状态广播。

4. **resource bridge**
   - 收集 URL、headers、referer、cookies、资源清单。
   - 生成 `CreateTaskRequest`。
   - 不直接维护下载进度 UI。

5. **daemon client**
   - 封装 HTTP API 和 WebSocket/SSE 订阅。
   - 统一处理 token、重试、超时、错误映射。

6. **popup UI**
   - 只展示状态和发送用户命令。
   - 打开时先查询快照，再订阅事件。

---

## 后端模块边界

建议边界：

1. **gateway/http**
   - HTTP 路由。
   - 鉴权。
   - 请求体验证。
   - 调用应用服务。

2. **gateway/events**
   - WebSocket/SSE 事件订阅。
   - 按 `task_id` 管理订阅者。
   - 事件补发、心跳、断开清理。

3. **protocol**
   - 前后端共享协议的 Rust 类型。
   - 请求、响应、事件、错误码、状态枚举。

4. **task application service**
   - 处理创建任务、幂等、暂停、恢复、取消。
   - 调用 `PackRegistry` 和 `CoreService`。

5. **TaskService**
   - 任务数据所有者。
   - 任务状态持久化。
   - 任务事件追加。

6. **PackRegistry / Packs**
   - 解析请求，生成标准任务。
   - 不处理前端协议和连接细节。

7. **DownloadExecutor**
   - 统一执行入口。
   - 负责把执行过程转化为任务事件或状态更新。

---

## 鉴权和安全

必须实现：

1. 扩展配对 token。
2. token 可撤销。
3. token 不写入普通日志。
4. 校验请求来源，至少校验本地来源、扩展标识或配对凭证。
5. CORS 不允许无限放开到任意网页。
6. cookies、Authorization、敏感 headers 只在必要链路中短暂使用，不进入任务事件和普通日志。
7. 错误日志只记录脱敏摘要，例如 `has_cookies=true`，不要记录 cookie 内容。

---

## 可观测性

每条关键链路必须记录结构化日志字段：

- `request_id`
- `task_id`
- `event_id`
- `source`
- `tab_id`
- `page_origin`
- `pack_type`
- `stage`
- `status`
- `failure_code`
- `retryable`

关键日志点：

1. HTTP 创建任务请求收到。
2. 鉴权通过或失败。
3. 幂等命中或冲突。
4. Pack 匹配结果。
5. 任务入队。
6. 任务开始执行。
7. 下载进度按节流间隔记录。
8. 合并开始和结束。
9. 任务完成、失败、取消。
10. 事件订阅建立、补发、断开。

日志要求：

- 不记录 token、cookies、密码、完整敏感 headers。
- 高频进度日志要节流，避免刷爆日志。
- 失败日志必须能定位到具体阶段。

---

## 实施阶段

### Phase 1：冻结目标协议

输出：

- Rust 请求/响应/事件类型。
- TypeScript 对应类型。
- 错误码枚举。
- 任务状态枚举。

要求：

- 不再新增裸 `serde_json::Value` 解析主链路。
- 所有跨前后端字段使用 snake_case 或明确转换规则，不要同一协议混用 `requestId` 和 `request_id`。

### Phase 2：新增 HTTP 创建任务 API

实现：

```text
POST /api/tasks
GET  /api/tasks/{task_id}
```

要求：

- 支持 `request_id` 幂等。
- 创建成功返回 `task_id` 和 `event_cursor`。
- 创建失败返回结构化错误。
- 旧 WebSocket `create_task` 可以短期保留为兼容入口，但必须标记为 deprecated，并内部转调同一应用服务。

### Phase 3：新增任务事件存储和发布

实现：

- 每个任务有递增 `event_id`。
- 任务状态变化时追加事件。
- 事件可查询。
- 事件可发布给订阅者。

要求：

- 任务事件和任务当前状态都由后端统一维护。
- 不要让 gateway 自己拼装业务状态。

### Phase 4：新增 WebSocket/SSE 订阅接口

实现：

```text
GET /api/tasks/{task_id}/stream?from_event_id=...
```

或 WebSocket：

```json
{
  "type": "subscribe_task",
  "task_id": "task_xxx",
  "from_event_id": 123
}
```

要求：

- 订阅时先补发事件。
- 下载中推实时事件。
- 终态事件发送后允许前端关闭连接。
- 前端断开时后端清理订阅者，但不取消任务。

### Phase 5：改造扩展 daemon client

实现：

- `createTask(request)` 使用 HTTP。
- `subscribeTask(taskId, fromEventId)` 使用 WebSocket/SSE。
- `pause/resume/cancel` 使用 HTTP。
- 所有错误映射成 UI 能展示的明确中文消息。

要求：

- overlay 点击后拿到 `task_id` 再订阅。
- popup 打开时先查任务快照，再订阅运行中的任务。
- service worker 重启后能从存储中恢复未完成任务并重新订阅。

### Phase 6：移除或降级旧 WebSocket 命令入口

要求：

- `create_task` over WebSocket 不再作为正式主线。
- 如果短期保留，代码和文档必须标记 deprecated。
- 最终只允许 WebSocket/SSE 承担事件订阅职责。

### Phase 7：验收和回归测试

后端测试：

- 创建任务成功。
- 创建任务失败。
- 相同 `request_id` 重试返回同一 `task_id`。
- 相同 `request_id` 不同 payload 返回 `idempotency_conflict`。
- 事件 `event_id` 单调递增。
- 订阅时能从 `from_event_id` 补发事件。
- WebSocket/SSE 断开后任务继续执行。

前端测试：

- 点击悬浮按钮后通过 HTTP 创建任务。
- 创建成功后订阅任务事件。
- 进度、合并、完成、失败状态能正确显示。
- popup 关闭后任务继续。
- popup 重新打开后能恢复当前任务状态。
- daemon 未启动、token 无效、资源不可解析时有明确错误。

命令验证：

```powershell
cargo check
cd browser-extension/app
npm run typecheck
npm run build
```

---

## 验收标准

这个改造完成的标准：

1. 用户点击视频旁边的悬浮下载按钮后，扩展通过 HTTP 创建 daemon 任务。
2. HTTP 创建接口只返回任务是否被接受和 `task_id`，不承担进度推送。
3. 下载期间前端通过 WebSocket/SSE 订阅该 `task_id` 的事件。
4. 任务完成或失败后，前端收到终态事件并关闭订阅连接。
5. 前端连接断开不影响后端任务继续执行。
6. 前端重新打开时能查询当前任务状态并从 cursor 继续订阅。
7. 重复点击或请求重试不会创建重复任务。
8. 所有跨前后端协议有明确类型、状态枚举和错误码。
9. 鉴权、token、cookies、headers 均有明确安全边界和日志脱敏。
10. 日志能用 `request_id` 和 `task_id` 追踪从点击、创建、解析、下载、合并到完成或失败的完整链路。

---

## 给实现者的核心判断

不要把 10 分架构理解成“更多中间件”。

这里的 10 分来自清晰边界：

- HTTP 负责命令。
- WebSocket/SSE 负责观察。
- 任务属于后端。
- 连接只是订阅窗口。
- 协议必须稳定。
- 异常必须可恢复。
- 问题必须可追踪。
