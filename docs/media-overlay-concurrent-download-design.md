# 媒体悬浮按钮并发下载设计

## 背景

当前媒体悬浮按钮按“页面全局单例”工作：页面里只有一个 `host/button/label/status`，并且只有一个 `currentMedia`、`downloadInFlight`、`trackedTaskId`、`activeTraceId`。这会导致两个问题：

1. 点击一个视频下载后，按钮进入“正在解析/下载速度/合并/完成”等状态，此时继续滑动到另一个视频，悬浮按钮仍复用同一套状态。
2. 第二个视频不能独立发起下载，因为 `downloadInFlight` 是全局锁；即使任务层支持并发，UI 也把并发入口锁死了。

用户期望是：X/B/Y 等页面里多个视频可以同时下载；每个视频旁边的悬浮按钮只显示自己的解析状态、任务速度和完成/失败状态。

## 现状结论

### 后端

后端任务调度已经支持并发，不是本次主要瓶颈。

- `CoreService` 用 `get_config().max_task_num` 初始化 `Semaphore`。
- `schedule_with_parts` 会在 `running_tasks.len() < max_tasks` 时持续从等待队列取任务并 `tokio::spawn`。
- 每个任务有独立 `task_id/request_id/trace_id` 和事件流。

因此本次不应为了前端并发显示去改后端调度核心。

### 通信协议

现有通信协议基本足够支持多个任务：

- `page_download_media` 每次点击都会返回独立 `taskId/requestId/traceId`。
- `task_snapshot` 广播包含多个任务列表。
- background 的 `task-store` 以 `taskId` 为主键维护任务。
- `task-event-subscriptions` 可以对多个 active task 建立事件订阅。

短期不需要新增后端 API 或破坏性协议字段。需要补的是前端 overlay 本地的“媒体实例到 taskId 的绑定关系”。

### 浏览器扩展 overlay

真正的问题在 `public/catch-script/media-download-overlay.js`：

- UI 是单例，只能显示一个媒体状态。
- `trackedTaskId` 是全局单值，只能追踪一个任务。
- `downloadInFlight` 是全局布尔，阻止第二个视频同时发起。
- `recordUiState` 使用全局 `activeTraceId/trackedTaskId`，日志无法稳定表示“哪个视频实例”的状态。
- `currentMedia` 随滚动切换，会让同一个按钮状态漂移到另一个视频旁边。

根因不是“后端不能并发”，而是 overlay 的状态所有权错误：下载状态属于一次点击绑定的媒体实例/任务，而不是属于页面全局按钮。

实现时还要避免把任务回写绑定到 DOM 元素本身，因为 X 信息流可能复用同一个 `<video>` 元素。任务快照应回写到点击时创建的 state 对象；当前按钮只在“正在展示的媒体仍对应这个 state”时渲染它。

## 目标

1. 支持同一页面多个视频同时下载。
2. 每个视频的按钮状态独立：A 视频下载时显示 A 的速度；滑到 B 视频时，B 的按钮是自己的默认/解析/下载状态。
3. 任务事件按 `taskId` 回写到正确媒体实例，不污染其他视频。
4. 不引入后端协议破坏性变更。
5. 保持 Bilibili/YouTube/X 共用 overlay 框架，站点策略仍只负责媒体解析，不直接负责 UI 并发。

## 非目标

1. 不在本阶段做复杂多按钮常驻面板。
2. 不改后端 `max_task_num` 配置语义。
3. 不改任务下载器内部 chunk/segment 并发策略。
4. 不要求页面上所有离屏视频都常驻显示按钮；优先保证当前可见视频显示自己的状态。

## 设计方案

### 核心状态模型

把 overlay 从“全局任务状态”改为“每个媒体元素一个状态”：

```js
type MediaOverlayState = {
  traceId: string;
  taskId: string;
  requestId: string;
  phase: "idle" | "resolving" | "waiting" | "dispatching" | "queued" | "downloading" | "merging" | "completed" | "failed";
  message: string;
  speed: number;
  progress: number;
  lastUiStateSignature: string;
  lockedUntil: number;
};
```

实现上不需要真的写 TypeScript 类型，因为 overlay 是普通注入 JS；但代码结构要按这个模型拆。

建议使用：

- `WeakMap<HTMLMediaElement, MediaOverlayState>`：按媒体元素保存状态，元素释放后可被 GC。
- `Map<string, MediaOverlayState>`：`taskId -> state`，用于 `task_snapshot` 回写。
- `WeakMap<HTMLMediaElement, string>` 或在 state 内保存 `traceId`：用于日志关联。

