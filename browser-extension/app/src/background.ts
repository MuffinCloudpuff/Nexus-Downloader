import type {
    AdvancedFeatureKey,
    DesktopRequestResult,
    GenericTaskSummary,
    PopupStatePayload,
    PopupView,
} from "./shared/types";
import {createDaemonClient} from "./background/daemon-client";
import {createDownloadProgressPopup} from "./background/download-progress-popup";
import {createFeatureBridge} from "./background/feature-bridge";
import {createMediaBridge} from "./background/media-bridge";
import {createResourceBridge} from "./background/resource-bridge";
import {handoffAndClearBrowserDownload} from "./background/browser-download-handoff";
import {createBrowserDownloadRequestPolicy} from "./background/browser-download-request-policy";
import {createTaskEventSubscriptions} from "./background/task-event-subscriptions";
import {createTaskStore} from "./background/task-store";
import {
    createClientTelemetry,
    createTraceId,
    summarizeTaskSnapshot,
    taskSnapshotSignature,
    urlHost,
    urlPathKind,
} from "./background/client-telemetry";
import {INTERCEPT_DOWNLOADS_KEY, MEDIA_DOWNLOAD_OVERLAY_KEY,} from "./background/constants";
import {
    cancelDownload,
    eraseDownloadFromHistory,
    findTab,
    loadFromLocalStorage,
    queryTabs,
} from "./background/chrome-helpers";
import {onSendHeadersExtraInfoSpec} from "./shared/browser";
import {getBrowserDownload, isAbsoluteDownloadPath, listenForConfirmedDownloadTargets} from "./background/browser-download-target";
import type {ClientTelemetryEvent, CreateTaskHttpReq} from "./shared/protocol/daemon";
import {messageForDaemonError, messageFromUnknownError} from "./shared/protocol/error-messages";

function notifyTaskSnapshot(tasks: GenericTaskSummary[]) {
  const message = { type: "task_snapshot", tasks };
  recordTaskSnapshotTelemetry(tasks);

  chrome.runtime.sendMessage(message, () => {
    void chrome.runtime.lastError;
  });

  sendTaskMessageToTabs(message);
}

function sendTaskMessageToTabs(message: Record<string, unknown>) {
  void queryTabs({}).then((tabs) => {
    for (const tab of tabs) {
      if (!tab.id || !tab.url || !/^https?:/i.test(tab.url)) {
        continue;
      }
      chrome.tabs.sendMessage(tab.id, message, () => {
        void chrome.runtime.lastError;
      });
    }
  });
}

function notifyTaskUpdated(task: GenericTaskSummary | null) {
  if (!task) {
    return;
  }
  const message = { type: "task_updated", task };
  chrome.runtime.sendMessage(message, () => {
    void chrome.runtime.lastError;
  });
  sendTaskMessageToTabs(message);
}

const daemonClient = createDaemonClient();
const downloadProgressPopup = createDownloadProgressPopup();
const browserDownloadRequests = createBrowserDownloadRequestPolicy();
const clientTelemetry = createClientTelemetry((events) => daemonClient.sendClientEvents(events));
const taskStore = createTaskStore();
const taskSubscriptions = createTaskEventSubscriptions({
  client: daemonClient,
  taskStore,
  onTasksChanged: notifyTaskSnapshot,
  onTaskUpdated: notifyTaskUpdated,
});

let lastTaskSnapshotSignature = "";

function recordClientTelemetry(event: ClientTelemetryEvent): void {
  clientTelemetry.record(event);
}

function recordTaskSnapshotTelemetry(tasks: GenericTaskSummary[]): void {
  const signature = taskSnapshotSignature(tasks);
  if (signature === lastTaskSnapshotSignature) {
    return;
  }
  lastTaskSnapshotSignature = signature;
  recordClientTelemetry({
    event_type: "client.task_snapshot_broadcast",
    surface: "background",
    action: "task_snapshot",
    status: "snapshot",
    details: summarizeTaskSnapshot(tasks),
  });
}

