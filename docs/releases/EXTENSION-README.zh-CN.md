# Nexus Downloader — Chrome / Edge 扩展

本扩展随 Nexus Downloader v0.1.3 发布，扩展内部版本为 1.4.4。

1. 将 ZIP 完整解压到一个固定目录。
2. 打开 `chrome://extensions` 或 `edge://extensions`，开启开发者模式。
3. 点击“加载已解压的扩展程序”，选择含 `manifest.json` 的文件夹。
4. 启动 Nexus Downloader 托盘，扩展会连接本地后台服务。

无需输入配对码。先开扩展或先开托盘均可：后台未启动时扩展等待并自动重试，
后台启动后自动获取由 Rust 后台统一生成的令牌。浏览器休眠后通过定时唤醒继续重试。
如果一直等待，请确认完整解压、程序目录可写、托盘已启动，并检查提示中的浏览器桥接错误。

更新已有的已解压扩展时，覆盖同一目录中的扩展发行文件，再点击扩展管理页的刷新按钮。
如果已更换目录，请重新选择新目录。不要将 ZIP 直接拖入浏览器当作商店签名扩展安装。

扩展不是独立下载器，任务与文件下载由本地 Rust 后台统一管理。
本包不包含开发者的配对令牌、浏览器存储、设置或下载记录。
第三方许可证及源码地址见 `THIRD_PARTY_NOTICES.md` 和 `licenses/`。

项目：https://github.com/MuffinCloudpuff/Nexus-Downloader
