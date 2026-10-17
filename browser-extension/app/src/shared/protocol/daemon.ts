import type {GenericTaskSummary} from "../types";
import {fileExtension} from "../utils";

export type TaskStatus =
  | "queued"
  | "parsing"
  | "resuming"
  | "downloading"
  | "merging"
  | "completed"
  | "failed"
  | "canceled"
  | "paused"
  | string;

export type ErrorCode =
  | "invalid_request"
  | "token_invalid"
  | "cookie_decryption_failed"
  | "unsupported_media"
  | "idempotency_conflict"
  | "network_timeout"
  | "merge_failed"
  | "internal_error"
  | string;

export interface TaskError {
  code: ErrorCode;
  message: string;
  retryable: boolean;
  stage?: string;
}

export interface CapturedResourcePayload {
  kind?: string;
  url: string;
  mime?: string;
  filename?: string;
  size?: number;
  supports_range?: boolean;
  headers?: Record<string, string>;
  page_title?: string;
  page_url?: string;
}

export interface CreateTaskPayload {
  url: string;
  headers?: Record<string, string>;
  resources?: CapturedResourcePayload[];
  encrypted_cookies?: string;
  cookie_iv?: string;
  filename?: string;
  save_path?: string;
  mime?: string;
  size?: number;
  supports_range?: boolean;
}

export interface ClientContext {
  extension_id?: string;
  tab_id?: number;
  page_url?: string;
  protocol_version?: number;
  trace_id?: string;
}

export type CreateTaskSource = "page_media" | "resource" | "resource_merge" | "download" | string;

export interface CreateTaskHttpReq {
  request_id: string;
  source?: CreateTaskSource;
  title: string;
  payload: CreateTaskPayload;
  client_context?: ClientContext;
}

export interface CreateTaskHttpResp {
  request_id: string;
  trace_id?: string;
  task_id?: string;
  accepted: boolean;
  initial_status?: TaskStatus;
  event_cursor?: number;
  error?: TaskError;
}

export interface TaskCommandResp {
  task_id: string;
  command: string;
  command_accepted: boolean;
  error?: TaskError;
}

export interface TaskSummary {
  task_id: string;
  request_id?: string;
  trace_id?: string;
  title: string;
  status: TaskStatus;
  progress: number;
  received_bytes: number;
  file_size: number;
  speed_bytes_per_sec: number;
  event_cursor: number;
  pack_name: string;
  output_category?: "video" | "audio" | "image" | "document" | "archive" | "program" | "other";
  output_path?: string;
  source?: string;
  artifact_state?: "unknown" | "available" | "missing" | "moved" | string;
}

export interface GetTasksResp {
  tasks: TaskSummary[];
}

export interface GetTaskEventsResp {
  task_id: string;
  from_event_id: number;
  events: TaskEvent[];
}

export interface PairingRequest {
  request_id?: string;
  client_context?: ClientContext;
}

export interface PairingResponse {
  request_id?: string;
  pairing_issued: boolean;
  token?: string;
  error?: TaskError;
}

export interface TaskEventBase {
  event_id: number;
  task_id: string;
  request_id?: string;
  trace_id?: string;
  occurred_at: string;
  type: string;
}

export type TaskEvent = TaskEventBase & {
  stage?: string;
  progress?: number;
  received_bytes?: number;
  total_bytes?: number;
  speed_bytes_per_sec?: number;
  output_path?: string;
  old_path?: string;
  new_path?: string;
  failure?: TaskError;
};

export interface ClientTelemetryEvent {
  occurred_at?: string;
  trace_id?: string;
  request_id?: string;
  task_id?: string;
  event_type: string;
  surface?: string;
  action?: string;
  source?: string;
  status?: string;
  message?: string;
  tab_id?: number;
  page_url?: string;
  url_host?: string;
  url_path_kind?: string;
  details?: Record<string, unknown>;
}

export interface ClientTelemetryResp {
  accepted: boolean;
  event_count: number;
}

export function isTerminalTaskStatus(status: string): boolean {
  return status === "completed" || status === "failed" || status === "canceled";
}

export function normalizeTaskProgress(value: number): number {
  if (!Number.isFinite(value)) {
    return 0;
  }
  const normalized = value > 1 ? value / 100 : value;
  return Math.max(0, Math.min(1, normalized));
}

export function daemonTaskToGenericTask(task: TaskSummary, fallbackCreatedAt = Date.now()): GenericTaskSummary {
  const status = String(task.status || "queued");
  const title = task.title || task.task_id;
  const fileExt = fileExtension(task.output_path || title);
  return {
    taskId: task.task_id,
    requestId: task.request_id,
    traceId: task.trace_id,
    title,
    status,
    progress: normalizeTaskProgress(Number(task.progress ?? 0)),
    receivedBytes: Number(task.received_bytes ?? 0),
    fileSize: Number(task.file_size ?? 0),
    speed: Number(task.speed_bytes_per_sec ?? 0),
    createdAt: fallbackCreatedAt,
    canPause: status === "queued" || status === "parsing" || status === "resuming" || status === "downloading" || status === "merging",
    canOpenFile: false,
    canOpenFolder: false,
    fileExt,
    packName: task.pack_name || task.source || "",
    outputCategory: task.output_category,
    outputPath: task.output_path,
    artifactState: task.artifact_state || "unknown",
    eventCursor: Number(task.event_cursor ?? 0),
  };
}
