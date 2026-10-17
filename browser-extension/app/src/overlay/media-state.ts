import type {GenericTaskSummary} from "../shared/types";

export type OverlayPhase =
  | "idle"
  | "resolving"
  | "waiting"
  | "dispatching"
  | "queued"
  | "starting"
  | "parsing"
  | "resuming"
  | "downloading"
  | "merging"
  | "completed"
  | "failed"
  | "canceled"
  | "paused";

export interface OverlayMediaState {
  mediaKey: string;
  mediaTitle: string;
  identity: string;
  traceId: string;
  taskId: string;
  requestId: string;
  phase: OverlayPhase;
  message: string;
  speed: number;
  progress: number;
  failed: boolean;
  selectionKind: string;
  lastUiStateSignature: string;
  lastDownloadUiStateSignature: string;
  terminalTimer: number;
  missingTaskTimer: number;
}

export interface TaskApplyResult {
  applied: boolean;
  terminal: boolean;
}

export function createOverlayMediaState(input: {
  mediaKey: string;
  mediaTitle: string;
  identity: string;
}): OverlayMediaState {
  return {
    mediaKey: input.mediaKey,
    mediaTitle: input.mediaTitle,
    identity: input.identity,
    traceId: "",
    taskId: "",
    requestId: "",
    phase: "idle",
    message: "",
    speed: 0,
    progress: 0,
    failed: false,
    selectionKind: "",
    lastUiStateSignature: "",
    lastDownloadUiStateSignature: "",
    terminalTimer: 0,
    missingTaskTimer: 0,
  };
}

export function isBusyOverlayState(state: Pick<OverlayMediaState, "phase"> | null | undefined): boolean {
  return Boolean(
    state
      && ["resolving", "waiting", "dispatching", "queued", "starting", "parsing", "resuming", "downloading", "merging"].includes(state.phase),
  );
}

export function bindTaskToMediaState(state: OverlayMediaState, taskId: string): void {
  state.taskId = taskId;
  state.phase = "queued";
  state.message = "";
  state.failed = false;
}

export function resetOverlayMediaState(
  state: OverlayMediaState,
  nextIdentity?: { mediaTitle: string; identity: string },
): void {
  state.traceId = "";
  state.taskId = "";
  state.requestId = "";
  state.phase = "idle";
  state.message = "";
  state.speed = 0;
  state.progress = 0;
  state.failed = false;
  state.selectionKind = "";
  state.lastUiStateSignature = "";
  state.lastDownloadUiStateSignature = "";
  if (nextIdentity) {
    state.mediaTitle = nextIdentity.mediaTitle;
    state.identity = nextIdentity.identity;
  }
}

export function taskProgressRatio(task: Pick<GenericTaskSummary, "progress"> | null | undefined): number {
  const progress = Number(task?.progress || 0);
  if (!Number.isFinite(progress)) {
    return 0;
  }
  return Math.max(0, Math.min(1, progress));
}

export function applyTaskToMediaState(
  state: OverlayMediaState | null | undefined,
  task: GenericTaskSummary | null | undefined,
): TaskApplyResult {
  if (!state || !task) {
    return { applied: false, terminal: false };
  }

  state.taskId = task.taskId || state.taskId;
  state.requestId = task.requestId || state.requestId;
  state.traceId = task.traceId || state.traceId;
  state.speed = Number(task.speed || 0);
  state.progress = taskProgressRatio(task);

  if (task.status === "completed" || task.status === "failed" || task.status === "canceled") {
    state.phase = task.status === "canceled" ? "failed" : task.status;
    state.message = task.status === "completed" ? "下载完成" : task.status === "canceled" ? "下载已取消" : "下载失败";
    state.failed = task.status !== "completed";
    return { applied: true, terminal: true };
  }

  if (["downloading", "starting", "parsing", "resuming", "merging", "queued", "paused"].includes(task.status)) {
    state.phase = task.status as OverlayPhase;
    state.message = "";
  } else {
    state.phase = "queued";
    state.message = "";
  }
  state.failed = false;
  return { applied: true, terminal: false };
}

export function applyTaskSnapshotToBindings(
  bindings: Map<string, OverlayMediaState>,
  tasks: GenericTaskSummary[] | null | undefined,
): OverlayMediaState[] {
  if (!Array.isArray(tasks) || bindings.size === 0) {
    return [];
  }
  const changed: OverlayMediaState[] = [];
  for (const [taskId, state] of [...bindings.entries()]) {
    const task = tasks.find((item) => item.taskId === taskId);
    const result = applyTaskToMediaState(state, task);
    if (result.applied) {
      changed.push(state);
    }
    if (result.terminal) {
      bindings.delete(taskId);
    }
  }
  return changed;
}

export function applyTaskUpdateToBindings(
  bindings: Map<string, OverlayMediaState>,
  task: GenericTaskSummary | null | undefined,
): OverlayMediaState | null {
  if (!task?.taskId) {
    return null;
  }
  const state = bindings.get(task.taskId);
  if (!state) {
    return null;
  }
  const result = applyTaskToMediaState(state, task);
  if (result.terminal) {
    bindings.delete(task.taskId);
  }
  return result.applied ? state : null;
}
