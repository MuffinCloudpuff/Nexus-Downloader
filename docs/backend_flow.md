# Rust Daemon 后端处理流程图

> 当前状态提示：本文是旧后端流程参考，部分 PackRegistry/WebSocket 描述已过期。当前正式架构以 `current-architecture-10-point-audit-and-roadmap.md`、`docs/adr/0001-http-commands-sse-task-events.md` 和 `docs/adr/0002-task-events-as-source-of-truth.md` 为准：HTTP commands 是正式命令入口，SSE task events 是正式观察通道，legacy WebSocket 只保留兼容。

这张图专门展示了纯后端链路：从接到请求开始，如何按站点或协议分流、解包资源、生成统一执行计划，并交给统一执行入口处理。

```mermaid
flowchart TD
    %% 定义样式
    classDef input fill:#f9f9f9,stroke:#333,stroke-width:2px;
    classDef core fill:#e1f5fe,stroke:#0288d1,stroke-width:2px;
    classDef pack fill:#fff3e0,stroke:#f57c00,stroke-width:2px;
    classDef engine fill:#e8f5e9,stroke:#388e3c,stroke-width:2px;
    classDef merge fill:#f3e5f5,stroke:#8e24aa,stroke-width:2px;

    %% 第一阶段：请求入口与路由
    Input([CreateTaskRequest 传入<br>url, headers, resources]) ::: input
    Registry{PackRegistry 匹配循环<br>按优先级调用 matches} ::: core

    Input --> Registry

    %% 第二阶段：按协议分发 (Feature Packs 解析层)
    subgraph 解析层 [Feature Packs: 解析视频流、提取底层资源 URL]
        direction TB

        BT[BitTorrentPack<br>匹配: magnet: / .torrent] ::: pack
        Bili[BilibiliDashPack<br>匹配: B站域名] ::: pack
        YT[YoutubePack<br>匹配: YT域名] ::: pack
        HLS[HlsPack<br>匹配: .m3u8] ::: pack
        GM[GenericMergePack<br>匹配: Extension 手动发来的2个资源] ::: pack
        HTTP[HttpPack<br>匹配: 兜底] ::: pack
    end

    Registry -- matches() == true --> BT
    Registry -- matches() == true --> Bili
    Registry -- matches() == true --> YT
    Registry -- matches() == true --> HLS
    Registry -- matches() == true --> GM
    Registry -- 兜底匹配 --> HTTP

    %% 解包逻辑
    BT -.直接交由 libtorrent 处理.-> BTEngine((BitTorrent<br>Engine)) ::: engine

    Bili -.请求 B 站 API<br>获取 DASH manifest.-> BiliStages(Task<br>execution_kind: DashMediaMerge<br>2 个 Stage: Video / Audio) ::: core
    YT -.解析网页/API.-> YTStages(Task<br>execution_kind: Youtube<br>1 个 Stage: 页面 URL) ::: core
    HLS -.解析 m3u8 播放列表.-> HLSStages(Task<br>execution_kind: Hls<br>1 个 Stage: m3u8 URL) ::: core
    GM -.读取 payload resources.-> GMStages(Task<br>execution_kind: DashMediaMerge<br>2 个 Stage: Video / Audio) ::: core
    HTTP -.直接使用原始 URL.-> HTTPStage(Task<br>execution_kind: HttpFile<br>单 Stage) ::: core

    %% 第三阶段：统一执行计划与入口
    subgraph 执行层 [Core Service 任务调度]
        TaskQueue[(Task Queue)] ::: core
        ExecutionPlan[ExecutionPlan<br>读取 Task.execution_kind] ::: core
        DownloadExecutor[DownloadExecutor<br>统一执行入口] ::: engine
    end

    BiliStages --> TaskQueue
    YTStages --> TaskQueue
    HLSStages --> TaskQueue
    GMStages --> TaskQueue
    HTTPStage --> TaskQueue

    TaskQueue -- "生成明确执行计划" --> ExecutionPlan
    ExecutionPlan --> DownloadExecutor

    %% 第四阶段：后处理与合并
    subgraph 后处理层 [Post Processing]
        UniversalDownloader[Universal Downloader<br>HTTP 分块 / HLS 分片下载] ::: engine
        YoutubeExec[Youtube 执行器<br>yt-dlp 解析 + 下载] ::: engine
        BTEngine((BitTorrent<br>Engine)) ::: engine
        FFmpeg[FFmpeg 混流合并<br>ffmpeg -i video -i audio -c copy] ::: merge
        TSMerge[TS 分片无缝拼接] ::: merge
    end

    DownloadExecutor -- "HttpFile / Hls" --> UniversalDownloader
    DownloadExecutor -- "DashMediaMerge" --> UniversalDownloader
    DownloadExecutor -- "Youtube" --> YoutubeExec
    DownloadExecutor -- "BitTorrent" --> BTEngine

    UniversalDownloader -- "Bili / GM 双轨资源下载完成" --> FFmpeg
    UniversalDownloader -- "HLS 多切片下载完成" --> TSMerge
    UniversalDownloader -- "Http 单文件" --> Done([下载完成]) ::: input
    YoutubeExec --> FFmpeg

    FFmpeg --> Done
    TSMerge --> Done
    BTEngine --> Done
```

### 流程拆解说明

1. **入口 (`CreateTaskRequest`)**
   扩展将页面抓取的 URL、请求头和 cookie 封成标准请求，通过 WebSocket 丢给后端。
2. **路由 (`PackRegistry`)**
   注册表维护了一个 Pack 列表（有先后优先级）。它会拿着这个请求挨个问每个 Pack：“这个协议/URL 你能处理吗？”
3. **解包 (`Feature Packs` 层)**
   - **Bilibili / Youtube**：它们拿到视频页的 URL 后，会在后台请求真正的 API，解析出底层的 CDN 下载地址，然后把视频流和音频流强行剥离开，分别装进 `Stage` 数据结构里（也就是“子任务”）。
   - **HLS**：负责拉取 `m3u8` 文件，把里面上百个 TS 小文件的 URL 提取出来，生成一组 `Stage`。
   - **GenericMerge**：扩展端已经拦截好了音视频真实地址，直接生成 2 个 `Stage`。
4. **执行计划 (`ExecutionPlan`)**
   Pack 解析完成后会产出带 `execution_kind` 的 `Task`。计划里用 `ExecutionKind` 表达它是普通 HTTP 文件、DASH 双轨合并、HLS、YouTube 还是 BitTorrent。`CoreService` 不再直接按 `PackId` 猜应该调用哪个下载器。
5. **统一执行入口 (`DownloadExecutor`)**
   `DownloadExecutor` 是后端唯一的下载执行入口。它根据 `ExecutionKind` 调用底层执行器：HTTP 类资源交给 `UniversalDownloader`，Bili/GenericMerge 这类双轨资源走 DASH 合并流程，YouTube 走 yt-dlp 执行流程，BitTorrent 保留独立协议执行入口。
6. **下载 (`UniversalDownloader`)**
   对最终可表示为 HTTP/HTTPS 资源的阶段，`UniversalDownloader` 负责多并发、重试、分块下载、HLS 分片下载，并实时上报 `0.0~1.0` 的进度。它不是所有协议细节的唯一承载者，而是 HTTP 类资源的底层下载器。
7. **合并 (Post Processing)**
   当所有 `Stage` 都被下载器安全拉回本地硬盘后，如果是分离的音视频流（Bili/YT/GM），调度器会调起 `FFmpeg` 强行把它们混流成一个 `.mp4`；如果是 HLS，就会把所有 `.ts` 按顺序拼接成一个完整视频。
