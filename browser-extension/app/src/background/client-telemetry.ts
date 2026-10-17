import type {ClientTelemetryEvent} from "../shared/protocol/daemon";
import type {GenericTaskSummary} from "../shared/types";

const FLUSH_DELAY_MS = 400;
const MAX_BATCH_SIZE = 50;
const MAX_QUEUE_SIZE = 500;
const MAX_STRING_LENGTH = 500;
const MAX_DETAILS_KEYS = 40;

const SENSITIVE_KEYS = new Set([
  "authorization",
  "cookie",
  "cookie_iv",
  "encrypted",
  "encrypted_cookie",
  "encrypted_cookies",
  "iv",
  "password",
  "secret",
  "token",
]);
const SENSITIVE_KEY_PATTERN = /(^|[_-])(authorization|cookie|token|secret|password|encrypted|iv)($|[_-])/i;

type EventSender = (events: ClientTelemetryEvent[]) => Promise<unknown>;

function randomId(): string {
  if (typeof crypto?.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `${Date.now().toString(36)}_${Math.random().toString(36).slice(2)}`;
}

export function createTraceId(surface = "client"): string {
  return `trace_${surface}_${randomId()}`
    .replace(/[^a-zA-Z0-9_-]/g, "_")
    .slice(0, 96);
}

export function urlHost(value?: string): string | undefined {
  if (!value) {
    return undefined;
  }
  try {
    return new URL(value).host;
  } catch {
    return "invalid";
  }
}

export function urlPathKind(value?: string): string | undefined {
  if (!value) {
    return undefined;
  }
  try {
    const path = new URL(value).pathname.toLowerCase();
    if (path.endsWith(".m3u8")) {
      return "m3u8";
    }
    if (path.endsWith(".mp4")) {
      return "mp4";
    }
    if (path.endsWith(".m4s")) {
      return "m4s";
    }
    if (path.endsWith(".mpd")) {
      return "mpd";
    }
    return "other";
  } catch {
    return "invalid";
  }
}

function truncateString(value: string): string {
  return value.length > MAX_STRING_LENGTH ? `${value.slice(0, MAX_STRING_LENGTH)}...` : value;
}

function isSensitiveKey(key: string): boolean {
  const normalized = key
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/\s+/g, "_")
    .toLowerCase();
  return SENSITIVE_KEYS.has(normalized) || SENSITIVE_KEY_PATTERN.test(normalized);
}

function sanitizeValue(value: unknown, depth = 0): unknown {
  if (value == null || typeof value === "boolean" || typeof value === "number") {
    return value;
  }
  if (typeof value === "string") {
    return truncateString(value);
  }
  if (depth >= 2) {
    return "[truncated]";
  }
  if (Array.isArray(value)) {
    return value.slice(0, 20).map((item) => sanitizeValue(item, depth + 1));
  }
  if (typeof value === "object") {
    const result: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value).slice(0, MAX_DETAILS_KEYS)) {
      result[key] = isSensitiveKey(key) ? "[redacted]" : sanitizeValue(item, depth + 1);
    }
    return result;
  }
  return String(value);
}

function sanitizeEvent(event: ClientTelemetryEvent): ClientTelemetryEvent {
  return {
    occurred_at: event.occurred_at || new Date().toISOString(),
    trace_id: event.trace_id ? truncateString(event.trace_id) : undefined,
    request_id: event.request_id ? truncateString(event.request_id) : undefined,
    task_id: event.task_id ? truncateString(event.task_id) : undefined,
    event_type: truncateString(event.event_type || "client.event"),
    surface: event.surface ? truncateString(event.surface) : undefined,
    action: event.action ? truncateString(event.action) : undefined,
    source: event.source ? truncateString(event.source) : undefined,
    status: event.status ? truncateString(event.status) : undefined,
    message: event.message ? truncateString(event.message) : undefined,
    tab_id: typeof event.tab_id === "number" && Number.isFinite(event.tab_id) ? event.tab_id : undefined,
    page_url: event.page_url,
    url_host: event.url_host ? truncateString(event.url_host) : undefined,
    url_path_kind: event.url_path_kind ? truncateString(event.url_path_kind) : undefined,
    details: event.details ? (sanitizeValue(event.details) as Record<string, unknown>) : undefined,
  };
}

export function summarizeTaskSnapshot(tasks: GenericTaskSummary[]) {
  return {
    total: tasks.length,
    active: tasks.filter((task) => !["completed", "failed", "canceled"].includes(task.status)).length,
    completed: tasks.filter((task) => task.status === "completed").length,
    failed: tasks.filter((task) => task.status === "failed").length,
    tasks: tasks.slice(0, 20).map((task) => ({
      taskId: task.taskId,
      requestId: task.requestId,
      traceId: task.traceId,
      status: task.status,
      title: truncateString(task.title || ""),
      cursor: task.eventCursor ?? 0,
      progress: Number(task.progress ?? 0),
      speed: Number(task.speed ?? 0),
    })),
  };
}

export function taskSnapshotSignature(tasks: GenericTaskSummary[]): string {
  const total = tasks.length;
  const active = tasks.filter((task) => !["completed", "failed", "canceled"].includes(task.status)).length;
  const completed = tasks.filter((task) => task.status === "completed").length;
  const failed = tasks.filter((task) => task.status === "failed").length;
  const activeTasks = tasks
    .filter((task) => !["completed", "failed", "canceled"].includes(task.status))
    .map((task) => `${task.taskId}:${task.status}:${task.eventCursor ?? 0}:${Math.round((task.progress ?? 0) * 1000)}:${Math.round(task.speed ?? 0)}`)
    .sort()
    .join("|");
  const failedTasks = tasks
    .filter((task) => task.status === "failed")
    .map((task) => `${task.taskId}:${task.eventCursor ?? 0}`)
    .sort()
    .join("|");
  return `${total}:${active}:${completed}:${failed}:${activeTasks}:${failedTasks}`;
}

export function createClientTelemetry(sendEvents: EventSender) {
  const queue: ClientTelemetryEvent[] = [];
  let flushTimer: number | null = null;
  let flushing = false;

  function scheduleFlush(): void {
    if (flushTimer !== null) {
      return;
    }
    flushTimer = self.setTimeout(() => {
      flushTimer = null;
      void flushNow();
    }, FLUSH_DELAY_MS);
  }

  async function flushNow(): Promise<void> {
    if (flushing || queue.length === 0) {
      return;
    }
    flushing = true;
    const batch = queue.splice(0, MAX_BATCH_SIZE);
    try {
      await sendEvents(batch);
    } catch {
      // Telemetry must never break the user action path.
    } finally {
      flushing = false;
      if (queue.length > 0) {
        scheduleFlush();
      }
    }
  }

  function record(event: ClientTelemetryEvent): void {
    if (!event.event_type) {
      return;
    }
    if (queue.length >= MAX_QUEUE_SIZE) {
      queue.splice(0, queue.length - MAX_QUEUE_SIZE + 1);
    }
    queue.push(sanitizeEvent(event));
    scheduleFlush();
  }

  return {
    flushNow,
    record,
  };
}
