# Media Download Overlay Stabilization Plan

## 目标

恢复并稳定正式浏览器扩展里的页面悬浮下载按钮：

- 普通视频页面：按钮出现在可见 `<video>` 附近。
- 全屏视频：按钮出现在全屏元素内部的右上角。
- 点击按钮后：能够解析当前媒体、向 Rust daemon 创建任务、显示解析/下载/合并/完成/失败状态。
- 正式实现只放在 `browser-extension/app`。

不要继续扩展 `browser-extension/simple_ext`。`upstream` 只作为 Cat Catch 行为参考，不整套回滚或搬入。

## 当前判断

Gemini 当前计划方向基本正确，但还缺少几个硬约束：

- 必须确认加载的是 `app` 构建产物，而不是 `simple_ext`。
- 必须检查 overlay 脚本是否真实进入构建输出目录。
- 必须检查 `page_download_media` 的返回值是否包含 `taskId`，否则 overlay 无法追踪进度。
- 必须处理 all frames 多实例、全屏 DOM 迁移、SPA/延迟视频加载。
- 必须让失败创建任务时返回 `ok: false`，不能前端永远收到成功。

## 架构边界

正式主线：

- `browser-extension/app/src/background.ts`
- `browser-extension/app/src/background/resource-bridge.ts`
- `browser-extension/app/src/background/desktop-bridge.ts`
- `browser-extension/app/src/page-media/*`
- `browser-extension/app/public/catch-script/media-download-overlay.js`
- `browser-extension/app/public/manifest.json`

废弃线：

- `browser-extension/simple_ext`

参考线：

- `browser-extension/upstream`

## Phase 1：确认注入链路

先不要改 UI，先确认 overlay 真的运行。

1. 构建扩展：

   ```powershell
   cd browser-extension/app
   npm run typecheck
   npm run build
   ```

2. 检查构建输出目录必须包含：

   - `manifest.json`
   - `background.js`
   - `popup.html`
   - `catch-script/media-download-overlay.js`
   - `catch-script/mse-attribution.js`
   - `cat-catch-content-script.js`
   - `content-script.js`

3. 浏览器加载扩展时必须加载 `app` 的构建输出目录，不要加载 `simple_ext`。

4. 打开有视频的页面，在页面控制台确认：

   ```js
   window.GhostDownloaderMediaButton?.installed === true
   document.querySelector("#ghostDownloaderMediaDownload") !== null
   ```

5. 如果上面任一项失败，先修构建脚本或 manifest，不要继续改 overlay 行为。

## Phase 2：修正 overlay 可见性

修改 `app/public/catch-script/media-download-overlay.js`。

硬要求：

- 默认启用 overlay，除非 popup 设置明确关闭。
- 对可见 `<video>` 做选择，优先级为：正在播放 > readyState 高 > 可视面积大。
- 页面滚动、resize、play、pause、loadedmetadata、fullscreenchange、DOM 新增视频后都要重新定位。
- 普通模式按钮不要被视频容器裁剪，默认挂到 `document.documentElement`。
- 全屏模式必须把 host 移到 `document.fullscreenElement` 内部，否则浏览器原生全屏会遮挡扩展 DOM。
- `all_frames: true` 下可能多个 frame 注入。每个 frame 只管理自己文档内的视频，不要跨 frame 假设。

建议增加 `MutationObserver`：

```js
const observer = new MutationObserver(scheduleUpdate);
observer.observe(document.documentElement, {
  childList: true,
  subtree: true,
  attributes: true,
  attributeFilter: ["src", "style", "class"],
});
```

注意：MutationObserver 要节流，继续复用 `scheduleUpdate()`，不要每次 mutation 直接算布局。

## Phase 3：修正点击闭环

点击按钮后的链路应该是：

```text
media-download-overlay.js
  -> window.__gd3PageMedia.resolveForElement(...)
  -> chrome.runtime.sendMessage({ type: "page_download_media", ... })
  -> background.ts
  -> resource-bridge.downloadPageMedia(...)
  -> desktopBridge.sendRequest({ type: "create_task", ... })
  -> Rust daemon create_task_result
  -> overlay trackDownloadProgress(taskId)
```

