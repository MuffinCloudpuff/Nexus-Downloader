import {DEFAULT_SERVER_URL} from "../shared/constants";
import type {DesktopConnectionState} from "../shared/types";
import type {
  ClientTelemetryEvent,
  ClientTelemetryResp,
  CreateTaskHttpReq,
  CreateTaskHttpResp,
  GetTaskEventsResp,
  GetTasksResp,
  PairingResponse,
  TaskCommandResp,
  TaskEvent,
  TaskSummary,
} from "../shared/protocol/daemon";
import {messageForDaemonError} from "../shared/protocol/error-messages";
import {PAIR_TOKEN_KEY, PROTOCOL_VERSION, RECONNECT_ALARM, SERVER_URL_KEY} from "./constants";
import {loadFromLocalStorage, localStorageSet} from "./chrome-helpers";

const HTTP_TIMEOUT_MS = 15000;
const STREAM_READER_ENCODING = "utf-8";

export type DaemonClientSnapshot = {
  connectionState: DesktopConnectionState;
  connectionMessage: string;
  desktopVersion: string;
  token: string;
  serverUrl: string;
};

function nextRequestId(): string {
  return crypto.randomUUID();
}

function normalizeHttpBaseUrl(value: string): string {
  const trimmed = String(value || "").trim();
  if (!trimmed) {
    return DEFAULT_SERVER_URL;
  }

  try {
    const url = new URL(trimmed);
    if (url.protocol === "ws:" || url.protocol === "wss:") {
      url.protocol = url.protocol === "wss:" ? "https:" : "http:";
      if (url.port === "14370") {
        url.port = "14371";
      }
    }
    return url.toString().replace(/\/$/, "");
  } catch {
    return DEFAULT_SERVER_URL;
  }
}

function isUnauthorizedStatus(status: number): boolean {
  return status === 401 || status === 403;
}

function createTimeoutSignal(timeoutMs: number): { signal: AbortSignal; cancel: () => void } {
  const controller = new AbortController();
  const timeoutId = self.setTimeout(() => controller.abort(), timeoutMs);
  return {
    signal: controller.signal,
    cancel: () => self.clearTimeout(timeoutId),
  };
}

function parseSseEvents(buffer: string): { events: string[]; rest: string } {
  const normalized = buffer.replace(/\r\n/g, "\n");
  const parts = normalized.split("\n\n");
  const rest = parts.pop() ?? "";
  const events = parts
    .map((chunk) =>
      chunk
        .split("\n")
        .filter((line) => line.startsWith("data:"))
        .map((line) => line.slice(5).trimStart())
        .join("\n")
        .trim(),
    )
    .filter(Boolean);
  return { events, rest };
}

