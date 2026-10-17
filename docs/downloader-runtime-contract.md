# Downloader Runtime Contract

更新时间：2026-06-08

## 控制命令语义

### cancel

`cancel` 表示停止运行中的网络请求、分片任务或外部子进程，并将任务进入 `canceled` 终态。

要求：

- 运行任务必须持有 `CancellationToken`。
- downloader 内部循环、分片 worker、外部进程应监听 token。
- cancel 后不应继续写最终输出文件或继续合并。

### pause

`pause` 表示停止当前运行，但保留可恢复状态。当前后端会取消运行 token，并写入 `TaskPaused` 事件。

要求：

- 支持 resume 的 downloader 必须能留下有效 `ResumeManifest` 或等价恢复数据。
- 不支持完整恢复的 downloader 可以接受 pause，但 resume 必须返回 `unsupported_operation`，不能假装恢复。

### resume

`resume` 表示只对 paused 任务重新入队，并从恢复数据继续执行。

当前支持：

- `HttpFile`
- `DashMediaMerge`

当前不声明支持：

- `Hls`
- `Youtube`
- `BitTorrent`

## 错误语义

控制命令不再统一映射为 `internal_error`：

- 任务不存在：`task_not_found`
- 当前状态不允许该命令：`invalid_task_state`
- 当前执行类型不支持该命令：`unsupported_operation`

## 验收

已有测试覆盖：

- 不存在任务 cancel 返回 `TaskControlError::NotFound`。
- completed 任务 cancel 返回 `InvalidState`。
- HLS paused 后 resume 返回 `UnsupportedOperation`。
- HTTP paused 后 resume 重新进入 queued。

后续真实站点验收：

- HTTP/Bilibili 下载中 cancel 后无最终文件继续增长。
- HTTP/Bilibili pause 后存在有效 manifest，resume 后继续。
- HLS/YouTube pause 后如果用户点击 resume，UI 显示明确不支持恢复，而不是进入假下载。
