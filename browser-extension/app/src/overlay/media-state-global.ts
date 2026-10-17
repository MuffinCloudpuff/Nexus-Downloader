import {
  applyTaskSnapshotToBindings,
  applyTaskToMediaState,
  applyTaskUpdateToBindings,
  bindTaskToMediaState,
  createOverlayMediaState,
  isBusyOverlayState,
  resetOverlayMediaState,
  taskProgressRatio,
} from "./media-state";

Object.assign(globalThis, {
  GD3OverlayMediaState: {
    applyTaskSnapshotToBindings,
    applyTaskToMediaState,
    applyTaskUpdateToBindings,
    bindTaskToMediaState,
    createOverlayMediaState,
    isBusyOverlayState,
    resetOverlayMediaState,
    taskProgressRatio,
  },
});
