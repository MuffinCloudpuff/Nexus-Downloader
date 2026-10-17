export type PopupView = "tasks" | "resources" | "advanced" | "settings";
// Resource collection remains available internally, but has no popup page.
export type VisiblePopupView = Exclude<PopupView, "resources">;
export type TaskAction = "toggle_pause" | "redownload" | "open_file" | "open_folder" | "cancel";
export type ThemePreference = "system" | "light" | "dark";

export type DesktopConnectionState =
  | "missing_token"
  | "connecting"
  | "authenticating"
  | "connected"
  | "unauthorized"
  | "disconnected";

export type ResourceFilter = "all" | "video" | "audio" | "streaming";
export type ResourceScope = "current" | "other";
export type ResourceCollectionState = "restoring" | "ready" | "unavailable";

export type AdvancedFeatureKey =
  | "recorder"
  | "recorder2"
  | "mobileUserAgent"
  | "search"
  | "catch";

export interface GenericTaskSummary {
  taskId: string;
  requestId?: string;
  traceId?: string;
  title: string;
  status: string;
  progress: number;
  receivedBytes: number;
  fileSize: number;
  speed: number;
  createdAt: number;
  canPause: boolean;
  canOpenFile: boolean;
  canOpenFolder: boolean;
  fileExt: string;
  packName: string;
  outputCategory?: "video" | "audio" | "image" | "document" | "archive" | "program" | "other";
  outputPath?: string;
  artifactState?: "unknown" | "available" | "missing" | "moved" | string;
  eventCursor?: number;
}

export interface CapturedResource {
  id: string;
  tabId: number;
  url: string;
  pageTitle: string;
  pageUrl: string;
  filename: string;
  mime: string;
  size: number;
  supportsRange: boolean;
  referer: string;
  requestHeaders: Record<string, string>;
  capturedAt: number;
  sentToDesktopAt?: number;
}

export interface TaskCounters {
  total: number;
  active: number;
  completed: number;
}

export interface FeatureStateMap {
  recorder: boolean;
  recorder2: boolean;
  mobileUserAgent: boolean;
  search: boolean;
  catch: boolean;
}

export interface MediaItemOption {
  index: number;
  label: string;
}

export interface MediaPlaybackState {
  available: boolean;
  message: string;
  tabId: number | null;
  mediaIndex: number;
  currentTime: number;
  duration: number;
  progress: number;
  volume: number;
  paused: boolean;
  loop: boolean;
  muted: boolean;
  speed: number;
}

export interface PopupStatePayload {
  connectionState: DesktopConnectionState;
  connectionMessage: string;
  desktopVersion: string;
  token: string;
  serverUrl: string;
  interceptDownloads: boolean;
  mediaDownloadOverlayEnabled: boolean;
  tasks: GenericTaskSummary[];
  taskCounters: TaskCounters;
  resourceState: ResourceCollectionState;
  resourceStateMessage: string;
  currentResources: CapturedResource[];
  otherResources: CapturedResource[];
  tabId: number | null;
  activePageDomain: string;
  featureStates: FeatureStateMap;
  mediaItems: MediaItemOption[];
  mediaPlaybackState: MediaPlaybackState;
}

export interface DesktopRequestResult {
  ok: boolean;
  message?: string;
  requestId?: string;
  traceId?: string;
  taskId?: string;
  eventCursor?: number;
}
