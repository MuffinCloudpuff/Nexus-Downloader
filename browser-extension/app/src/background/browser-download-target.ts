// onDeterminingFilename contains a suggested name and runs before Save As.
// Only a filename change exposes the browser-confirmed absolute destination.
export function isAbsoluteDownloadPath(path: string): boolean {
  if (/^\\\\[?.]\\/.test(path) || /(?:^|[\\/])\.\.(?:[\\/]|$)/.test(path)) return false;
  if (/^(?:[a-z]:|\\\\)/i.test(path) && path.slice(2).includes(":")) return false;
  return /^(?:[a-z]:[\\/]|\\\\[^\\/]+[\\/][^\\/]+[\\/]|\/)/i.test(path)
    && !/[\\/]$/.test(path) && !path.includes("\0");
}

export function getBrowserDownload(id: number): Promise<chrome.downloads.DownloadItem | undefined> {
  return new Promise((resolve) => {
    chrome.downloads.search({id}, (items) => {
      if (chrome.runtime.lastError) { resolve(undefined); return; }
      resolve(items[0]);
    });
  });
}

export function listenForConfirmedDownloadTargets(
  intercept: (item: chrome.downloads.DownloadItem) => Promise<void>,
) {
  const inFlight = new Set<number>();
  chrome.downloads.onDeterminingFilename?.addListener((_item, suggest) => suggest());
  chrome.downloads.onChanged.addListener((delta) => {
    if (!delta.filename?.current || !isAbsoluteDownloadPath(delta.filename.current)
      || delta.state?.current === "interrupted" || delta.state?.current === "complete"
      || inFlight.has(delta.id)) return;
    inFlight.add(delta.id);
    void (async () => {
      try {
        const item = await getBrowserDownload(delta.id);
        // Fast downloads may already be finished. Never cancel/re-download them.
        if (item?.state === "in_progress" && isAbsoluteDownloadPath(item.filename)) {
          await intercept(item);
        }
      } finally {
        inFlight.delete(delta.id);
      }
    })().catch(() => { /* Leave the browser download untouched on lookup failure. */ });
  });
}
