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
  intercept: (item: chrome.downloads.DownloadItem) => Promise<void | boolean>,
  options: {
    prepare?: (item: chrome.downloads.DownloadItem) => Promise<boolean>;
    completed?: (item: chrome.downloads.DownloadItem) => Promise<void>;
    onError?: (id: number, error: unknown) => void;
  } = {},
) {
  const inFlight = new Set<number>();
  const handled = new Set<number>();
  const imported = new Set<number>();
  const pausedByUs = new Set<number>();
  const determining = new Set<number>();
  // Download IDs grow for the lifetime of a browser profile. Bound transient
  // deduplication state instead of accumulating every historical download.
  function remember(set: Set<number>, id: number) {
    set.add(id);
    if (set.size > 2048) set.delete(set.values().next().value!);
  }
  async function resumeOwned(id: number) {
    if (!pausedByUs.delete(id)) return;
    await new Promise<void>(resolve => chrome.downloads.resume(id, () => {
      void chrome.runtime.lastError;
      resolve();
    }));
  }
  function inspect(id: number) {
    if (inFlight.has(id) || determining.has(id)) return;
    inFlight.add(id);
    void (async () => {
      let awaitingFilename = false;
      try {
        let item = await getBrowserDownload(id);
        if (!item) return;
        if (!isAbsoluteDownloadPath(item.filename)) {
          awaitingFilename = item.state === "in_progress" && !item.filename;
          return;
        }
        if (item.state === "in_progress" && !handled.has(id)) {
          if (item.paused && !pausedByUs.has(id)) return;
          if (await intercept(item) !== false) remember(handled, id);
          // Completion can race a backend capability/connection check. The
          // completion event may already have been coalesced while in flight.
          item = await getBrowserDownload(id);
        }
        if (!item) return;
        if (item.state === "complete" && !handled.has(id) && !imported.has(id)
          && item.exists !== false && options.completed) {
          // Blob/data URLs cannot be fetched by a separate Rust process. The
          // browser saves their bytes; import the existing file, never re-fetch
          // or transcode it. This also covers small HTTP files already finished.
          await options.completed(item);
          remember(imported, id);
        }
      } finally {
        inFlight.delete(id);
        if (!awaitingFilename) await resumeOwned(id);
      }
    })().catch(error => {
      // Keep the browser's file/record intact, but never make a failed import
      // indistinguishable from a successful transfer in diagnostics.
      options.onError?.(id, error);
    });
  }
  chrome.downloads.onDeterminingFilename?.addListener((item, suggest) => {
    // Hold filename determination until our pause has taken effect. Without
    // this gate a tiny TXT/ZIP/image can finish before onChanged is processed.
    // Do not cancel or choose a path here: Save As must still run normally.
    if (!options.prepare || !/^https?:/i.test(item.finalUrl || item.url)
      || item.paused || !chrome.downloads.pause) { suggest(); return; }
    determining.add(item.id);
    void (async () => {
      let timeout: ReturnType<typeof setTimeout> | undefined;
      try {
        const ready = await Promise.race([
          options.prepare!(item),
          new Promise<boolean>(resolve => { timeout = setTimeout(() => resolve(false), 2500); }),
        ]);
        if (ready) {
          await new Promise<void>(resolve => chrome.downloads.pause(item.id, () => {
            if (!chrome.runtime.lastError) pausedByUs.add(item.id);
            resolve();
          }));
        }
      } finally {
        if (timeout) clearTimeout(timeout);
        determining.delete(item.id);
        suggest(); // Exactly once, even on backend/Chrome errors.
        // The query after suggest may still have an empty filename while Save
        // As is open. The later filename/state event will inspect it again.
        inspect(item.id);
      }
    })().catch(() => { void resumeOwned(item.id); });
    return true;
  });
  chrome.downloads.onCreated?.addListener(item => {
    // Some downloads begin with an already confirmed target and no later
    // filename delta. Non-HTTP files are never paused/cancelled here.
    if (isAbsoluteDownloadPath(item.filename)) inspect(item.id);
  });
  chrome.downloads.onChanged.addListener(delta => {
    if (delta.state?.current === "interrupted") {
      pausedByUs.delete(delta.id);
      return;
    }
    if (delta.filename?.current || delta.state?.current || delta.paused?.current === false) inspect(delta.id);
  });
  chrome.downloads.onErased?.addListener(id => {
    handled.delete(id);
    imported.delete(id);
    pausedByUs.delete(id);
  });
}
