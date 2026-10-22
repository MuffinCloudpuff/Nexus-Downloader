# Nexus Downloader 0.1.3 — Windows x64 便携版

1. 将整个文件夹解压到可写目录，例如 `D:\Apps\NexusDownloader`，不要直接在压缩包内运行。
2. 如有旧版正在运行，请先从托盘菜单退出旧版，再运行 `Nexus Downloader.exe`。
3. 系统需要 Microsoft Edge WebView2 Runtime。程序目前没有代码签名。
4. 浏览器扩展单独下载并解压，在 Chrome / Edge 扩展管理页面开启开发者模式，
   加载含 `manifest.json` 的目录。保持托盘运行，扩展会连接本地服务。

关闭界面进入托盘模式会销毁 WebView，但后台下载继续运行；退出程序请使用托盘菜单。
本地服务地址为 `127.0.0.1:14371`。不要同时运行不同位置的两份程序。

本包包含托盘、Rust 后台、FFmpeg / ffprobe 8.1、Deno 2.8.2，不需要另外配置媒体工具 PATH。
yt-dlp 在首次需要时联网下载并校验，之后由程序管理更新。网站下载受权限和网络影响。

这是不含个人数据的干净发行包。首次运行会在程序目录生成自己的配置、配对令牌和任务数据。
不要分享运行后的 `config/`、`data/`、`logs/`、`downloads/` 等目录。
升级时只替换发行文件，保留自己的运行数据；移动安装目录后退出旧进程，再从新目录启动以更新浏览器连接入口。
令牌仅由常驻 Rust 后台生成，扩展无需手动配对；未连接时自动等待重试。
下载位置以设置或浏览器传入的保存路径为准，并不是由程序安装盘决定。
媒体临时文件位于下载目标旁，合并成功后清理；失败或暂停时可能保留供续传，删除任务时一起清理。

版本、来源及文件校验值见 `RELEASE-MANIFEST.json`。第三方许可证及源码地址见
`THIRD_PARTY_NOTICES.md` 和 `licenses/`。请仅下载你有权访问和保存的内容。

项目与更新：https://github.com/MuffinCloudpuff/Nexus-Downloader/releases