function ensureRequestTrace(request: CreateTaskHttpReq, surface: string): string {
  const existing = request.client_context?.trace_id;
  const traceId = existing || createTraceId(surface);
  request.client_context = {
    ...(request.client_context ?? {}),
    extension_id: request.client_context?.extension_id || chrome.runtime.id,
    trace_id: traceId,
  };
  return traceId;
}

async function refreshDaemonTasks(): Promise<GenericTaskSummary[]> {
  const daemonTasks = await daemonClient.connect();
  taskStore.replaceSnapshot(daemonTasks);
  const tasks = taskStore.getTasks();
  notifyTaskSnapshot(tasks);
  taskSubscriptions.syncWithActiveTasks();
  return tasks;
}

async function createDaemonTask(request: CreateTaskHttpReq): Promise<DesktopRequestResult> {
  const traceId = ensureRequestTrace(request, String(request.source || "task"));
  recordClientTelemetry({
    event_type: "client.dispatch",
    trace_id: traceId,
    request_id: request.request_id,
    surface: "background",
    action: "create_task",
    source: request.source,
    status: "started",
    tab_id: request.client_context?.tab_id,
    page_url: request.client_context?.page_url,
    url_host: urlHost(request.payload.url),
    url_path_kind: urlPathKind(request.payload.url),
    details: {
      title: request.title,
      hasResources: Array.isArray(request.payload.resources) && request.payload.resources.length > 0,
      resourceCount: request.payload.resources?.length ?? 0,
      hasHeaders: Boolean(request.payload.headers && Object.keys(request.payload.headers).length > 0),
      headerCount: Object.keys(request.payload.headers ?? {}).length,
      headerNames: Object.keys(request.payload.headers ?? {}).sort(),
    },
  });

  try {
    const response = await daemonClient.createTask(request);
    if (!response.accepted || !response.task_id) {
      recordClientTelemetry({
        event_type: "client.dispatch_result",
        trace_id: response.trace_id || traceId,
        request_id: response.request_id || request.request_id,
        surface: "background",
        action: "create_task",
        source: request.source,
        status: "failed",
        message: messageForDaemonError(response.error, "任务创建失败"),
        tab_id: request.client_context?.tab_id,
        page_url: request.client_context?.page_url,
        url_host: urlHost(request.payload.url),
        url_path_kind: urlPathKind(request.payload.url),
        details: { error: response.error },
      });
      return {
        ok: false,
        requestId: response.request_id || request.request_id,
        traceId: response.trace_id || traceId,
        message: messageForDaemonError(response.error, "任务创建失败"),
      };
    }

    const acceptedTask = taskStore.upsertAccepted(response, request);
    if (acceptedTask) {
      notifyTaskUpdated(acceptedTask);
    }
    taskSubscriptions.subscribeTask(response.task_id, response.event_cursor ?? 0);
    if (!request.payload.defer_start) void downloadProgressPopup.show(response.task_id);
    void refreshDaemonTasks().catch(() => {
      // The accepted task is already in the local store; refresh is best-effort.
    });

    recordClientTelemetry({
      event_type: "client.dispatch_result",
      trace_id: response.trace_id || traceId,
      request_id: response.request_id,
      task_id: response.task_id,
      surface: "background",
      action: "create_task",
      source: request.source,
      status: "accepted",
      message: "任务已发送到 Nexus Downloader",
      tab_id: request.client_context?.tab_id,
      page_url: request.client_context?.page_url,
      url_host: urlHost(request.payload.url),
      url_path_kind: urlPathKind(request.payload.url),
      details: {
        eventCursor: response.event_cursor ?? 0,
        initialStatus: response.initial_status,
      },
    });

    return {
      ok: true,
      taskId: response.task_id,
      requestId: response.request_id,
      traceId: response.trace_id || traceId,
      eventCursor: response.event_cursor,
      message: "任务已发送到 Nexus Downloader",
    };
  } catch (error) {
    recordClientTelemetry({
      event_type: "client.dispatch_result",
      trace_id: traceId,
      request_id: request.request_id,
      surface: "background",
      action: "create_task",
      source: request.source,
      status: "failed",
      message: messageFromUnknownError(error, "任务创建失败"),
      tab_id: request.client_context?.tab_id,
      page_url: request.client_context?.page_url,
      url_host: urlHost(request.payload.url),
      url_path_kind: urlPathKind(request.payload.url),
    });
    return {
      ok: false,
      requestId: request.request_id,
      traceId,
      message: messageFromUnknownError(error, "任务创建失败"),
    };
  }
}

