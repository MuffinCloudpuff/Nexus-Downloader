import type { DownloadTask } from "../types";

export function compareDownloadTasksForDisplay(a: DownloadTask, b: DownloadTask) {
  const aMissing = a.artifactState === "missing";
  const bMissing = b.artifactState === "missing";

  if (aMissing !== bMissing) {
    return aMissing ? 1 : -1;
  }

  return (b.createdAt || 0) - (a.createdAt || 0);
}
