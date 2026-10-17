# Nexus Downloader

A Rust-powered downloader with a React UI that unloads its WebView when closed,
keeping only the tray service running. Download videos directly from inline
buttons on YouTube, Bilibili, and X.

基于 **Rust + React + Tauri** 的 Windows 下载器。关闭界面进入托盘模式时销毁
WebView，只保留下载服务；在 YouTube、哔哩哔哩和 X 的视频旁直接显示下载按钮。
托盘、桌面界面和浏览器扩展共用同一份任务状态。

## Highlights

- **Rust download engine:** adaptive HTTP range connections, resumable file
  downloads, HLS segments, and separate video/audio merging.
- **On-demand React UI:** closing the interface in tray mode unloads its WebView;
  the tray service and active downloads continue running.
- **Inline video downloads:** download buttons appear beside supported videos on
  YouTube, Bilibili, and X. Availability depends on the site and media access.
- **One task list:** the backend owns task state; the React UI and extension
  synchronize with it. Deleting a task removes it from both interfaces.
- **Managed YouTube extractor:** yt-dlp extracts media metadata; Nexus's Rust
  engine transfers the media. Managed executable updates are verified before activation.
- **Shared branding:** the desktop tray and browser extension use the same icons.

## Source layout

```text
src/                        Rust backend, engines and localhost API
tests/                      Backend and audio/video regression tests
nexus-downloader/           React UI and Tauri desktop/tray shell
browser-extension/app/      Nexus browser extension source
browser-extension/upstream/ Cat Catch dependency, pinned Git submodule
docs/                       Architecture notes and design history
third-party/                Retained third-party license notices
```

The legacy Ghost Python application, abandoned WinUI implementation, old UI
prototype, installed executables, downloaded media, and local runtime data are
not part of this repository snapshot.

## Requirements

The current desktop/backend implementation targets **Windows**.

- Rust stable, Windows MSVC toolchain, and Microsoft C++ Build Tools.
- Node.js 22 or newer and npm.
- Microsoft Edge WebView2 Runtime for the desktop interface.
- FFmpeg and ffprobe on `PATH` for media merging and verification.
- Deno on `PATH`, or `deno.exe` beside `gd3-daemon.exe`, for YouTube extraction.
- Git, including the Cat Catch submodule below.

The tray application must be running for the browser extension to submit or
manage downloads. The Rust backend is the download service.

## Get the source

```powershell
git clone --recurse-submodules https://github.com/MuffinCloudpuff/Nexus-Downloader.git
cd Nexus-Downloader
```

If you cloned without submodules, run `git submodule update --init --recursive`.

## Build the backend and desktop application

From the repository root:

```powershell
cargo build --release --bin gd3-daemon
cd nexus-downloader
npm ci
npm run desktop:build -- --no-bundle -- --bin nexus-downloader
cd ..
```

Assemble a portable folder. Close any existing copy before replacing its executables:

```powershell
New-Item -ItemType Directory -Force dist-portable/NexusDownloaderPortable
Copy-Item target/release/gd3-daemon.exe dist-portable/NexusDownloaderPortable/
Copy-Item nexus-downloader/src-tauri/target/release/nexus-downloader.exe 'dist-portable/NexusDownloaderPortable/Nexus Downloader.exe'
```

Start `Nexus Downloader.exe`. It starts the adjacent backend and provides tray
controls. Choose **Minimize to Tray** in Settings to keep downloads running when
closing the UI; choosing **Exit App** exits the application.

The backend listens on `127.0.0.1:14371`. Pairing and settings remain local;
never publish generated `config/`, `data/`, or `logs/` folders.

## Build and load the browser extension

```powershell
cd browser-extension/app
npm ci
npm run build
```

For Chrome or Edge, enable Developer mode on the extension management page and
load `browser-extension/chromium` as an unpacked extension. Keep Nexus Downloader
running; the extension pairs with the local desktop service.

The build also generates `browser-extension/firefox`. Its temporary-addon loading
can be used for development, but browser-specific native-messaging setup must be
available before desktop pairing can work.

Internal extension/native-host identifiers remain stable for compatibility;
historical internal names do not select a Python backend.

## Checks

```powershell
cargo test --all-targets
cd nexus-downloader
npm ci
npm run lint
npm run build
cd ../browser-extension/app
npm ci
npm run typecheck
npm run test:overlay
npm run test:pairing
npm run test:popup
npm run test:icons
npm run test:branding
```

Media regression tests synthesize local audio/video with FFmpeg and inspect the
result with ffprobe. They cover separate HLS audio, embedded audio, direct media
playlists, intentional video-only streams, and missing-audio failures. The
external yt-dlp release-download test is opt-in and ignored by default.

## Third-party components

See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for provenance and retained
licenses. Download only content you are authorized to access, subject to the
source site's terms.