async function performDaemonTaskAction(taskId: string, action: string, traceId = createTraceId("task_action")): Promise<DesktopRequestResult> {
  const task = taskStore.getTask(taskId);
  let command: "pause" | "resume" | "cancel" | "delete";

  if (action === "cancel") {
    command = "delete";
  } else if (action === "toggle_pause") {
    command = task?.status === "paused" || task?.status === "failed" ? "resume" : "pause";
  } else if (action === "pause" || action === "resume") {
    command = action;
  } else if (action === "redownload") {
    command = "resume";
  } else {
    return { ok: false, message: "当前操作暂不支持" };
  }

  recordClientTelemetry({
    event_type: "client.dispatch",
    trace_id: traceId,
    request_id: task?.requestId,
    task_id: taskId,
    surface: "background",
    action,
    status: "started",
    details: { command, currentStatus: task?.status },
  });

  try {
    const response = await daemonClient.sendTaskCommand(taskId, command);
    if (!response.command_accepted) {
      recordClientTelemetry({
        event_type: "client.dispatch_result",
        trace_id: traceId,
        request_id: task?.requestId,
        task_id: taskId,
        surface: "background",
        action,
        status: "failed",
        message: messageForDaemonError(response.error, "任务操作失败"),
        details: { command, error: response.error },
      });
      return {
        ok: false,
        requestId: task?.requestId,
        traceId,
        message: messageForDaemonError(response.error, "任务操作失败"),
      };
    }
    if (command === "delete") {
      // Match the daemon's deletion contract immediately. The subsequent SSE event
      // and snapshot refresh remain authoritative fallbacks if another client wins
      // the race or the daemon rejects a later state transition.
      taskStore.removeTask(taskId);
      notifyTaskSnapshot(taskStore.getTasks());
    } else if (command === "resume") {
      taskSubscriptions.subscribeTask(taskId, taskStore.getCursor(taskId));
    }
    void refreshDaemonTasks().catch(() => {
      // Command result is already returned; snapshot refresh is best-effort.
    });
    recordClientTelemetry({
      event_type: "client.dispatch_result",
      trace_id: traceId,
      request_id: task?.requestId,
      task_id: taskId,
      surface: "background",
      action,
      status: "accepted",
      message: "任务操作已发送",
      details: { command },
    });
    return { ok: true, requestId: task?.requestId, traceId, message: "任务操作已发送" };
  } catch (error) {
    recordClientTelemetry({
      event_type: "client.dispatch_result",
      trace_id: traceId,
      request_id: task?.requestId,
      task_id: taskId,
      surface: "background",
      action,
      status: "failed",
      message: messageFromUnknownError(error, "任务操作失败"),
      details: { command },
    });
    return { ok: false, requestId: task?.requestId, traceId, message: messageFromUnknownError(error, "任务操作失败") };
  }
}

const resourceBridge = createResourceBridge({
  createTask: createDaemonTask,
  startBrowserTask: async id => {
    const result = await daemonClient.sendTaskCommand(id, "start");
    if (!result.command_accepted) throw new Error("浏览器任务已创建，但后台尚未启动传输");
    // Browser history cleanup has finished before this callback. Opening earlier
    // would expose a resume button while Chrome still owns target-file cleanup.
    void downloadProgressPopup.show(id);
  },
  recordTelemetry: recordClientTelemetry,
});
const featureBridge = createFeatureBridge();
const mediaBridge = createMediaBridge();

