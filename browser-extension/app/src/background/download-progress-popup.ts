import {bridgeStorageSet, loadFromBridgeStorage} from "./chrome-helpers";
import {progressPopupPath} from "../shared/progress-popup";

const SEEN_TASKS_KEY = "progressPopupSeenTasks";
const MAX_RECENT_TASKS = 64;
const DEFAULT_POPUP = "popup.html";

function warnPopupUnavailable(error?: unknown) {
  console.warn("[Nexus] Download accepted; toolbar progress popup could not be opened",
    error instanceof Error ? error.message : "API unavailable");
}

// The automatic panel is the browser's toolbar popup, never a separate window.
// Its task-specific URL is temporary: manual clicks must retain the full list.
export function createDownloadProgressPopup() {
  const recent = new Set<string>();

  async function restoreDefaultPopup() {
    if (chrome.action?.setPopup) await chrome.action.setPopup({popup: DEFAULT_POPUP});
  }

  // Recover an override left behind if Chrome terminated the previous worker
  // between setPopup/openPopup and finally. Queue new opens behind this reset.
  let pending = restoreDefaultPopup().catch(warnPopupUnavailable);

  async function focusExistingPopup(taskId: string): Promise<boolean> {
    try {
      const response = await chrome.runtime.sendMessage({type: "progress_popup_focus", taskId});
      return response?.focused === true;
    } catch { return false; } // No automatically opened panel is listening.
  }

  async function showOnce(taskId: string) {
    if (!taskId.trim()) return;
    try {
      const state = await loadFromBridgeStorage<{[SEEN_TASKS_KEY]: string[]}>({[SEEN_TASKS_KEY]: []});
      if (Array.isArray(state[SEEN_TASKS_KEY])) {
        for (const id of state[SEEN_TASKS_KEY].slice(-MAX_RECENT_TASKS)) recent.add(id);
      }
    } catch { /* In-memory deduplication still works if storage is unavailable. */ }
    if (recent.has(taskId)) return;

    if (!await focusExistingPopup(taskId)) {
      // Older browsers/policies can prohibit automatic toolbar popups. Do not
      // substitute a standalone browser window or turn UI failure into a
      // failed download; the normal toolbar list remains available.
      if (!chrome.action?.setPopup || !chrome.action?.openPopup) {
        warnPopupUnavailable();
        return;
      }
      try {
        await chrome.action.setPopup({popup: progressPopupPath(taskId)});
        await chrome.action.openPopup();
      } finally {
        await restoreDefaultPopup().catch(warnPopupUnavailable);
      }
    }

    recent.add(taskId);
    const ids = [...recent].slice(-MAX_RECENT_TASKS);
    recent.clear();
    ids.forEach(id => recent.add(id));
    try { await bridgeStorageSet({[SEEN_TASKS_KEY]: ids}); } catch { /* Best effort. */ }
  }

  function show(taskId: string): Promise<void> {
    // Serialize temporary URL changes and keep all failures outside the
    // already successful download handoff.
    pending = pending.then(() => showOnce(taskId)).catch(warnPopupUnavailable);
    return pending;
  }
  return {show};
}
