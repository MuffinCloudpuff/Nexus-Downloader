import {eraseDownloadFromHistory} from "./chrome-helpers";

type HandoffResult =
  | {historyCleared: true}
  | {historyCleared: false; cleanupError: unknown};

// The caller must first successfully cancel this exact browser download.
// A rejected handoff preserves its cancelled record for diagnosis/retry.
export async function handoffAndClearBrowserDownload(
  downloadItem: chrome.downloads.DownloadItem,
  handoff: (item: chrome.downloads.DownloadItem) => Promise<void>,
): Promise<HandoffResult> {
  const downloadId = downloadItem.id;
  await handoff(downloadItem);
  try {
    // This removes one history entry only, never a file or other downloads.
    await eraseDownloadFromHistory(downloadId);
    return {historyCleared: true};
  } catch (cleanupError) {
    // The daemon already accepted the task. Do not report handoff failure or
    // dispatch a duplicate task merely because browser history cleanup failed.
    return {historyCleared: false, cleanupError};
  }
}