let interceptDownloads = true;
let mediaDownloadOverlayEnabled = true;

async function injectMediaDownloadOverlay(tabId: number) {
  if (!mediaDownloadOverlayEnabled) {
    return;
  }
  const tab = await findTab(tabId);
  if (!tab?.url || !/^https?:/i.test(tab.url)) {
    return;
  }

  try {
    await chrome.scripting.executeScript({
      files: ["content-script.js"],
      injectImmediately: true,
      target: { tabId, allFrames: true },
    });
    await chrome.scripting.executeScript({
      files: ["catch-script/overlay-media-state.js", "catch-script/media-download-overlay.js"],
      injectImmediately: false,
      target: { tabId, allFrames: true },
    });
  } catch {
    // chrome:// and similar reject injection.
  }
}

async function updateMediaDownloadOverlay(enabled: boolean) {
  mediaDownloadOverlayEnabled = enabled;
  await chrome.storage.local.set({ [MEDIA_DOWNLOAD_OVERLAY_KEY]: enabled });

  const tabs = await queryTabs({});
  for (const tab of tabs) {
    if (!tab.id || !tab.url || !/^https?:/i.test(tab.url)) {
      continue;
    }
    chrome.tabs.sendMessage(tab.id, {
      type: "media_download_overlay_set_enabled",
      enabled,
    }, () => {
      const lastError = chrome.runtime.lastError;
      if (enabled && lastError && tab.id) {
        void injectMediaDownloadOverlay(tab.id);
      }
    });
  }
}

function taskCounters(tasks: GenericTaskSummary[]) {
  return {
    total: tasks.length,
    active: tasks.filter((task) => !["completed", "failed", "canceled"].includes(task.status)).length,
    completed: tasks.filter((task) => task.status === "completed").length,
  };
}

async function buildPopupState(options: {
  preferredTabId?: number | null;
  currentView?: PopupView;
} = {}): Promise<PopupStatePayload> {
  const resolvedTabId = await resourceBridge.resolveActiveTabId(options.preferredTabId ?? null);
  const activeTab = resolvedTabId != null ? await findTab(resolvedTabId) : null;
  const desktopState = daemonClient.buildSnapshot();
  const tasks = taskStore.getTasks();
  const resourceState = resourceBridge.buildPopupStateData(resolvedTabId, activeTab);

  const mediaPanelState = await mediaBridge.buildPanelState(
    options.currentView === "advanced" ? resolvedTabId : null,
  );

  return {
    connectionState: desktopState.connectionState,
    connectionMessage: desktopState.connectionMessage,
    desktopVersion: desktopState.desktopVersion,
    token: desktopState.token,
    serverUrl: desktopState.serverUrl,
    interceptDownloads,
    mediaDownloadOverlayEnabled,
    tasks,
    taskCounters: taskCounters(tasks),
    tabId: resolvedTabId,
    featureStates: featureBridge.createFeatureStateMap(resolvedTabId),
    mediaItems: mediaPanelState.mediaItems,
    mediaPlaybackState: mediaPanelState.playbackState,
    ...resourceState,
  };
}

