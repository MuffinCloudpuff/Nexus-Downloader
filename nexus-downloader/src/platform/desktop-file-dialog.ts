type TauriWindow = Window &
  typeof globalThis & {
    __TAURI_INTERNALS__?: unknown;
    __TAURI__?: unknown;
  };

function isTauriRuntime(): boolean {
  const currentWindow = window as TauriWindow;
  return Boolean(currentWindow.__TAURI_INTERNALS__ || currentWindow.__TAURI__);
}

function joinBrowserFallbackPath(currentPath: string, folderName: string): string {
  const normalized = currentPath.replace(/\\/g, "/");
  const slashIndex = normalized.lastIndexOf("/");
  if (slashIndex >= 0) {
    return `${normalized.slice(0, slashIndex + 1)}${folderName}`;
  }
  return folderName;
}

async function pickDirectoryWithBrowserApi(currentPath: string): Promise<string | null> {
  if ("showDirectoryPicker" in window) {
    const handle = await (
      window as Window & {
        showDirectoryPicker?: () => Promise<{ name: string }>;
      }
    ).showDirectoryPicker?.();
    if (handle?.name) {
      return joinBrowserFallbackPath(currentPath, handle.name);
    }
  }
  return null;
}

export async function pickDirectory(currentPath: string): Promise<string | null> {
  if (isTauriRuntime()) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      directory: true,
      multiple: false,
      defaultPath: currentPath || undefined,
    });
    return typeof selected === "string" ? selected : null;
  }

  return pickDirectoryWithBrowserApi(currentPath);
}

export function directoryFromFileInput(currentPath: string, relativePath: string): string | null {
  const folderName = relativePath.split("/")[0];
  if (!folderName) {
    return null;
  }
  return joinBrowserFallbackPath(currentPath, folderName);
}
