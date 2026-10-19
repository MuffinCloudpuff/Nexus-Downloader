import {eraseDownloadFromHistory} from "./chrome-helpers";

type HandoffResult =
  | {historyCleared: true}
  | {historyCleared: false; cleanupError: unknown};

// The caller must first successfully cancel this exact browser download.
// A rejected handoff preserves its cancelled record for diagnosis/retry.
export async function handoffAndClearBrowserDownload(
  downloadItem: chrome.downloads.DownloadItem,
  handoff: (item: chrome.downloads.DownloadItem) => Promise<void | (() => Promise<void>)>,
): Promise<HandoffResult> {
  const downloadId = downloadItem.id;
  const start = await handoff(downloadItem);
  let result: HandoffResult;
  try {
    // This removes one history entry only, never a file or other downloads.
    await eraseDownloadFromHistory(downloadId);
    result = {historyCleared: true};
  } catch (cleanupError) {
    // The daemon already accepted the task. Do not report handoff failure or
    // dispatch a duplicate task merely because browser history cleanup failed.
    result = {historyCleared: false, cleanupError};
  }
  // Chrome can clean up a cancelled download's target when its record is
  // erased. Do not create the replacement file until that operation finishes.
  if (start) await start();
  return result;
}