### UI 形态

短期仍保留单个悬浮按钮 DOM，但它显示“当前悬浮/当前可见媒体”的状态。

这样可以避免一次性改成多 DOM 按钮带来的布局和性能风险。用户滑到哪个视频，按钮移到哪个视频旁边，并读取该视频自己的 `MediaOverlayState` 渲染：

- 当前媒体没有状态：显示“下载此媒体”。
- 当前媒体正在解析：显示“正在解析/等待资源”。
- 当前媒体已经绑定任务：显示该任务速度/进度。
- 当前媒体完成/失败：只在该媒体旁显示完成/失败短暂反馈。

如果用户从 A 视频滑到 B 视频，按钮会切到 B 的状态；A 的任务仍在后台继续，等用户滑回 A 时，按钮显示 A 的进度或完成状态。

### 点击行为

当前逻辑：

- 全局 `downloadInFlight` 阻止任何第二次点击。

目标逻辑：

- 只阻止“同一个媒体元素”的重复点击。
- 允许不同媒体元素同时点击下载。

规则：

1. 点击时先通过 `currentMedia/findActiveMedia()` 锁定 `media`。
2. 读取 `stateByMedia.get(media)`。
3. 如果该媒体状态是 `resolving/dispatching/queued/downloading/merging`，忽略重复点击并记录 `client.click ignored`。
4. 如果是另一个媒体，不受影响，可以发起新下载。

### 任务回写

当前逻辑：

- `trackedTaskId` 全局单值。
- `applyTaskSnapshot` 只找一个 task。

目标逻辑：

- `taskIdToState` 记录所有 overlay 发起的任务。
- 每次收到 `task_snapshot`，遍历 `taskIdToState`，把对应 task 应用到对应点击 state。
- 当前按钮只负责渲染当前媒体 state。

流程：

```text
点击媒体 A
  -> 解析 A
  -> background 创建任务 taskA
  -> taskIdToState[taskA] = stateA
  -> stateByMedia[A] = downloading/queued

点击媒体 B
  -> 解析 B
  -> background 创建任务 taskB
  -> taskIdToState[taskB] = stateB
  -> stateByMedia[B] = downloading/queued

task_snapshot(taskA/taskB)
  -> 更新 A state
  -> 更新 B state
  -> 如果当前按钮正显示 A，就渲染 A；正显示 B，就渲染 B
```

### 日志关联

现有日志字段可以继续使用：

- `trace_id`
- `task_id`
- `request_id`
- `surface=overlay`
- `action=page_download_media`

建议新增 details 字段，不需要改协议：

- `mediaKey`：本地生成的媒体实例短 ID，例如 `media_1`。
- `mediaTitle`：已清理标题。
- `selectionKind`
- `phase`

这些只放在 telemetry `details`，后端协议已经支持 `details: Map<String, Value>`，不需要新字段。

### 后端是否要改

本阶段不需要改后端核心。

需要确认配置：

- `max_task_num` 如果是 1，那后端会排队而不是同时跑；这是配置语义，不是 UI 并发 bug。
- 如果用户希望真正同时下载多个任务，要保证配置大于 1。

可选增强：

- 在设置页或日志里显示当前 `max_task_num`，但这不是本次 overlay 状态隔离的必要条件。

### 通信协议是否要改

本阶段不需要改后端 API。

可选增强但不建议现在做：

- `page_download_media` 请求体新增 `client_media_id`。
- 后端任务模型保存 `client_media_id`。

这会把浏览器 DOM 概念泄漏到后端，不划算。当前用 `traceId/taskId` 在扩展内部维护映射更合适。

## 需要修改的文件

### 必改

1. `browser-extension/app/public/catch-script/media-download-overlay.js`

   主要改动：

   - 删除全局 `downloadInFlight/trackedTaskId/activeTraceId/lastUiStateSignature` 的单例语义。
   - 新增 `stateByMedia`、`taskIdToState`、媒体实例 ID 分配。
   - `downloadMedia()` 改为按当前 `media` 建状态。
   - `recordUiState()` 接收 `media/state`，从媒体状态取 `traceId/taskId`。
   - `applyTaskSnapshot()` 改为遍历所有 `taskIdToState`。
   - `updatePosition()` 每次选中当前媒体后调用 `renderCurrentMediaState()`。
   - `resetToDefault()` 改为只重置当前媒体或某个媒体 state，不再全局解锁。

### 可能改

