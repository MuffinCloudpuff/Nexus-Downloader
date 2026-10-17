# Downloader Runtime Contract

更新时间：2026-10-17

## 浏览器另存为与图片

- 扩展在 `downloads.onChanged` 给出最终绝对 `filename` 后才考虑接管；`onDeterminingFilename` 仅调用 `suggest()`，不能在保存对话框确认前取消下载。
- 已完成、已中断、用户取消保存、路径不明确或后端不支持新规则时，保留浏览器下载，不创建替代任务。
- 接管前通过鉴权接口 `GET /api/capabilities` 确认 `browser_download_save_path: true`，避免旧后端忽略新字段。
- 浏览器下载取消成功且后台确认接管后，才调用 `downloads.erase({id})` 移除该次下载的“已取消”记录。不清空其他记录、不调用文件删除或全局隐藏下载界面的接口；接管失败保留记录。历史清理失败仅记录警告，不重建后台任务，也不改变已接管状态或保存位置。
- `source: "download"` 的 `payload.save_path` 为浏览器确认的完整绝对路径，优先于全部目录模式；路径非法返回错误，不能悄悄改存默认目录。该字段参与幂等性指纹。
- 这类任务按原始 HTTP 文件传输，不根据来源网页触发视频提取或转码。视频旁的下载按钮仍走原有媒体解析流程。
- 图片 MIME 或 JPG/PNG/WebP/AVIF 等后缀归类为 `image`；新分类贯通后端、React 侧栏、目录设置与扩展展示。旧 `other` 图片记录只修正分类，不移动或重命名文件。
- React 与扩展任务卡复用 Windows 文件关联图标；无法获取时回退为图片/文件分类图标。
- HTTP 服务器忽略 Range 并返回 200 时，使用单连接流式完整下载；保留取消、文件大小上限与长度校验。无 Range 的任务重新开始传输，不宣称字节级续传。

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