async function initialize() {
  const localState = await loadFromLocalStorage<{
    [INTERCEPT_DOWNLOADS_KEY]: boolean;
    [MEDIA_DOWNLOAD_OVERLAY_KEY]: boolean;
  }>({
    [INTERCEPT_DOWNLOADS_KEY]: true,
    [MEDIA_DOWNLOAD_OVERLAY_KEY]: true,
  });

  interceptDownloads = Boolean(localState[INTERCEPT_DOWNLOADS_KEY] ?? true);
  mediaDownloadOverlayEnabled = Boolean(localState[MEDIA_DOWNLOAD_OVERLAY_KEY] ?? true);

  await daemonClient.loadPersistentState();
  await taskStore.loadPersistentState();
  await resourceBridge.loadPersistentState();
  await featureBridge.loadPersistentState();
  notifyTaskSnapshot(taskStore.getTasks());
  const activeTabId = await resourceBridge.resolveActiveTabId();
  if (activeTabId != null) {
    void injectMediaDownloadOverlay(activeTabId);
  }

  // connect() obtains the native token when needed, including on a fresh install.
  void refreshDaemonTasks().catch(() => {
    notifyTaskSnapshot(taskStore.getTasks());
  });

  daemonClient.ensureReconnectAlarm();

  // Also probe an apparently connected daemon: a closed idle SSE connection
  // must not leave the popup permanently reporting an old connected state.
  // connect() coalesces overlapping probes and pairs only when necessary.
  setInterval(() => {
    const state = daemonClient.buildSnapshot().connectionState;
    if (state !== "connecting") {
      void refreshDaemonTasks().catch(() => {
        notifyTaskSnapshot(taskStore.getTasks());
      });
    }
  }, 5000);
}

chrome.alarms.onAlarm.addListener((alarm) => {
  daemonClient.onReconnectAlarm(alarm, () => {
    void refreshDaemonTasks().catch(() => {
      notifyTaskSnapshot(taskStore.getTasks());
    });
  });
});

chrome.runtime.onConnect.addListener((port) => {
  if (port.name === "HeartBeat") {
    port.postMessage("HeartBeat");
  }
});

chrome.storage.onChanged.addListener((changes, areaName) => {
  if (areaName !== "local") {
    return;
  }
  daemonClient.onLocalStorageChanged(changes);
  if (changes[INTERCEPT_DOWNLOADS_KEY]) {
    interceptDownloads = Boolean(changes[INTERCEPT_DOWNLOADS_KEY].newValue ?? true);
  }
  if (changes[MEDIA_DOWNLOAD_OVERLAY_KEY]) {
    mediaDownloadOverlayEnabled = Boolean(changes[MEDIA_DOWNLOAD_OVERLAY_KEY].newValue ?? true);
  }
});

chrome.tabs.onActivated.addListener((activeInfo) => {
  void resourceBridge.setLastActiveTab(activeInfo.tabId);
  void injectMediaDownloadOverlay(activeInfo.tabId);
});

chrome.windows.onFocusChanged.addListener((windowId) => {
  if (windowId === chrome.windows.WINDOW_ID_NONE) {
    return;
  }
  void resourceBridge.refreshActiveTabFromBrowser().then((tabId) => {
    if (tabId != null) {
      void injectMediaDownloadOverlay(tabId);
    }
  });
});

chrome.tabs.onRemoved.addListener((tabId) => {
  resourceBridge.onTabRemoved(tabId);
  mediaBridge.onTabRemoved(tabId);
  featureBridge.onTabRemoved(tabId);
});

chrome.webNavigation.onCommitted.addListener((details) => {
  resourceBridge.onNavigationCommitted(details);
  featureBridge.onNavigationCommitted(details);
});

chrome.webRequest.onBeforeSendHeaders.addListener(
  (details) => {
    browserDownloadRequests.record(details.url, details.method);
    resourceBridge.onRequestHeaders(details);
    return undefined;
  },
  { urls: ["<all_urls>"] },
  onSendHeadersExtraInfoSpec() as unknown as chrome.webRequest.OnBeforeSendHeadersOptions[],
);

chrome.webRequest.onSendHeaders.addListener(
  (details) => {
    resourceBridge.onRequestHeaders(details);
    void resourceBridge.captureRequestResource(details);
  },
  { urls: ["<all_urls>"] },
  onSendHeadersExtraInfoSpec(),
);

