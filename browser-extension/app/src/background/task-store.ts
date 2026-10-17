import type {GenericTaskSummary} from "../shared/types";
import {
  daemonTaskToGenericTask,
  isTerminalTaskStatus,
  normalizeTaskProgress,
  type CreateTaskHttpReq,
  type CreateTaskHttpResp,
  type TaskEvent,
  type TaskSummary,
} from "../shared/protocol/daemon";
import {localStorageSet, loadFromLocalStorage} from "./chrome-helpers";

const DAEMON_TASK_STATE_KEY = "daemonTaskState";

type PersistedTaskState = {
  tasks: GenericTaskSummary[];
  cursors: Record<string, number>;
  createdAt: Record<string, number>;
};

function now(): number {
  return Date.now();
}

function eventCursorOf(task: GenericTaskSummary): number {
  return Number(task.eventCursor ?? 0);
}

function clampCursor(value: unknown): number {
  const numeric = Number(value ?? 0);
  return Number.isFinite(numeric) && numeric > 0 ? Math.floor(numeric) : 0;
}

function patchTaskFromEvent(task: GenericTaskSummary, event: TaskEvent): GenericTaskSummary {
  const next = { ...task, eventCursor: Math.max(eventCursorOf(task), event.event_id) };

  switch (event.type) {
    case "task_accepted":
    case "task_queued":
      next.status = "queued";
      break;
    case "task_parsing_started":
      next.status = "parsing";
      break;
    case "task_resuming_started":
      next.status = "resuming";
      next.speed = 0;
      break;
    case "task_stage_changed":
      if (event.stage?.toLowerCase().includes("merge")) {
        next.status = "merging";
      }
      break;
    case "task_download_progress_updated":
      next.status = "downloading";
      next.progress = normalizeTaskProgress(Number(event.progress ?? next.progress));
      next.receivedBytes = Number(event.received_bytes ?? next.receivedBytes);
      {
        const eventTotal = Number(event.total_bytes);
        next.fileSize = Number.isFinite(eventTotal) && eventTotal > 0 ? eventTotal : next.fileSize;
      }
      next.speed = Number(event.speed_bytes_per_sec ?? next.speed);
      break;
    case "task_merge_started":
      next.status = "merging";
      break;
    case "task_completed":
      next.status = "completed";
      next.progress = 1;
      next.speed = 0;
      next.canPause = false;
      next.canOpenFile = false;
      next.canOpenFolder = false;
      next.outputPath = event.output_path || next.outputPath;
      next.artifactState = "available";
      break;
    case "task_artifact_available":
      next.outputPath = event.output_path || next.outputPath;
      next.artifactState = "available";
      break;
    case "task_artifact_missing":
      next.artifactState = "missing";
      next.canOpenFile = false;
      next.canOpenFolder = false;
      break;
    case "task_artifact_moved":
      next.outputPath = event.new_path || next.outputPath;
      next.artifactState = "moved";
      break;
    case "task_failed":
      next.status = "failed";
      next.speed = 0;
      next.canPause = false;
      break;
    case "task_canceled":
      next.status = "canceled";
      next.speed = 0;
      next.canPause = false;
      break;
    case "task_paused":
      next.status = "paused";
      next.speed = 0;
      next.canPause = false;
      break;
    case "task_resumed":
      next.status = "queued";
      next.canPause = true;
      break;
    default:
      break;
  }

  next.canPause =
    next.status === "queued"
    || next.status === "parsing"
    || next.status === "resuming"
    || next.status === "downloading"
    || next.status === "merging";
  next.canOpenFile = false;
  next.canOpenFolder = false;
  return next;
}

