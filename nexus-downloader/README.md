# Nexus Downloader desktop UI

React interface and Tauri desktop/tray shell for the Rust download service.

See the [repository README](../README.md) for requirements, packaging, browser
extension setup, and testing. No Gemini API key or AI Studio deployment is
required to run this application.

```powershell
npm ci
npm run lint
npm run desktop:build -- --no-bundle -- --bin nexus-downloader
```

Place the built desktop executable beside `gd3-daemon.exe` before launching it.
