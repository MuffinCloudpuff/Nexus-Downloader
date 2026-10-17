import assert from "node:assert/strict";
import {
  applyTaskSnapshotToBindings,
  applyTaskToMediaState,
  applyTaskUpdateToBindings,
  bindTaskToMediaState,
  createOverlayMediaState,
  isBusyOverlayState,
  resetOverlayMediaState,
} from "./media-state";
import type {GenericTaskSummary} from "../shared/types";

function task(input: Partial<GenericTaskSummary> & Pick<GenericTaskSummary, "taskId" | "status">): GenericTaskSummary {
  return {
    taskId: input.taskId,
    title: input.title ?? input.taskId,
    status: input.status,
    progress: input.progress ?? 0,
    receivedBytes: input.receivedBytes ?? 0,
    fileSize: input.fileSize ?? 0,
    speed: input.speed ?? 0,
    createdAt: input.createdAt ?? 0,
    canPause: input.canPause ?? false,
    canOpenFile: input.canOpenFile ?? false,
    canOpenFolder: input.canOpenFolder ?? false,
    fileExt: input.fileExt ?? "",
    packName: input.packName ?? "",
    requestId: input.requestId,
    traceId: input.traceId,
    eventCursor: input.eventCursor,
  };
}

{
  const first = createOverlayMediaState({ mediaKey: "m1", mediaTitle: "first", identity: "v1" });
  const second = createOverlayMediaState({ mediaKey: "m2", mediaTitle: "second", identity: "v2" });
  const bindings = new Map<string, typeof first>();
  bindTaskToMediaState(first, "task-1");
  bindTaskToMediaState(second, "task-2");
  bindings.set("task-1", first);
  bindings.set("task-2", second);

  const changed = applyTaskSnapshotToBindings(bindings, [
    task({ taskId: "task-1", status: "downloading", progress: 0.42, speed: 1024 }),
    task({ taskId: "task-2", status: "queued", progress: 0 }),
  ]);

  assert.equal(changed.length, 2);
  assert.equal(first.phase, "downloading");
  assert.equal(first.progress, 0.42);
  assert.equal(first.speed, 1024);
  assert.equal(second.phase, "queued");
}

{
  const first = createOverlayMediaState({ mediaKey: "m1", mediaTitle: "first", identity: "v1" });
  const second = createOverlayMediaState({ mediaKey: "m2", mediaTitle: "second", identity: "v2" });
  const bindings = new Map<string, typeof first>([
    ["task-1", first],
    ["task-2", second],
  ]);
  bindTaskToMediaState(first, "task-1");
  bindTaskToMediaState(second, "task-2");

  const changed = applyTaskUpdateToBindings(bindings, task({ taskId: "task-2", status: "downloading", progress: 0.8 }));

  assert.equal(changed, second);
  assert.equal(first.phase, "queued");
  assert.equal(second.phase, "downloading");
  assert.equal(second.progress, 0.8);
}

{
  const state = createOverlayMediaState({ mediaKey: "m1", mediaTitle: "first", identity: "v1" });
  const bindings = new Map<string, typeof state>([["task-1", state]]);
  bindTaskToMediaState(state, "task-1");

  const changed = applyTaskUpdateToBindings(bindings, task({ taskId: "task-1", status: "completed", progress: 1 }));

  assert.equal(changed, state);
  assert.equal(state.phase, "completed");
  assert.equal(state.message, "下载完成");
  assert.equal(state.failed, false);
  assert.equal(bindings.has("task-1"), false);
}

{
  const state = createOverlayMediaState({ mediaKey: "m1", mediaTitle: "first", identity: "v1" });
  bindTaskToMediaState(state, "task-1");
  assert.equal(isBusyOverlayState(state), true);

  applyTaskToMediaState(state, task({ taskId: "task-1", status: "failed" }));
  assert.equal(isBusyOverlayState(state), false);
  assert.equal(state.failed, true);

  resetOverlayMediaState(state, { mediaTitle: "next", identity: "v2" });
  assert.equal(state.phase, "idle");
  assert.equal(state.taskId, "");
  assert.equal(state.mediaTitle, "next");
  assert.equal(state.identity, "v2");
}
