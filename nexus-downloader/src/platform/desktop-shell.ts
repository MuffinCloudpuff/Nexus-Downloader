type TauriWindow = Window &
  typeof globalThis & {
    __TAURI_INTERNALS__?: unknown;
    __TAURI__?: unknown;
  };

function isTauriRuntime(): boolean {
  const currentWindow = window as TauriWindow;
  return Boolean(currentWindow.__TAURI_INTERNALS__ || currentWindow.__TAURI__);
}

export async function revealPathInDesktop(path?: string): Promise<boolean> {
  if (!path || !isTauriRuntime()) {
    return false;
  }
  const { revealItemInDir, openPath } = await import("@tauri-apps/plugin-opener");
  try {
    await revealItemInDir(path);
  } catch {
    await openPath(path);
  }
  return true;
}

export async function notifyDownloadCompleted(title: string): Promise<boolean> {
  if (!isTauriRuntime()) {
    return false;
  }
  const notification = await import("@tauri-apps/plugin-notification");
  let allowed = await notification.isPermissionGranted();
  if (!allowed) {
    const permission = await notification.requestPermission();
    allowed = permission === "granted";
  }
  if (!allowed) {
    return false;
  }
  notification.sendNotification({
    title: "Download completed",
    body: title,
  });
  return true;
}

export async function syncCloseBehavior(behavior: "hide_to_tray" | "exit_app"): Promise<boolean> {
  if (!isTauriRuntime()) {
    return false;
  }
  const { emit } = await import("@tauri-apps/api/event");
  await emit("gd3://close-behavior-changed", behavior);
  return true;
}

export async function getLaunchOnStartup(): Promise<boolean | null> {
  if (!isTauriRuntime()) {
    return null;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<boolean>("gd3_get_autostart_enabled");
}

export async function syncLaunchOnStartup(enabled: boolean): Promise<boolean> {
  if (!isTauriRuntime()) {
    return false;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke<boolean>("gd3_set_autostart_enabled", { enabled });
  return true;
}
