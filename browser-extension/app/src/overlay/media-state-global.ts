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
import {fullscreenOverlayMode, mediaCandidates, overlayPosition, taskActionForOverlay} from "./media-controls";
import {claimMediaOverlay} from "./media-lifecycle";

Object.assign(globalThis, {
  GD3OverlayMediaControls: {claimMediaOverlay, fullscreenOverlayMode, mediaCandidates, overlayPosition, taskActionForOverlay},
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
