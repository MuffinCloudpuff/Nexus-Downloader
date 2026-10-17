# Desktop UI Lifecycle Plan

## Goal

The desktop tray process is the durable client shell. The React/WebView UI is a replaceable client surface and should only live while the main window is open.

Closing the main window in tray mode must release the WebView2/React runtime instead of only hiding it. Downloads, task state, settings, logs, and metrics remain owned by the daemon and must continue without the UI.

## Current Behavior

The Tauri close handler currently prevents close and calls `window.hide()`.

That keeps:

- WebView2 child processes
- React state
- task snapshot polling/SSE subscriptions
- frontend runtime memory

alive while the UI is visually hidden. This makes restore fast but keeps roughly the same UI memory footprint in Task Manager.

## Target Behavior

When the close behavior is tray mode:

1. The close request is intercepted.
2. The main WebView window is destroyed.
3. The Tauri tray shell remains alive.
4. Clicking the tray icon or choosing "Show" creates a fresh main WebView window if none exists.
5. The React UI boots from scratch and resynchronizes with the daemon through existing settings/task/metrics APIs and event streams.
6. The shell opens recreated UI windows centered on the screen.

When the close behavior is exit mode:

1. The close request is not intercepted.
2. The app exits normally.

## Module Boundaries

- `src-tauri/src/lib.rs`
  - Owns desktop window lifetime, tray behavior, and show/close UI commands.
  - Must not own download task state.
- `src/effects/desktop-shell-sync.tsx`
  - Maps UI setting `closeBehavior` to Tauri shell policy.
- `src/shell/app-store.tsx`
  - Owns React startup synchronization with daemon.
  - Must remain tolerant of fresh UI instances.
- Daemon
  - Owns durable task state, settings, logs, and execution.

## Implementation Notes

- Replace tray-mode `window.hide()` in the close handler with `window.destroy()`.
- Prevent the implicit app exit that Tauri requests after the final window is destroyed.
- Keep an explicit quit path so the tray menu can still exit the shell.
- Replace tray menu "Hide Window" with "Close UI".
- Reopen logic should:
  - focus an existing `main` window when present
  - otherwise create a new `main` WebView window from the same window config
- Recreated windows should always open centered. Do not keep in-memory UI position state.
- Keep the current protocol unchanged.
- Do not add frontend persistence to compensate for UI destruction; the daemon remains the source of truth.

## Verification

- Closing the main window in tray mode removes or significantly reduces WebView2 child processes.
- The tray icon remains available after closing the UI.
- Clicking the tray icon recreates the main window.
- The first and recreated windows open centered.
- Recreated UI reloads settings, task list, global speed, and storage information from the daemon.
- Choosing exit mode exits the application rather than keeping the tray shell alive.