2. `browser-extension/app/src/background/client-telemetry.ts`

   如果 overlay 新增 `details.mediaKey/phase`，这里通常不需要改，因为 details 已经泛化。但如果日志太多，需要进一步降噪时可改。

3. `browser-extension/app/src/background.ts`

   目前 `task_snapshot` 广播已经是全量任务列表，不需要改。只有在性能上需要按 tab/trace 过滤时再改。

4. `browser-extension/app/src/background/task-store.ts`

   当前按 `taskId` 存储任务，够用。通常不需要改。

### 不建议改

5. `src/services/core_service.rs`

   后端已有并发调度。本需求不是后端调度 bug。

6. `src/protocol/*`

   现有 `taskId/requestId/traceId/details` 足够表达多任务与日志关联。

7. `src/core/engine/downloader.rs`

   下载器内部并发和本次 overlay 多任务显示是两层问题，不应混改。

## 风险点

### 1. DOM 元素复用

X 信息流可能复用 `<video>` 元素播放不同 tweet。若只用 `WeakMap<video, state>`，可能把旧任务状态带到新内容。

缓解：

- state 中保存点击时的 `mediaTitle`、`src/poster`、可选 `mediaId`。
- `updatePosition()` 发现当前媒体标题/src/poster 与 state 不一致时，不把旧 idle/failed 状态渲染成新视频状态。
- active task 已绑定 taskId 的 state 不应随 DOM 复用直接删除，但 UI 渲染要避免显示到明显不同的新视频上。

### 2. 单按钮显示多个后台任务

短期设计是“单按钮显示当前媒体状态”，不是“所有任务常驻浮窗”。

这符合用户在信息流里滑动的使用方式，也避免页面上出现多个悬浮控件互相遮挡。

### 3. 任务完成后状态清理

完成/失败后不能立刻删除，否则用户滑回去看不到反馈；也不能永久留着。

建议：

- completed/failed 保留 10-30 秒或直到媒体元素明显变更。
- `taskIdToState` 在终态后延迟清理。

### 4. 日志量

多个任务并发会带来更多 overlay state 日志。

缓解：

- 每个媒体 state 独立 `lastUiStateSignature`。
- downloading 速度变化不要每次 snapshot 都记日志，只在状态阶段变化、完成、失败、dispatch 时记录。

## 实施步骤

### 阶段 1：状态隔离

只改 overlay：

1. 引入 `stateByMedia`、`taskIdToState`。
2. 把 `downloadInFlight` 改为 per-media 判断。
3. 把 `trackedTaskId` 改为 `taskIdToState` 多任务映射。
4. 把渲染逻辑抽成 `renderState(state)`。
5. `task_snapshot` 更新所有已绑定任务 state。

验收：

- A 视频下载中，滑到 B，B 显示默认按钮。
- B 点击后也能创建任务。
- A/B 两个任务在 popup 任务列表中同时存在。
- 滑回 A/B 时，各自显示自己的进度/完成状态。

### 阶段 2：DOM 复用保护

1. 给媒体 state 加 `identity`。
2. 基于 `title/src/poster/mediaId` 判断是否仍是同一媒体内容。
3. 避免 X feed 复用 video 元素导致状态错挂。

验收：

- X 首页快速上下滑，旧任务状态不会显示到无关新视频上。

### 阶段 3：日志与测试

1. 补 overlay telemetry 的 `mediaKey/phase`。
2. 增加最小 JS harness 或可执行静态测试，覆盖：
   - 同媒体重复点击被忽略。
   - 不同媒体允许并发。
   - task_snapshot 更新正确任务 state，并且只在当前媒体仍对应该 state 时渲染。

## 验证计划

### 自动验证

- `npm run typecheck`
- `npm run build`
- `cargo check`
- `cargo test`

### 手动真实站点验证

X：

1. 打开 X 信息流。
2. 点击视频 A 下载。
3. A 下载中继续向下滑。
4. 点击视频 B 下载。
5. 确认 B 不被 A 的 `downloadInFlight` 阻止。
6. 确认 A/B 在任务列表里各自有任务。
7. 滑回 A/B，按钮分别显示各自状态。

Bilibili/YouTube：

1. 单视频页面不应退化。
2. 多视频/推荐流页面如果存在多个 `<video>`，不应共享错误状态。

## 推荐结论

本需求主要改浏览器扩展 overlay，不应该改后端核心或通信协议主结构。

最稳的实现路线是：保持后端任务系统和协议不动，把 overlay 从页面全局状态改成按媒体元素/任务隔离的本地状态机。这样既支持同时下载多个视频，又不会把 DOM 细节泄漏到后端协议里。
