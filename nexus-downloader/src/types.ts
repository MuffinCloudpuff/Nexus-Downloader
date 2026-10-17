export type DownloadStatus = 'downloading' | 'resuming' | 'paused' | 'completed' | 'error' | 'waiting';
export type TaskType = 'video' | 'audio' | 'document' | 'archive' | 'program' | 'other';
export type ArtifactState = 'unknown' | 'available' | 'missing' | 'moved';

export interface DownloadTask {
  id: string;
  name: string;
  url: string;
  size: number; // in bytes
  downloaded: number; // in bytes
  progress: number; // 0.0 to 1.0 task progress from daemon
  status: DownloadStatus;
  speed: number; // bytes per second
  eta: number; // seconds
  platform?: 'youtube' | 'bilibili' | 'twitter' | 'other';
  thumbnail?: string;
  type: TaskType;
  createdAt: number;
  artifactState: ArtifactState;
  outputPath?: string;
  errorMessage?: string;
}
