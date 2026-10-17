export type OutputCategory = "video" | "audio" | "image" | "document" | "archive" | "program" | "other";
export type ArtifactState = "unknown" | "available" | "missing" | "moved";
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

export interface CategoryDownloadPaths {
  video: string;
  audio: string;
  image: string;
  document: string;
  archive: string;
  program: string;
  other: string;
}

export type DownloadDirectoryMode = "flat" | "categorized" | "system_default" | "custom";

export interface AppSettingsDto {
  download_folder: string;
  max_task_num: number;
  max_chunk_connections: number;
  enable_speed_limitation: boolean;
  speed_limitation: number;
  proxy_server: string;
  directory_mode: DownloadDirectoryMode;
  categorized_root: string;
  custom_paths: CategoryDownloadPaths;
  system_paths: CategoryDownloadPaths;
}

export interface PatchAppSettingsDto {
  download_folder?: string;
  max_task_num?: number;
  max_chunk_connections?: number;
  enable_speed_limitation?: boolean;
  speed_limitation?: number;
  proxy_server?: string;
  directory_mode?: DownloadDirectoryMode;
  categorized_root?: string;
  custom_paths?: Partial<CategoryDownloadPaths>;
}

export interface SettingsUpdateResp {
  settings: AppSettingsDto;
  error?: TaskError;
}

export interface ClientContext {
  extension_id?: string;
  tab_id?: number;
  page_url?: string;
  protocol_version?: number;
  trace_id?: string;
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
  save_path?: string;
  headers?: Record<string, string>;
  resources?: CapturedResourcePayload[];
  encrypted_cookies?: string;
  cookie_iv?: string;
  filename?: string;
  mime?: string;
  size?: number;
  supports_range?: boolean;
}

export interface CreateTaskHttpReq {
  request_id: string;
  source?: "desktop_manual" | string;
  title: string;
  payload: CreateTaskPayload;
  client_context?: ClientContext;
}

export interface TaskError {
  code: string;
  message: string;
  retryable: boolean;
  stage?: string;
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

export interface TaskSummary {
  task_id: string;
  request_id?: string;
  trace_id?: string;
  title: string;
  created_at?: string;
  status: TaskStatus;
  artifact_state?: ArtifactState;
  progress: number;
  received_bytes: number;
  file_size: number;
  speed_bytes_per_sec: number;
  event_cursor: number;
  pack_name: string;
  output_category?: OutputCategory;
  output_path?: string;
  source?: string;
}

export interface GetTasksResp {
  tasks: TaskSummary[];
}

export interface TaskCommandResp {
  task_id: string;
  command: "pause" | "resume" | "cancel" | "delete" | "retry" | string;
  command_accepted: boolean;
  error?: TaskError;
}

export interface GetTaskEventsResp {
  task_id: string;
  from_event_id: number;
  events: TaskEvent[];
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

export interface RuntimeMetrics {
  running_tasks: number;
  queued_tasks: number;
  total_speed_bytes_per_sec: number;
  active_chunk_connections?: number | null;
  max_task_num: number;
  max_chunk_connections: number;
}

export interface StoragePathStats {
  path: string;
  exists: boolean;
  writable: boolean;
  available_bytes?: number | null;
  total_bytes?: number | null;
  error?: TaskError | null;
}

export interface AppVolumeStats {
  source_path: string;
  volume_root: string;
  available_bytes?: number | null;
  total_bytes?: number | null;
  error?: TaskError | null;
}

export interface BilibiliQrGenerateResp {
  qr_image_url?: string;
  qr_image_data_url?: string;
  session_key?: string;
  error?: string;
}

export interface BilibiliQrStatusResp {
  login_state: string;
  message: string;
}
