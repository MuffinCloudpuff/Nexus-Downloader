import type { DownloadStatus, DownloadTask, TaskType } from "../types";
import type { TaskEvent, TaskSummary } from "../protocol/daemon";

function normalizeStatus(status: string): DownloadStatus {
  switch (status) {
    case "downloading":
    case "parsing":
    case "merging":
    case "queued":
      return "downloading";
    case "resuming":
      return "resuming";
    case "paused":
      return "paused";
    case "completed":
      return "completed";
    case "canceled":
    case "failed":
      return "error";
    default:
      return "waiting";
  }
}

function taskTypeOf(task: TaskSummary): TaskType {
  if (task.output_category && task.output_category !== "other") {
    return task.output_category;
  }
  const ext = fileExtension(task.output_path || task.title);
  if (["jpg", "jpeg", "jfif", "png", "gif", "webp", "avif", "bmp", "svg", "tif", "tiff", "ico", "heic", "heif", "jxl"].includes(ext)) {
    return "image";
  }
  if ([
    "mp4", "mkv", "webm", "mov", "avi", "flv", "m4v", "m3u8", "mpd", "wmv",
    "mpg", "mpeg", "3gp", "ogv", "vob", "mts", "m2ts", "h264", "h265", "hevc",
  ].includes(ext)) {
    return "video";
  }
  if (["mp3", "flac", "wav", "m4a", "aac", "ogg"].includes(ext)) {
    return "audio";
  }
  if (["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md"].includes(ext)) {
    return "document";
  }
  if (["zip", "7z", "rar", "tar", "gz"].includes(ext)) {
    return "archive";
  }
  if ([
    "exe", "msi", "deb", "dmg", "apk", "pkg", "appimage", "wasm",
    "c", "h", "cc", "cpp", "cxx", "hpp", "hxx", "m", "mm", "cs", "java", "kt",
    "kts", "swift", "go", "rs", "py", "pyw", "js", "jsx", "mjs", "cjs", "ts",
    "tsx", "php", "rb", "pl", "pm", "lua", "r", "jl", "dart", "scala", "sc",
    "clj", "cljs", "ex", "exs", "erl", "hrl", "hs", "lhs", "ml", "mli", "fs",
    "fsi", "fsx", "vb", "zig", "nim", "cr", "d", "pas", "pp", "asm", "s",
    "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd", "vbs", "sql", "html",
    "css", "scss", "sass", "less", "vue", "svelte", "glsl", "hlsl", "wgsl",
    "ipynb", "cmake", "gradle",
  ].includes(ext)) {
    return "program";
  }
  return "other";
}

function platformOf(task: TaskSummary): DownloadTask["platform"] {
  if (taskTypeOf(task) !== "video") return "other";
  // Platform identity belongs to the downloader/source, never a directory name.
  const identity = `${task.pack_name || ""} ${task.source || ""}`.toLowerCase();
  const capturedMedia = task.source === "page_media";
  if (identity.includes("bilibili") || (capturedMedia && task.title.startsWith("B-"))) {
    return "bilibili";
  }
  if (identity.includes("youtube") || (capturedMedia && task.title.startsWith("Y-"))) {
    return "youtube";
  }
  if (identity.includes("twitter") || ((capturedMedia || task.pack_name === "hls") && task.title.startsWith("X-"))) {
    return "twitter";
  }
  return "other";
}

function fileExtension(value: string): string {
  const clean = value.split(/[?#]/)[0] || "";
  const name = clean.split(/[\\/]/).pop() || "";
  const index = name.lastIndexOf(".");
  return index >= 0 ? name.slice(index + 1).toLowerCase() : "";
}

function normalizeProgress(value: unknown, status?: DownloadStatus): number {
  if (status === "completed") {
    return 1;
  }
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    return 0;
  }
  return Math.min(1, Math.max(0, parsed));
}

function createdAtFromBackend(value: unknown): number {
  if (typeof value !== "string" || !value.trim()) {
    return 0;
  }
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

export function taskSummaryToDownloadTask(task: TaskSummary, existing?: DownloadTask): DownloadTask {
  const size = Number(task.file_size || existing?.size || 0);
  const status = normalizeStatus(String(task.status || "queued"));
  const downloaded = status === "completed" && size > 0
    ? size
    : Number(task.received_bytes || existing?.downloaded || 0);
  const speed = Number(task.speed_bytes_per_sec || 0);
  const remainingBytes = Math.max(0, size - downloaded);
  return {
    id: task.task_id,
    name: task.title || task.output_path || task.task_id,
    url: existing?.url || task.output_path || "",
    size,
    downloaded,
    progress: normalizeProgress(task.progress, status),
    status,
    speed,
    eta: speed > 0 ? Math.ceil(remainingBytes / speed) : existing?.eta || 0,
    platform: platformOf(task),
    type: taskTypeOf(task),
    outputPath: task.output_path || existing?.outputPath,
    createdAt: createdAtFromBackend(task.created_at),
    artifactState: task.artifact_state || "unknown",
  };
}

export function applyTaskEvent(task: DownloadTask, event: TaskEvent): DownloadTask {
  switch (event.type) {
    case "task_download_progress_updated": {
      const received = Number(event.received_bytes ?? task.downloaded);
      const eventTotal = Number(event.total_bytes);
      const total = Number.isFinite(eventTotal) && eventTotal > 0 ? eventTotal : task.size;
      const speed = Number(event.speed_bytes_per_sec ?? task.speed);
      const fallbackProgress = total > 0 ? received / total : task.progress;
      return {
        ...task,
        status: "downloading",
        downloaded: received,
        size: total,
        progress: normalizeProgress(event.progress ?? fallbackProgress),
        speed,
        eta: speed > 0 ? Math.ceil(Math.max(0, total - received) / speed) : task.eta,
      };
    }
    case "task_completed":
      return {
        ...task,
        status: "completed",
        downloaded: task.size > 0 ? task.size : task.downloaded,
        progress: 1,
        speed: 0,
        eta: 0,
        outputPath: event.output_path || task.outputPath,
        artifactState: "available",
      };
    case "task_artifact_available":
      return {
        ...task,
        outputPath: event.output_path || task.outputPath,
        artifactState: "available",
      };
    case "task_artifact_missing":
      return {
        ...task,
        artifactState: "missing",
      };
    case "task_artifact_moved":
      return {
        ...task,
        outputPath: event.new_path || task.outputPath,
        artifactState: "moved",
      };
    case "task_canceled":
    case "task_failed":
      return { ...task, status: "error", speed: 0 };
    case "task_paused":
      return { ...task, status: "paused", speed: 0 };
    case "task_resuming_started":
      return { ...task, status: "resuming", speed: 0, progress: normalizeProgress(task.progress) };
    case "task_resumed":
    case "task_retried":
    case "task_parsing_started":
    case "task_queued":
      return { ...task, status: "downloading", progress: normalizeProgress(task.progress) };
    default:
      return task;
  }
}
