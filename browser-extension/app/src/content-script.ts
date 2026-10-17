import {startPageMediaController} from "./page-media/controller";

(function installGhostDownloaderBridge() {
  const installKey = "__gd3ContentScriptInstalled";
  if (window.__gd3PageMedia?.resolveForElement) {
    return;
  }
  if ((window as unknown as Record<string, unknown>)[installKey]) {
    startPageMediaController();
    return;
  }
  (window as unknown as Record<string, unknown>)[installKey] = true;

  window.addEventListener("message", (event) => {
    const payload = event.data;
    if (!payload || typeof payload !== "object" || payload.action !== "catCatchToBackground") {
      return;
    }

    chrome.runtime.sendMessage({
      type: "bridge_page_command",
      payload,
    });
  });

  startPageMediaController();
})();