chrome.webRequest.onResponseStarted.addListener(
  (details) => {
    void resourceBridge.captureNetworkResource(details);
  },
  { urls: ["<all_urls>"] },
  ["responseHeaders"],
);

async function interceptBrowserDownload(
  downloadItem: chrome.downloads.DownloadItem,
): Promise<boolean> {
  const traceId = createTraceId("browser_download_intercept");
  const finalUrl = downloadItem.finalUrl || downloadItem.url;
  const commonTelemetry = {
    trace_id: traceId,
    surface: "background",
    action: "intercept_browser_download",
    source: "download",
    url_host: urlHost(finalUrl),
    url_path_kind: urlPathKind(finalUrl),
    details: {
      downloadId: downloadItem.id,
      filename: downloadItem.filename,
      totalBytes: downloadItem.totalBytes,
      state: downloadItem.state,
      connectionState: daemonClient.buildSnapshot().connectionState,
    },
  };

  if (!interceptDownloads) {
    recordClientTelemetry({
      ...commonTelemetry,
      event_type: "client.intercept_decision",
      status: "skipped",
      message: "browser download intercept is disabled",
      details: { ...commonTelemetry.details, reason: "disabled" },
    });
    return false;
  }
  if (!browserDownloadRequests.canReplay(downloadItem)) {
    recordClientTelemetry({
      ...commonTelemetry,
      event_type: "client.intercept_decision",
      status: "skipped",
      message: "browser will save this local or non-replayable file before Rust import",
      details: { ...commonTelemetry.details, reason: "browser_file_import_after_completion" },
    });
    return false;
  }

  if (downloadItem.state !== "in_progress" || !isAbsoluteDownloadPath(downloadItem.filename)) return false;

  if (!daemonClient.isReady()) {
    if (daemonClient.hasToken()) {
      try {
        recordClientTelemetry({
          ...commonTelemetry,
          event_type: "client.intercept_decision",
          status: "connecting",
          message: "daemon is not connected; reconnecting before intercept",
          details: { ...commonTelemetry.details, reason: "daemon_not_connected" },
        });
        await daemonClient.connect();
      } catch (error) {
        recordClientTelemetry({
          ...commonTelemetry,
          event_type: "client.intercept_decision",
          status: "skipped",
          message: messageFromUnknownError(error, "daemon reconnect failed before intercept"),
          details: { ...commonTelemetry.details, reason: "daemon_reconnect_failed" },
        });
        return false;
      }
    }
  }

  if (!daemonClient.isReady()) {
    recordClientTelemetry({
      ...commonTelemetry,
      event_type: "client.intercept_decision",
      status: "skipped",
      message: "daemon is not ready for browser download intercept",
      details: { ...commonTelemetry.details, reason: "daemon_not_ready" },
    });
    return false;
  }

  recordClientTelemetry({
    ...commonTelemetry,
    event_type: "client.intercept_decision",
    status: "started",
    message: "browser download intercept started",
  });

  try {
    // Older daemons silently ignore unknown fields: leave the browser in
    // charge unless the backend explicitly supports exact destinations.
    if (!await daemonClient.supportsBrowserSavePath()) return false;
    const latest = await getBrowserDownload(downloadItem.id);
    if (latest?.state !== "in_progress" || !isAbsoluteDownloadPath(latest.filename)) return false;
    downloadItem = latest;
    await cancelDownload(downloadItem.id);
  } catch (error) {
    recordClientTelemetry({
      ...commonTelemetry,
      event_type: "client.intercept_decision",
      status: "skipped",
      message: messageFromUnknownError(error, "browser download cancel failed"),
      details: { ...commonTelemetry.details, reason: "cancel_failed" },
    });
    // Cleanup failed but the browser will still finish the download as fallback.
    return false;
  }

  try {
    const cleanup = await handoffAndClearBrowserDownload(
      downloadItem,
      resourceBridge.handoffBrowserDownload,
    );
    recordClientTelemetry({
      ...commonTelemetry,
      event_type: "client.intercept_decision",
      status: "accepted",
      message: "browser download handed off to daemon",
    });
    recordClientTelemetry({
      ...commonTelemetry,
      event_type: "client.intercept_cleanup",
      status: cleanup.historyCleared ? "cleared" : "warning",
      message: cleanup.historyCleared
        ? "intercepted browser download history entry cleared"
        : messageFromUnknownError(cleanup.cleanupError, "browser history cleanup failed after successful handoff"),
      details: {...commonTelemetry.details, reason: cleanup.historyCleared ? "handoff_accepted" : "history_cleanup_failed"},
    });
    return true;
  } catch (error) {
    recordClientTelemetry({
      ...commonTelemetry,
      event_type: "client.intercept_decision",
      status: "failed",
      message: messageFromUnknownError(error, "browser download handoff failed"),
      details: { ...commonTelemetry.details, reason: "handoff_failed" },
    });
    return false;
  }
}

