import type {
  AppSettingsDto,
  BilibiliQrGenerateResp,
  BilibiliQrStatusResp,
  CreateTaskHttpReq,
  CreateTaskHttpResp,
  GetTaskEventsResp,
  GetTasksResp,
  PatchAppSettingsDto,
  SettingsUpdateResp,
  RuntimeMetrics,
  AppVolumeStats,
  StoragePathStats,
  TaskCommandResp,
  TaskEvent,
  TaskSummary,
} from "../protocol/daemon";

export const DEFAULT_DAEMON_URL = "http://127.0.0.1:14371";

type DaemonRequestInit = RequestInit & {
  auth?: boolean;
  retryAuth?: boolean;
};

function authHeaders(auth = true): HeadersInit {
  if (!auth) {
    return {};
  }
  const token = localStorage.getItem("gd3.daemonToken");
  return token ? { Authorization: `Bearer ${token}` } : {};
}

function isUnauthorizedStatus(status: number): boolean {
  return status === 401 || status === 403;
}

async function requestJson<T>(path: string, init: DaemonRequestInit = {}): Promise<T> {
  const { auth = true, retryAuth = true, ...requestInit } = init;
  const response = await fetch(`${DEFAULT_DAEMON_URL}${path}`, {
    ...requestInit,
    cache: "no-store",
    headers: {
      ...authHeaders(auth),
      ...(requestInit.body ? { "content-type": "application/json" } : {}),
      ...(requestInit.headers ?? {}),
    },
  });

  if (isUnauthorizedStatus(response.status) && auth && retryAuth) {
    await requestPairing();
    return requestJson<T>(path, { ...requestInit, auth, retryAuth: false });
  }

  if (!response.ok) {
    let message = `daemon request failed: ${response.status}`;
    try {
      const body = await response.clone().json() as { error?: { message?: string } };
      if (body.error?.message) {
        message = body.error.message;
      }
    } catch {
      // Keep the status-based fallback when the daemon returns a non-JSON error.
    }
    throw new Error(message);
  }
  return response.json() as Promise<T>;
}

export function getSettings(): Promise<AppSettingsDto> {
  return requestJson<AppSettingsDto>("/api/settings");
}

export async function patchSettings(patch: PatchAppSettingsDto): Promise<AppSettingsDto> {
  const response = await requestJson<SettingsUpdateResp>("/api/settings", {
    method: "PATCH",
    body: JSON.stringify(patch),
  });
  return response.settings;
}

export async function requestPairing(): Promise<string> {
  const response = await requestJson<{ pairing_issued: boolean; token?: string; error?: { message: string } }>(
    "/api/pairing/request",
    {
      method: "POST",
      auth: false,
      retryAuth: false,
      body: JSON.stringify({
        request_id: crypto.randomUUID(),
        client_context: {
          protocol_version: 1,
          trace_id: `trace_desktop_${crypto.randomUUID()}`,
        },
      }),
    },
  );
  if (!response.pairing_issued || !response.token) {
    throw new Error(response.error?.message || "pairing failed");
  }
  localStorage.setItem("gd3.daemonToken", response.token);
  return response.token;
}

export async function getTasks(): Promise<TaskSummary[]> {
  const response = await requestJson<GetTasksResp>("/api/tasks");
  return response.tasks;
}

export function getTaskEvents(taskId: string, fromEventId: number): Promise<GetTaskEventsResp> {
  return requestJson<GetTaskEventsResp>(
    `/api/tasks/${encodeURIComponent(taskId)}/events?from_event_id=${encodeURIComponent(String(fromEventId))}`,
  );
}

export interface TaskEventStreamOptions {
  taskId: string;
  fromEventId: number;
  signal: AbortSignal;
  onEvent: (event: TaskEvent) => void;
}

export async function streamTaskEvents({
  taskId,
  fromEventId,
  signal,
  onEvent,
}: TaskEventStreamOptions): Promise<void> {
  const response = await fetch(
    `${DEFAULT_DAEMON_URL}/api/tasks/${encodeURIComponent(taskId)}/stream?from_event_id=${encodeURIComponent(String(fromEventId))}`,
    {
      headers: authHeaders(),
      signal,
    },
  );

  if (!response.ok) {
    throw new Error(`daemon task stream failed: ${response.status}`);
  }
  if (!response.body) {
    throw new Error("daemon task stream has no response body");
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  try {
    while (!signal.aborted) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      buffer += decoder.decode(value, { stream: true });
      buffer = drainSseBuffer(buffer, onEvent);
    }
    buffer += decoder.decode();
    drainSseBuffer(buffer, onEvent);
  } finally {
    reader.releaseLock();
  }
}

