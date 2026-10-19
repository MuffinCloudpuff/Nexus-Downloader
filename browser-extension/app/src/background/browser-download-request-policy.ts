// A download initiated by POST cannot safely be replayed as GET. Keep the
// browser's original request, then register its saved file with Rust instead.
// No extension/MIME/size allowlist belongs in this policy.
export function createBrowserDownloadRequestPolicy() {
  const nonGetRequests = new Map<string, number>();
  const lifetimeMs = 10 * 60 * 1000;
  return {
    record(url: string, method: string) {
      if (!/^https?:/i.test(url) || /^(GET|HEAD)$/i.test(method)) return;
      nonGetRequests.set(url, Date.now());
      if (nonGetRequests.size > 4096) nonGetRequests.delete(nonGetRequests.keys().next().value!);
    },
    canReplay(item: chrome.downloads.DownloadItem): boolean {
      const url = item.finalUrl || item.url;
      if (!/^https?:/i.test(url)) return false;
      const timestamp = nonGetRequests.get(url);
      if (timestamp == null) return true;
      if (Date.now() - timestamp > lifetimeMs) { nonGetRequests.delete(url); return true; }
      return false;
    },
  };
}