export function createTaskStore() {
  const tasksById = new Map<string, GenericTaskSummary>();
  const cursorByTaskId = new Map<string, number>();
  const createdAtByTaskId = new Map<string, number>();
  let persistTimer: number | null = null;

  function schedulePersist() {
    if (persistTimer !== null) {
      self.clearTimeout(persistTimer);
    }
    persistTimer = self.setTimeout(() => {
      persistTimer = null;
      void persistNow();
    }, 250);
  }

  async function persistNow() {
    const state: PersistedTaskState = {
      tasks: getTasks(),
      cursors: Object.fromEntries(cursorByTaskId.entries()),
      createdAt: Object.fromEntries(createdAtByTaskId.entries()),
    };
    await localStorageSet({ [DAEMON_TASK_STATE_KEY]: state });
  }

  async function loadPersistentState() {
    const loaded = await loadFromLocalStorage<{ [DAEMON_TASK_STATE_KEY]: PersistedTaskState }>({
      [DAEMON_TASK_STATE_KEY]: {
        tasks: [],
        cursors: {},
        createdAt: {},
      },
    });

    const state = loaded[DAEMON_TASK_STATE_KEY];
    for (const task of state.tasks ?? []) {
      tasksById.set(task.taskId, task);
      cursorByTaskId.set(task.taskId, clampCursor(task.eventCursor ?? state.cursors?.[task.taskId]));
      createdAtByTaskId.set(task.taskId, Number(state.createdAt?.[task.taskId] ?? task.createdAt ?? now()));
    }
  }

  function replaceSnapshot(tasks: TaskSummary[]) {
    const seen = new Set<string>();
    for (const task of tasks) {
      const createdAt = createdAtByTaskId.get(task.task_id) ?? now();
      createdAtByTaskId.set(task.task_id, createdAt);
      const generic = daemonTaskToGenericTask(task, createdAt);
      const previousCursor = cursorByTaskId.get(task.task_id) ?? 0;
      const cursor = Math.max(previousCursor, Number(task.event_cursor ?? 0));
      generic.eventCursor = cursor;
      cursorByTaskId.set(task.task_id, cursor);
      tasksById.set(task.task_id, generic);
      seen.add(task.task_id);
    }

    for (const taskId of tasksById.keys()) {
      if (!seen.has(taskId)) {
        tasksById.delete(taskId);
        cursorByTaskId.delete(taskId);
        createdAtByTaskId.delete(taskId);
      }
    }
    schedulePersist();
  }

  function upsertAccepted(response: CreateTaskHttpResp, request: CreateTaskHttpReq): GenericTaskSummary | null {
    if (!response.accepted || !response.task_id) {
      return null;
    }
    const taskId = response.task_id;
    const createdAt = createdAtByTaskId.get(taskId) ?? now();
    createdAtByTaskId.set(taskId, createdAt);
    const cursor = Math.max(cursorByTaskId.get(taskId) ?? 0, Number(response.event_cursor ?? 0));
    cursorByTaskId.set(taskId, cursor);
    const task: GenericTaskSummary = {
      taskId,
      title: request.title || request.payload.filename || taskId,
      status: String(response.initial_status ?? "queued"),
      progress: 0,
      receivedBytes: 0,
      fileSize: Number(request.payload.size ?? 0),
      speed: 0,
      createdAt,
      canPause: true,
      canOpenFile: false,
      canOpenFolder: false,
      fileExt: "",
      packName: request.source || "",
      eventCursor: cursor,
      requestId: response.request_id || request.request_id,
      traceId: response.trace_id || request.client_context?.trace_id,
      artifactState: "unknown",
    };
    tasksById.set(taskId, { ...tasksById.get(taskId), ...task });
    schedulePersist();
    return task;
  }

  function applyEvent(event: TaskEvent): GenericTaskSummary | null {
    if (event.type === "task_deleted") {
      removeTask(event.task_id);
      return null;
    }

    const existing = tasksById.get(event.task_id) ?? {
      taskId: event.task_id,
      title: event.task_id,
      status: "queued",
      progress: 0,
      receivedBytes: 0,
      fileSize: 0,
      speed: 0,
      createdAt: now(),
      canPause: true,
      canOpenFile: false,
      canOpenFolder: false,
      fileExt: "",
      packName: "",
      eventCursor: 0,
      requestId: event.request_id,
      traceId: event.trace_id,
      artifactState: "unknown",
    };

    createdAtByTaskId.set(event.task_id, existing.createdAt);
    const patched = {
      ...patchTaskFromEvent(existing, event),
      requestId: event.request_id || existing.requestId,
      traceId: event.trace_id || existing.traceId,
    };
    tasksById.set(event.task_id, patched);
    cursorByTaskId.set(event.task_id, Math.max(cursorByTaskId.get(event.task_id) ?? 0, event.event_id));
    schedulePersist();
    return patched;
  }

  function removeTask(taskId: string): void {
    tasksById.delete(taskId);
    cursorByTaskId.delete(taskId);
    createdAtByTaskId.delete(taskId);
    schedulePersist();
  }

  function getTasks(): GenericTaskSummary[] {
    return [...tasksById.values()];
  }

  function getTask(taskId: string): GenericTaskSummary | null {
    return tasksById.get(taskId) ?? null;
  }

  function getCursor(taskId: string): number {
    return cursorByTaskId.get(taskId) ?? 0;
  }

  function getActiveTaskIds(): string[] {
    return [...tasksById.values()]
      .filter((task) => !isTerminalTaskStatus(task.status))
      .map((task) => task.taskId);
  }

  return {
    applyEvent,
    getActiveTaskIds,
    getCursor,
    getTask,
    getTasks,
    loadPersistentState,
    removeTask,
    replaceSnapshot,
    upsertAccepted,
  };
}