必须检查 `resource-bridge.ts`：

- `downloadPageMedia()` 成功时必须返回 `{ ok: true, taskId, message }`。
- `sendHttpResourceToDesktop()`、`dispatchMergeResources()` 不能吞掉 daemon 返回的 `taskId`。
- daemon 返回 `ok: false` 时必须原样返回给 overlay。
- 没有选中媒体、没有 tab、没有资源、没有连接 daemon 时必须返回明确中文错误。

必须检查 `desktop-bridge.ts`：

- `create_task_result` 的 `requestId` 能匹配 pending request。
- 如果 WebSocket 未连接，`sendRequest` 应返回或抛出明确错误，不要让 overlay 一直“正在解析”。

必须检查 Rust daemon：

- `create_task` 解析失败时返回：

  ```json
  {
    "type": "create_task_result",
    "ok": false,
    "requestId": "...",
    "message": "..."
  }
  ```

- 创建成功时返回：

  ```json
  {
    "type": "create_task_result",
    "ok": true,
    "requestId": "...",
    "taskId": "..."
  }
  ```

当前如果失败仍返回 `ok: true`，必须修。

## Phase 4：状态与进度兼容

Rust daemon 新状态应统一为：

- `starting`
- `parsing`
- `downloading`
- `merging`
- `completed`
- `failed`

overlay 的显示建议：

- `starting`：`正在准备`
- `parsing`：`正在解析`
- `downloading`：显示速度和进度条
- `merging`：`正在合并`
- `completed`：`下载完成`
- `failed`：`下载失败`

进度单位长期应统一为 `0.0 ~ 1.0`。短期 overlay 必须兼容 `0~100`：

```js
const raw = Number(task.progress || 0);
const normalized = raw > 1 ? raw / 100 : raw;
const pct = Math.max(0, Math.min(100, normalized * 100));
```

如果修改后端，优先把 Rust daemon 所有 `update_task_status(... progress ...)` 改成 `0.0 ~ 1.0`。

## Phase 5：废弃 simple_ext

不要删除目录，先降低误用风险：

1. 在 `browser-extension/simple_ext/README.md` 新增说明：

   ```text
   This directory is deprecated experimental code.
   The official extension implementation is ../app.
   Do not add new features here.
   ```

2. 不要让构建脚本、文档、操作说明指向 `simple_ext`。

## Phase 6：验收

必须完成这些验证：

1. 命令验证：

   ```powershell
   cargo check
   cd browser-extension/app
   npm run typecheck
   npm run build
   ```

2. 页面验证：

   - 普通 HTML5 视频页面：按钮出现，点击后创建任务。
   - Bilibili 普通播放：按钮出现，点击后创建任务。
   - Bilibili 全屏播放：按钮出现在全屏右上角。
   - YouTube：按钮出现，点击后走 YouTube 任务。
   - X/Twitter：按钮出现，点击后走 HLS 或媒体资源任务。
   - 无视频网页：按钮不出现。

3. 状态验证：

   - 解析中显示 `正在解析` 或 `等待资源`。
   - 下载中显示速度和进度条。
   - 合并中显示 `正在合并`。
   - 完成后显示 `下载完成`，约 2 秒后恢复默认。
   - 失败后显示失败原因，约 2 秒后恢复默认。

4. 调试输出：

   - 页面控制台无 overlay runtime error。
   - background service worker 控制台无未处理 promise rejection。
   - daemon 日志能看到 `create_task` 和任务状态变化。

## 完成标准

这个任务完成的标准不是“代码改了”，而是：

- 用户在视频页面能看到悬浮下载按钮。
- 用户点击后能创建 daemon 任务。
- 按钮能看到解析、下载、合并、完成或失败状态。
- `simple_ext` 不再作为正式方案被使用。