function reply(sendResponse: (response?: unknown) => void, response: Promise<unknown>) {
  void response.then(sendResponse);
  return true;
}

listenForConfirmedDownloadTargets(interceptBrowserDownload, {
  prepare: async item => {
    await initialization;
    if (!interceptDownloads || !browserDownloadRequests.canReplay(item)) return false;
    if (!daemonClient.isReady()) await daemonClient.connect();
    return interceptDownloads && daemonClient.isReady() && await daemonClient.supportsBrowserSavePath();
  },
  completed: async item => {
    await initialization;
    if (!interceptDownloads) return;
    if (!daemonClient.isReady()) await daemonClient.connect();
    await daemonClient.importBrowserDownload(item);
    // The original file stays exactly where the user saved it. Erase one
    // browser record only after Rust has accepted its task metadata.
    await eraseDownloadFromHistory(item.id);
    await refreshDaemonTasks();
  },
  onError: (downloadId, error) => recordClientTelemetry({
    event_type: "client.browser_file_import", surface: "background", action: "browser_file_import",
    source: "download", status: "failed", message: messageFromUnknownError(error, "浏览器文件接入后台失败，已保留原文件和浏览器记录"),
    details: {downloadId},
  }),
});

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (!message || typeof message !== "object") {
    return;
  }

  if (message.Message === "addMedia" && typeof message.url === "string") {
    void resourceBridge.capturePageResource(sender, {
      url: message.url,
      href: message.href,
      mime: message.mime,
      ext: message.extraExt,
      requestHeaders: message.requestHeaders,
    });
    sendResponse("ok");
    return true;
  }

  if (typeof message.type !== "string") {
    return;
  }

  if (message.type === "bridge_page_command") {
    void featureBridge.onBridgeScriptCommand(message.payload, sender);
    sendResponse({ ok: true });
    return true;
  }

  if (message.type === "client_telemetry" && message.event && typeof message.event === "object") {
    recordClientTelemetry(message.event as ClientTelemetryEvent);
    sendResponse({ ok: true });
    return true;
  }

  if (message.type === "popup_get_state") {
    return reply(sendResponse, buildPopupState({
      preferredTabId: typeof message.tabId === "number" ? message.tabId : null,
      currentView: message.view as PopupView | undefined,
    }));
  }

  if (message.type === "popup_set_token") {
    return reply(sendResponse, (async () => {
      await daemonClient.setToken(String(message.token ?? "").trim());
      if (daemonClient.buildSnapshot().token) {
        await refreshDaemonTasks().catch(() => {
          notifyTaskSnapshot(taskStore.getTasks());
        });
      } else {
        taskSubscriptions.stopAll();
      }
      return buildPopupState({ currentView: message.view as PopupView | undefined });
    })());
  }

  if (message.type === "popup_set_server_url") {
    return reply(sendResponse, (async () => {
      await daemonClient.setServerUrl(String(message.serverUrl ?? ""));
      if (daemonClient.buildSnapshot().token) {
        await refreshDaemonTasks().catch(() => {
          notifyTaskSnapshot(taskStore.getTasks());
        });
      }
      return buildPopupState({ currentView: message.view as PopupView | undefined });
    })());
  }

  if (message.type === "popup_refresh_connection") {
    return reply(sendResponse, (async () => {
      await refreshDaemonTasks().catch(() => {
        notifyTaskSnapshot(taskStore.getTasks());
      });
      return buildPopupState({ currentView: message.view as PopupView | undefined });
    })());
  }

  if (message.type === "popup_request_pairing") {
    return reply(sendResponse, (async () => {
      try {
        await daemonClient.requestPairing();
        await refreshDaemonTasks().catch(() => {
          notifyTaskSnapshot(taskStore.getTasks());
        });
        return { ok: true, message: "配对成功" };
      } catch (error) {
        return { ok: false, message: messageFromUnknownError(error, "自动配对失败") };
      }
    })());
  }

  if (message.type === "popup_set_intercept_downloads") {
    return reply(sendResponse, (async () => {
      interceptDownloads = Boolean(message.enabled);
      await chrome.storage.local.set({ [INTERCEPT_DOWNLOADS_KEY]: interceptDownloads });
      return buildPopupState({ currentView: message.view as PopupView | undefined });
    })());
  }

  if (message.type === "popup_set_media_download_overlay") {
    return reply(sendResponse, (async () => {
      await updateMediaDownloadOverlay(Boolean(message.enabled));
      return buildPopupState({ currentView: message.view as PopupView | undefined });
    })());
  }

  if (message.type === "popup_task_action") {
    return reply(sendResponse, (async () => {
      return performDaemonTaskAction(
        String(message.taskId ?? ""),
        String(message.action ?? ""),
        String(message.traceId || createTraceId("popup_task_action")),
      );
    })());
  }

  if (message.type === "popup_send_resource") {
    return reply(sendResponse, resourceBridge.sendResource(
      String(message.resourceId ?? ""),
      String(message.traceId || createTraceId("popup_resource")),
    ));
  }

  if (message.type === "popup_merge_resources") {
    return reply(sendResponse, (async () => {
      const resourceIds = Array.isArray(message.resourceIds)
        ? message.resourceIds.map((value: unknown) => String(value ?? "")).filter(Boolean)
        : [];
      return resourceBridge.mergeResources(
        resourceIds,
        String(message.traceId || createTraceId("popup_merge")),
      );
    })());
  }

  if (message.type === "page_download_media") {
    return reply(sendResponse, resourceBridge.downloadPageMedia(sender, {
      selection: message.selection,
      href: String(message.href ?? ""),
      title: String(message.title ?? ""),
      traceId: String(message.traceId || createTraceId("overlay")),
    }));
  }

  if (message.type === "page_media_overlay_state") {
    sendResponse({ enabled: mediaDownloadOverlayEnabled });
    return;
  }

  if (message.type === "popup_toggle_feature") {
    return reply(sendResponse, (async () => {
      const tabId = typeof message.tabId === "number" ? message.tabId : null;
      if (tabId == null) {
        return { ok: false, message: "当前没有可操作的标签页" };
      }
      try {
        const infoMessage = await featureBridge.toggleFeature(message.feature as AdvancedFeatureKey, tabId);
        return { ok: true, message: infoMessage };
      } catch (error) {
        return {
          ok: false,
          message: error instanceof Error ? error.message : "功能切换失败",
        };
      }
    })());
  }

  if (message.type === "popup_set_media_index") {
    return reply(sendResponse, (async () => {
      await mediaBridge.setMediaIndex(Number(message.tabId ?? 0), Number(message.index ?? -1));
      return buildPopupState({
        currentView: "advanced",
      });
    })());
  }

});

const initialization = initialize();