export function createDaemonClient() {
  let connectionState: DesktopConnectionState = "missing_token";
  let connectionMessage = "待配对";
  let desktopVersion = "";
  let pairToken = "";
  let pairingPromise: Promise<string> | null = null;
  let serverUrl = DEFAULT_SERVER_URL;

  function setConnectionState(state: DesktopConnectionState, message: string) {
    connectionState = state;
    connectionMessage = message;
  }

  function apiUrl(path: string): string {
    return `${serverUrl.replace(/\/$/, "")}${path}`;
  }

  async function parseResponseBody<T>(response: Response): Promise<{ json?: T; text: string }> {
    const text = await response.text();
    if (!text) {
      return { text: "" };
    }
    try {
      return {
        json: JSON.parse(text) as T,
        text,
      };
    } catch {
      return { text };
    }
  }

  async function requestJson<T>(
    path: string,
    options: RequestInit & { auth?: boolean; timeoutMs?: number } = {},
  ): Promise<T> {
    const timeout = createTimeoutSignal(options.timeoutMs ?? HTTP_TIMEOUT_MS);
    const headers = new Headers(options.headers);
    if (!headers.has("Content-Type") && options.body) {
      headers.set("Content-Type", "application/json");
    }
    if (options.auth !== false) {
      headers.set("Authorization", `Bearer ${pairToken}`);
    }

    try {
      const response = await fetch(apiUrl(path), {
        ...options,
        headers,
        signal: timeout.signal,
      });

      const body = await parseResponseBody<T & { error?: unknown }>(response);
      if (!response.ok) {
        if (isUnauthorizedStatus(response.status)) {
          setConnectionState("unauthorized", "配对令牌无效，请重新配对");
          // Auto-repair on next connection attempt by clearing the token
          void setToken("");
        }
        const maybeError = body.json && typeof body.json === "object" ? (body.json as { error?: any }).error : undefined;
        const fallback = isUnauthorizedStatus(response.status)
          ? "配对令牌无效，请重新配对"
          : body.text || response.statusText || "daemon 请求失败";
        throw new Error(messageForDaemonError(maybeError, fallback));
      }
      if (typeof body.json === "undefined") {
        return undefined as T;
      }
      return body.json as T;
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        throw new Error("daemon 请求超时");
      }
      throw error;
    } finally {
      timeout.cancel();
    }
  }

  async function getTasks(): Promise<TaskSummary[]> {
    const response = await requestJson<GetTasksResp>("/api/tasks", { method: "GET" });
    return Array.isArray(response.tasks) ? response.tasks : [];
  }

  async function getTask(taskId: string): Promise<TaskSummary | null> {
    return requestJson<TaskSummary | null>(`/api/tasks/${encodeURIComponent(taskId)}`, { method: "GET" });
  }

  async function getTaskEvents(taskId: string, fromEventId: number): Promise<TaskEvent[]> {
    const query = new URLSearchParams({ from_event_id: String(Math.max(0, fromEventId)) });
    const response = await requestJson<GetTaskEventsResp | null>(
      `/api/tasks/${encodeURIComponent(taskId)}/events?${query}`,
      { method: "GET" },
    );
    return response?.events ?? [];
  }

  async function createTask(request: CreateTaskHttpReq): Promise<CreateTaskHttpResp> {
    return requestJson<CreateTaskHttpResp>("/api/tasks", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async function sendClientEvents(events: ClientTelemetryEvent[]): Promise<ClientTelemetryResp> {
    return requestJson<ClientTelemetryResp>("/api/client-events", {
      method: "POST",
      body: JSON.stringify({ events }),
    });
  }

  async function supportsBrowserSavePath(): Promise<boolean> {
    try {
      const capabilities = await requestJson<{browser_download_save_path?: boolean; browser_download_deferred_start?: boolean}>(
        "/api/capabilities", {method: "GET"},
      );
      return capabilities?.browser_download_save_path === true && capabilities.browser_download_deferred_start === true;
    } catch {
      return false;
    }
  }

  async function importBrowserDownload(item: chrome.downloads.DownloadItem): Promise<void> {
    const capabilities = await requestJson<{browser_download_import?: boolean}>("/api/capabilities");
    if (capabilities.browser_download_import !== true) {
      throw new Error("请更新托盘后台，以便统一管理网页本地文件");
    }
    // startTime protects against browser download-ID reuse after profile resets.
    const result = await requestJson<{accepted: boolean}>("/api/browser-downloads/import", {
      method: "POST",
      body: JSON.stringify({
        request_id: `browser-file:${item.id}:${item.startTime}`,
        save_path: item.filename,
        source_url: /^data:/i.test(item.finalUrl || item.url) ? "data:" : (item.finalUrl || item.url),
        mime: item.mime || undefined,
        size: item.fileSize >= 0 ? item.fileSize : item.totalBytes >= 0 ? item.totalBytes : undefined,
      }),
    });
    if (!result.accepted) throw new Error("后台未接受浏览器已保存文件");
  }

  async function sendTaskCommand(
    taskId: string,
    command: "pause" | "resume" | "cancel" | "delete" | "start",
  ): Promise<TaskCommandResp> {
    return requestJson<TaskCommandResp>(`/api/tasks/${encodeURIComponent(taskId)}/${command}`, {
      method: "POST",
      body: JSON.stringify({}),
    });
  }

  function requestPairing(): Promise<string> {
    if (pairingPromise) {
      return pairingPromise;
    }
    clearConnection();
    setConnectionState("connecting", "配对中 (Native Messaging)");

    pairingPromise = new Promise<string>((resolve, reject) => {
      let settled = false;
      const timeoutId = self.setTimeout(() => {
        fail("自动配对超时，请确认托盘服务正在运行");
      }, HTTP_TIMEOUT_MS);
      function fail(message: string) {
        if (settled) return;
        settled = true;
        self.clearTimeout(timeoutId);
        setConnectionState("disconnected", message);
        reject(new Error(message));
      }
      try {
        chrome.runtime.sendNativeMessage(
          "com.gd3.nexusdownloader",
          { action: "get_token" },
          async (response) => {
            const nativeError = chrome.runtime.lastError;
            if (settled) return;
            if (nativeError) {
              fail("Native Messaging 配对失败: " + (nativeError.message || "未知错误"));
              return;
            }
            try {
              if (typeof response?.token !== "string" || !response.token.trim()) {
                fail("自动配对失败: 未收到有效令牌");
                return;
              }
              await setToken(response.token);
              if (settled) return;
              settled = true;
              self.clearTimeout(timeoutId);
              resolve(response.token);
            } catch (error) {
              fail(error instanceof Error ? error.message : "配对令牌保存失败");
            }
          }
        );
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        fail("Native Messaging 调用失败: " + msg);
      }
    }).finally(() => {
      pairingPromise = null;
    });
    return pairingPromise;
  }

  async function revokePairing(): Promise<void> {
    await requestJson<{ pairing_revoked: boolean }>("/api/pairing/revoke", {
      method: "POST",
      body: JSON.stringify({}),
    });
    await setToken("");
  }

  async function wakeUp(): Promise<void> {
    return new Promise((resolve) => {
      try {
        chrome.runtime.sendNativeMessage(
          "com.gd3.nexusdownloader",
          { action: "ping" },
          () => {
            const _err = chrome.runtime.lastError; // Ignore errors, just want to trigger host
            resolve();
          }
        );
      } catch {
        resolve();
      }
    });
  }

  async function connect(): Promise<TaskSummary[]> {
    if (!pairToken) {
      await requestPairing();
    }

    if (!pairToken) {
      clearConnection();
      setConnectionState("missing_token", "待配对");
      return [];
    }

    setConnectionState("connecting", "连接中");
    try {
      const tasks = await getTasks();
      desktopVersion = "rust-daemon";
      setConnectionState("connected", "已连接");
      return tasks;
    } catch (error) {
      if (connectionState !== "unauthorized") {
        desktopVersion = "";
        setConnectionState("disconnected", error instanceof Error ? error.message : "连接失败");
      }
      throw error;
    }
  }

  function clearConnection() {
    desktopVersion = "";
  }

  async function loadPersistentState() {
    const localState = await loadFromLocalStorage<{
      [PAIR_TOKEN_KEY]: string;
      [SERVER_URL_KEY]: string;
    }>({
      [PAIR_TOKEN_KEY]: "",
      [SERVER_URL_KEY]: DEFAULT_SERVER_URL,
    });

    pairToken = String(localState[PAIR_TOKEN_KEY] ?? "").trim();
    serverUrl = normalizeHttpBaseUrl(String(localState[SERVER_URL_KEY] ?? DEFAULT_SERVER_URL));
    if (serverUrl !== localState[SERVER_URL_KEY]) {
      await localStorageSet({ [SERVER_URL_KEY]: serverUrl });
    }
  }

  async function setToken(token: string) {
    pairToken = String(token ?? "").trim();
    await localStorageSet({ [PAIR_TOKEN_KEY]: pairToken });
    if (!pairToken) {
      clearConnection();
      setConnectionState("missing_token", "待配对");
    }
  }

  async function setServerUrl(nextServerUrl: string) {
    serverUrl = normalizeHttpBaseUrl(nextServerUrl);
    await localStorageSet({ [SERVER_URL_KEY]: serverUrl });
  }

  function onLocalStorageChanged(changes: { [key: string]: chrome.storage.StorageChange }) {
    if (changes[PAIR_TOKEN_KEY]) {
      pairToken = String(changes[PAIR_TOKEN_KEY].newValue ?? "").trim();
    }
    if (changes[SERVER_URL_KEY]) {
      serverUrl = normalizeHttpBaseUrl(String(changes[SERVER_URL_KEY].newValue ?? DEFAULT_SERVER_URL));
    }
  }

  function buildSnapshot(): DaemonClientSnapshot {
    return {
      connectionState,
      connectionMessage,
      desktopVersion,
      token: pairToken,
      serverUrl,
    };
  }

  function isReady(): boolean {
    return connectionState === "connected" && Boolean(pairToken);
  }

  function hasToken(): boolean {
    return Boolean(pairToken);
  }

  function ensureReconnectAlarm() {
    chrome.alarms.create(RECONNECT_ALARM, { periodInMinutes: 1 });
  }

  function onReconnectAlarm(alarm: chrome.alarms.Alarm, reconnect: () => void) {
    if (alarm.name !== RECONNECT_ALARM || connectionState === "connected") {
      return;
    }
    reconnect();
  }

  async function streamTaskEvents(
    taskId: string,
    fromEventId: number,
    signal: AbortSignal,
    onEvent: (event: TaskEvent) => void,
  ): Promise<void> {
    const query = new URLSearchParams({ from_event_id: String(Math.max(0, fromEventId)) });
    const response = await fetch(apiUrl(`/api/tasks/${encodeURIComponent(taskId)}/stream?${query}`), {
      method: "GET",
      headers: {
        Authorization: `Bearer ${pairToken}`,
      },
      signal,
    });

    if (!response.ok || !response.body) {
      if (isUnauthorizedStatus(response.status)) {
        setConnectionState("unauthorized", "配对令牌无效，请重新配对");
      }
      throw new Error(response.statusText || "任务事件订阅失败");
    }

    setConnectionState("connected", "已连接");
    const reader = response.body.getReader();
    const decoder = new TextDecoder(STREAM_READER_ENCODING);
    let buffer = "";

    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      buffer += decoder.decode(value, { stream: true });
      const parsed = parseSseEvents(buffer);
      buffer = parsed.rest;
      for (const data of parsed.events) {
        onEvent(JSON.parse(data) as TaskEvent);
      }
    }
  }

  async function streamAllTaskEvents(
    signal: AbortSignal,
    onEvent: (event: TaskEvent) => void,
  ): Promise<void> {
    const response = await fetch(apiUrl("/api/tasks/stream"), {
      method: "GET",
      headers: {
        Authorization: `Bearer ${pairToken}`,
      },
      signal,
    });

    if (!response.ok || !response.body) {
      if (isUnauthorizedStatus(response.status)) {
        setConnectionState("unauthorized", "配对令牌无效，请重新配对");
      }
      throw new Error(response.statusText || "任务事件订阅失败");
    }

    setConnectionState("connected", "已连接");
    const reader = response.body.getReader();
    const decoder = new TextDecoder(STREAM_READER_ENCODING);
    let buffer = "";

    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      buffer += decoder.decode(value, { stream: true });
      const parsed = parseSseEvents(buffer);
      buffer = parsed.rest;
      for (const data of parsed.events) {
        onEvent(JSON.parse(data) as TaskEvent);
      }
    }
  }

  return {
    buildSnapshot,
    connect,
    createTask,
    supportsBrowserSavePath,
    importBrowserDownload,
    ensureReconnectAlarm,
    getTask,
    getTaskEvents,
    getTasks,
    hasToken,
    isReady,
    loadPersistentState,
    onLocalStorageChanged,
    onReconnectAlarm,
    requestPairing,
    revokePairing,
    sendClientEvents,
    sendTaskCommand,
    setServerUrl,
    setToken,
    streamAllTaskEvents,
    streamTaskEvents,
    wakeUp,
  };
}