export async function streamAllTaskEvents({
  signal,
  onEvent,
}: Omit<TaskEventStreamOptions, "taskId" | "fromEventId">): Promise<void> {
  const response = await fetch(`${DEFAULT_DAEMON_URL}/api/tasks/stream`, {
    headers: authHeaders(),
    signal,
  });

  if (!response.ok) {
    throw new Error(`daemon task stream failed: ${response.status}`);
  }
  if (!response.body) {
    throw new Error("daemon task stream has no response body");
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  try {
    while (!signal.aborted) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      buffer += decoder.decode(value, { stream: true });
      buffer = drainSseBuffer(buffer, onEvent);
    }
    buffer += decoder.decode();
    drainSseBuffer(buffer, onEvent);
  } finally {
    reader.releaseLock();
  }
}

function drainSseBuffer(buffer: string, onEvent: (event: TaskEvent) => void): string {
  let nextBuffer = buffer;
  while (true) {
    const separatorMatch = /\r?\n\r?\n/.exec(nextBuffer);
    if (!separatorMatch) {
      return nextBuffer;
    }
    const separatorIndex = separatorMatch.index;
    const rawEvent = nextBuffer.slice(0, separatorIndex);
    nextBuffer = nextBuffer.slice(separatorIndex + separatorMatch[0].length);
    const event = parseTaskEventSse(rawEvent);
    if (event) {
      onEvent(event);
    }
  }
}

function parseTaskEventSse(rawEvent: string): TaskEvent | null {
  let eventName = "message";
  const dataLines: string[] = [];
  for (const line of rawEvent.split(/\r?\n/)) {
    if (line.startsWith("event:")) {
      eventName = line.slice("event:".length).trim();
    } else if (line.startsWith("data:")) {
      dataLines.push(line.slice("data:".length).trimStart());
    }
  }
  if (eventName !== "task_event" || dataLines.length === 0) {
    return null;
  }
  return JSON.parse(dataLines.join("\n")) as TaskEvent;
}

export function createTask(request: CreateTaskHttpReq): Promise<CreateTaskHttpResp> {
  return requestJson<CreateTaskHttpResp>("/api/tasks", {
    method: "POST",
    body: JSON.stringify(request),
  });
}

export function pauseTask(taskId: string): Promise<TaskCommandResp> {
  return taskCommand(taskId, "pause");
}

export function resumeTask(taskId: string): Promise<TaskCommandResp> {
  return taskCommand(taskId, "resume");
}

export function cancelTask(taskId: string): Promise<TaskCommandResp> {
  return taskCommand(taskId, "cancel");
}

export function deleteTask(taskId: string): Promise<TaskCommandResp> {
  return taskCommand(taskId, "delete");
}

export function retryTask(taskId: string): Promise<TaskCommandResp> {
  return taskCommand(taskId, "retry");
}

function taskCommand(
  taskId: string,
  command: "pause" | "resume" | "cancel" | "delete" | "retry",
): Promise<TaskCommandResp> {
  return requestJson<TaskCommandResp>(`/api/tasks/${encodeURIComponent(taskId)}/${command}`, {
    method: "POST",
  });
}

export function getRuntimeMetrics(): Promise<RuntimeMetrics> {
  return requestJson<RuntimeMetrics>("/api/runtime/metrics");
}

export function getStoragePathStats(path: string): Promise<StoragePathStats> {
  return requestJson<StoragePathStats>(`/api/storage/path_stats?path=${encodeURIComponent(path)}`);
}

export function getStorageAppVolumeStats(): Promise<AppVolumeStats> {
  return requestJson<AppVolumeStats>("/api/storage/app_volume_stats");
}

export function generateBilibiliQr(): Promise<BilibiliQrGenerateResp> {
  return requestJson<BilibiliQrGenerateResp>("/api/auth/bilibili/qr_generate", {
    method: "POST",
  });
}

export function getBilibiliQrStatus(sessionKey: string): Promise<BilibiliQrStatusResp> {
  return requestJson<BilibiliQrStatusResp>(
    `/api/auth/bilibili/qr_status?session_key=${encodeURIComponent(sessionKey)}`,
  );
}
