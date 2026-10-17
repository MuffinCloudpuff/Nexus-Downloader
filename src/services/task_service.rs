use crate::core::models::{ArtifactState, ErrorPayload, Task, TaskEventPayload, TaskStatus};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio::time::{Duration, sleep};

#[derive(Clone, Copy, Debug)]
pub enum TaskChangeReason {
    Added,
    Updated,
}

#[derive(Clone, Debug)]
pub struct TaskChangeEvent {
    pub task_id: String,
    pub reason: TaskChangeReason,
}

pub struct TaskService {
    tasks: Arc<RwLock<HashMap<String, Task>>>,
    flush_tx: mpsc::Sender<()>,
    change_tx: broadcast::Sender<TaskChangeEvent>,
    event_tx: broadcast::Sender<crate::core::models::TaskEvent>,
}

const PROGRESS_EVENT_DELTA: f64 = 0.01;
const RECEIVED_BYTES_EVENT_DELTA: u64 = 1024 * 1024;

fn task_event_type(payload: &TaskEventPayload) -> &'static str {
    match payload {
        TaskEventPayload::TaskAccepted => "task_accepted",
        TaskEventPayload::TaskQueued => "task_queued",
        TaskEventPayload::TaskParsingStarted => "task_parsing_started",
        TaskEventPayload::TaskResumingStarted => "task_resuming_started",
        TaskEventPayload::TaskStageChanged { .. } => "task_stage_changed",
        TaskEventPayload::TaskDownloadProgressUpdated { .. } => "task_download_progress_updated",
        TaskEventPayload::TaskMergeStarted => "task_merge_started",
        TaskEventPayload::TaskCompleted { .. } => "task_completed",
        TaskEventPayload::TaskArtifactAvailable { .. } => "task_artifact_available",
        TaskEventPayload::TaskArtifactMissing { .. } => "task_artifact_missing",
        TaskEventPayload::TaskArtifactMoved { .. } => "task_artifact_moved",
        TaskEventPayload::TaskFailed { .. } => "task_failed",
        TaskEventPayload::TaskCanceled => "task_canceled",
        TaskEventPayload::TaskDeleted => "task_deleted",
        TaskEventPayload::TaskPaused => "task_paused",
        TaskEventPayload::TaskResumed => "task_resumed",
        TaskEventPayload::TaskRetried { .. } => "task_retried",
    }
}

fn reduce_task_event(task: &mut Task, payload: &TaskEventPayload) {
    match payload {
        TaskEventPayload::TaskQueued => {
            task.status = TaskStatus::Queued;
        }
        TaskEventPayload::TaskAccepted => {}
        TaskEventPayload::TaskParsingStarted => {
            task.status = TaskStatus::Parsing;
        }
        TaskEventPayload::TaskResumingStarted => {
            task.status = TaskStatus::Resuming;
            task.speed = 0;
        }
        TaskEventPayload::TaskStageChanged { stage } => {
            task.status = TaskStatus::Other(stage.clone());
        }
        TaskEventPayload::TaskDownloadProgressUpdated {
            progress,
            received_bytes,
            speed_bytes_per_sec,
            ..
        } => {
            task.status = TaskStatus::Downloading;
            task.progress = *progress;
            task.received_bytes = *received_bytes;
            task.speed = *speed_bytes_per_sec;
        }
        TaskEventPayload::TaskMergeStarted => {
            task.status = TaskStatus::Merging;
        }
        TaskEventPayload::TaskCompleted { output_path } => {
            task.status = TaskStatus::Completed;
            task.progress = 1.0;
            task.speed = 0;
            task.path = output_path.clone();
            task.artifact_state = ArtifactState::Available;
        }
        TaskEventPayload::TaskArtifactAvailable { output_path } => {
            task.path = output_path.clone();
            task.artifact_state = ArtifactState::Available;
        }
        TaskEventPayload::TaskArtifactMissing { .. } => {
            task.artifact_state = ArtifactState::Missing;
        }
        TaskEventPayload::TaskArtifactMoved { new_path, .. } => {
            task.path = new_path.clone();
            task.artifact_state = ArtifactState::Moved;
        }
        TaskEventPayload::TaskFailed { .. } => {
            task.status = TaskStatus::Failed;
            task.speed = 0;
        }
        TaskEventPayload::TaskCanceled => {
            task.status = TaskStatus::Canceled;
            task.speed = 0;
        }
        TaskEventPayload::TaskDeleted => {}
        TaskEventPayload::TaskPaused => {
            task.status = TaskStatus::Paused;
            task.speed = 0;
        }
        TaskEventPayload::TaskResumed => {
            task.status = TaskStatus::Queued;
        }
        TaskEventPayload::TaskRetried { .. } => {
            task.status = TaskStatus::Queued;
            task.progress = 0.0;
            task.speed = 0;
            task.received_bytes = 0;
            task.artifact_state = ArtifactState::Unknown;
        }
    }
}

fn rebuild_task_snapshot_from_events(task: &mut Task) {
    if task.events.is_empty() {
        return;
    }
    task.status = TaskStatus::Queued;
    task.progress = 0.0;
    task.speed = 0;
    task.received_bytes = 0;
    let events = task.events.clone();
    for event in events {
        reduce_task_event(task, &event.payload);
    }
}

impl TaskService {
    pub fn new(app_data_dir: &str) -> Self {
        let tasks = Arc::new(RwLock::new(Self::load_from_disk(app_data_dir)));
        let (flush_tx, mut flush_rx) = mpsc::channel::<()>(100);
        let (change_tx, _) = broadcast::channel::<TaskChangeEvent>(128);
        let (event_tx, _) = broadcast::channel::<crate::core::models::TaskEvent>(1024);
        let tasks_clone = tasks.clone();

        let base_dir = app_data_dir.to_string();

        // 独立的防抖刷盘死循环任务 (Debounced Flush)
        tokio::spawn(async move {
            let mut pending_flush = false;
            loop {
                tokio::select! {
                    _ = flush_rx.recv() => {
                        pending_flush = true;
                    }
                    _ = sleep(Duration::from_millis(200)), if pending_flush => {
                        pending_flush = false;
                        if let Err(e) = Self::flush_to_disk(&tasks_clone, &base_dir).await {
                            eprintln!("Failed to flush tasks: {}", e);
                        }
                    }
                }
            }
        });

        Self {
            tasks,
            flush_tx,
            change_tx,
            event_tx,
        }
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<crate::core::models::TaskEvent> {
        self.event_tx.subscribe()
    }

    pub fn subscribe_task_changes(&self) -> broadcast::Receiver<TaskChangeEvent> {
        self.change_tx.subscribe()
    }

    pub async fn add_task(&self, mut task: Task) -> Result<()> {
        let task_id = task.id.clone();
        if task.created_at.trim().is_empty() {
            task.created_at = chrono::Utc::now().to_rfc3339();
        }
        let mut initial_events = Vec::new();
        if task.events.is_empty() {
            if let Some(request_id) = task.request_id.clone() {
                task.event_cursor += 1;
                initial_events.push(crate::core::models::TaskEvent {
                    event_id: task.event_cursor,
                    task_id: task_id.clone(),
                    request_id: Some(request_id.clone()),
                    trace_id: task.trace_id.clone(),
                    occurred_at: chrono::Utc::now().to_rfc3339(),
                    payload: TaskEventPayload::TaskAccepted,
                });
            }
            task.event_cursor += 1;
            initial_events.push(crate::core::models::TaskEvent {
                event_id: task.event_cursor,
                task_id: task_id.clone(),
                request_id: task.request_id.clone(),
                trace_id: task.trace_id.clone(),
                occurred_at: chrono::Utc::now().to_rfc3339(),
                payload: TaskEventPayload::TaskQueued,
            });
            for event in &initial_events {
                reduce_task_event(&mut task, &event.payload);
                task.events.push(event.clone());
            }
        }

        let mut guard = self.tasks.write().await;
        guard.insert(task_id.clone(), task);
        drop(guard);

        for event in initial_events {
            tracing::info!(
                task_id = %event.task_id,
                event_id = event.event_id,
                request_id = ?event.request_id,
                trace_id = ?event.trace_id,
                event_type = task_event_type(&event.payload),
                "task event emitted"
            );
            let _ = self.event_tx.send(event);
        }
        let _ = self.flush_tx.send(()).await;
        let _ = self.change_tx.send(TaskChangeEvent {
            task_id,
            reason: TaskChangeReason::Added,
        });
        Ok(())
    }

    pub async fn get_tasks(&self) -> Vec<Task> {
        let guard = self.tasks.read().await;
        guard.values().cloned().collect()
    }

    pub async fn get_task(&self, task_id: &str) -> Option<Task> {
        let guard = self.tasks.read().await;
        guard.get(task_id).cloned()
    }

    pub async fn remove_task(&self, task_id: &str) -> Option<Task> {
        let mut guard = self.tasks.write().await;
        let removed = guard.remove(task_id);
        drop(guard);

        if let Some(ref task) = removed {
            let _ = self.flush_tx.send(()).await;
            let _ = self.change_tx.send(TaskChangeEvent {
                task_id: task_id.to_string(),
                reason: TaskChangeReason::Updated,
            });

            // Broadcast the deletion to all clients via SSE / Event bus
            let event = crate::core::models::TaskEvent {
                event_id: task.event_cursor + 1,
                task_id: task_id.to_string(),
                request_id: task.request_id.clone(),
                trace_id: task.trace_id.clone(),
                occurred_at: chrono::Utc::now().to_rfc3339(),
                payload: TaskEventPayload::TaskDeleted,
            };
            tracing::info!(
                task_id = %event.task_id,
                event_id = event.event_id,
                request_id = ?event.request_id,
                trace_id = ?event.trace_id,
                event_type = "task_deleted",
                "task event emitted"
            );
            let _ = self.event_tx.send(event);
        }
        removed
    }

    pub async fn update_task_output_path(&self, task_id: &str, new_path: String) -> Option<Task> {
        let old_path = {
            let guard = self.tasks.read().await;
            let task = guard.get(task_id)?;
            if task.path == new_path {
                return Some(task.clone());
            }
            task.path.clone()
        };

        self.append_event(
            task_id,
            TaskEventPayload::TaskArtifactMoved { old_path, new_path },
        )
        .await;
        self.get_task(task_id).await
    }

    pub async fn mark_task_artifact_available(
        &self,
        task_id: &str,
        output_path: String,
    ) -> Option<Task> {
        let should_update = {
            let guard = self.tasks.read().await;
            let task = guard.get(task_id)?;
            task.path != output_path || task.artifact_state != ArtifactState::Available
        };
        if !should_update {
            return self.get_task(task_id).await;
        }

        self.append_event(
            task_id,
            TaskEventPayload::TaskArtifactAvailable { output_path },
        )
        .await;
        self.get_task(task_id).await
    }

    pub async fn mark_task_artifact_missing(
        &self,
        task_id: &str,
        output_path: String,
    ) -> Option<Task> {
        let should_update = {
            let guard = self.tasks.read().await;
            let task = guard.get(task_id)?;
            task.artifact_state != ArtifactState::Missing
        };
        if !should_update {
            return self.get_task(task_id).await;
        }

        self.append_event(
            task_id,
            TaskEventPayload::TaskArtifactMissing { output_path },
        )
        .await;
        self.get_task(task_id).await
    }

    pub async fn get_events_since(
        &self,
        task_id: &str,
        from_event_id: u64,
    ) -> Option<Vec<crate::core::models::TaskEvent>> {
        let guard = self.tasks.read().await;
        guard.get(task_id).map(|task| {
            task.events
                .iter()
                .filter(|event| event.event_id >= from_event_id)
                .cloned()
                .collect()
        })
    }

    pub async fn append_event(
        &self,
        task_id: &str,
        payload: crate::core::models::TaskEventPayload,
    ) {
        let mut guard = self.tasks.write().await;
        if let Some(task) = guard.get_mut(task_id) {
            task.event_cursor += 1;
            let event = crate::core::models::TaskEvent {
                event_id: task.event_cursor,
                task_id: task_id.to_string(),
                request_id: task.request_id.clone(),
                trace_id: task.trace_id.clone(),
                occurred_at: chrono::Utc::now().to_rfc3339(),
                payload: payload.clone(),
            };

            reduce_task_event(task, &payload);

            task.events.push(event.clone());
            tracing::info!(
                task_id = %event.task_id,
                event_id = event.event_id,
                request_id = ?event.request_id,
                trace_id = ?event.trace_id,
                event_type = task_event_type(&event.payload),
                status = %task.status,
                "task event emitted"
            );
            let _ = self.event_tx.send(event);
            let _ = self.flush_tx.send(()).await;
            let _ = self.change_tx.send(TaskChangeEvent {
                task_id: task_id.to_string(),
                reason: TaskChangeReason::Updated,
            });
        }
    }

    pub async fn complete_task(&self, task_id: &str, output_path: impl Into<String>) {
        self.append_event(
            task_id,
            TaskEventPayload::TaskCompleted {
                output_path: output_path.into(),
            },
        )
        .await;
    }

    pub async fn fail_task(&self, task_id: &str, failure: ErrorPayload) {
        self.append_event(task_id, TaskEventPayload::TaskFailed { failure })
            .await;
    }

    pub async fn reset_task_for_retry(&self, task_id: &str) -> Option<Task> {
        let event = {
            let mut guard = self.tasks.write().await;
            let task = guard.get_mut(task_id)?;
            let previous_status = task.status.clone();
            task.status = TaskStatus::Queued;
            task.progress = 0.0;
            task.speed = 0;
            task.received_bytes = 0;
            task.artifact_state = ArtifactState::Unknown;
            task.event_cursor += 1;
            let event = crate::core::models::TaskEvent {
                event_id: task.event_cursor,
                task_id: task_id.to_string(),
                request_id: task.request_id.clone(),
                trace_id: task.trace_id.clone(),
                occurred_at: chrono::Utc::now().to_rfc3339(),
                payload: TaskEventPayload::TaskRetried { previous_status },
            };
            task.events.push(event.clone());
            event
        };

        tracing::info!(
            task_id = %event.task_id,
            event_id = event.event_id,
            request_id = ?event.request_id,
            trace_id = ?event.trace_id,
            event_type = task_event_type(&event.payload),
            "task event emitted"
        );
        let _ = self.event_tx.send(event);
        let _ = self.flush_tx.send(()).await;
        let _ = self.change_tx.send(TaskChangeEvent {
            task_id: task_id.to_string(),
            reason: TaskChangeReason::Updated,
        });

        self.get_task(task_id).await
    }

    pub async fn update_task_status(
        &self,
        task_id: &str,
        status: crate::core::models::TaskStatus,
        progress: f64,
        speed: u64,
        received_bytes: u64,
    ) {
        let (status_changed, progress_changed, received_bytes_changed, speed_changed) = {
            let guard = self.tasks.read().await;
            if let Some(task) = guard.get(task_id) {
                let status_changed = task.status != status;
                let last_progress_event = task
                    .events
                    .iter()
                    .rev()
                    .find_map(|event| match &event.payload {
                        TaskEventPayload::TaskDownloadProgressUpdated { progress, .. } => {
                            Some(*progress)
                        }
                        _ => None,
                    })
                    .unwrap_or(task.progress);
                let last_received_bytes_event = task
                    .events
                    .iter()
                    .rev()
                    .find_map(|event| match &event.payload {
                        TaskEventPayload::TaskDownloadProgressUpdated {
                            received_bytes, ..
                        } => Some(*received_bytes),
                        _ => None,
                    })
                    .unwrap_or(task.received_bytes);
                let last_speed_event = task
                    .events
                    .iter()
                    .rev()
                    .find_map(|event| match &event.payload {
                        TaskEventPayload::TaskDownloadProgressUpdated {
                            speed_bytes_per_sec,
                            ..
                        } => Some(*speed_bytes_per_sec),
                        _ => None,
                    })
                    .unwrap_or(task.speed);
                let progress_changed =
                    (last_progress_event - progress).abs() >= PROGRESS_EVENT_DELTA;
                let received_bytes_changed = received_bytes
                    .saturating_sub(last_received_bytes_event)
                    >= RECEIVED_BYTES_EVENT_DELTA;
                let speed_changed = speed > 0 && last_speed_event == 0;
                (
                    status_changed,
                    progress_changed,
                    received_bytes_changed,
                    speed_changed,
                )
            } else {
                return;
            }
        };

        if status_changed || progress_changed || received_bytes_changed || speed_changed {
            let payload = match status {
                TaskStatus::Queued => TaskEventPayload::TaskQueued,
                TaskStatus::Parsing => TaskEventPayload::TaskParsingStarted,
                TaskStatus::Resuming => TaskEventPayload::TaskResumingStarted,
                TaskStatus::Downloading => TaskEventPayload::TaskDownloadProgressUpdated {
                    stage: "downloading".to_string(),
                    progress,
                    received_bytes,
                    total_bytes: 0,
                    speed_bytes_per_sec: speed,
                },
                TaskStatus::Merging => TaskEventPayload::TaskMergeStarted,
                TaskStatus::Completed => {
                    tracing::warn!(
                        task_id,
                        "update_task_status called for completed; use complete_task with output_path"
                    );
                    return;
                }
                TaskStatus::Failed => {
                    tracing::warn!(
                        task_id,
                        "update_task_status called for failed; use fail_task with structured failure"
                    );
                    return;
                }
                TaskStatus::Canceled => TaskEventPayload::TaskCanceled,
                TaskStatus::Paused => TaskEventPayload::TaskPaused,
                TaskStatus::Other(_) => TaskEventPayload::TaskStageChanged {
                    stage: "other".to_string(),
                },
            };
            // append_event will update the task in memory and fire the change event
            self.append_event(task_id, payload).await;
        }
    }

    pub async fn schedule_flush(&self) {
        let _ = self.flush_tx.send(()).await;
    }

    async fn flush_to_disk(
        tasks: &Arc<RwLock<HashMap<String, Task>>>,
        base_dir: &str,
    ) -> Result<()> {
        let guard = tasks.read().await;
        let mut lines = Vec::new();
        for task in guard.values() {
            let json = serde_json::to_string(&Self::redacted_task_for_persistence(task))?;
            lines.push(json);
        }

        let content = lines.join("\n") + "\n";

        let target_path = Path::new(base_dir).join("Memory.log");
        let temp_path = Path::new(base_dir).join("Memory.log.tmp");

        // 保证原子写入防损坏：先写临时文件，然后快速覆盖重命名
        tokio::fs::write(&temp_path, content).await?;
        tokio::fs::rename(&temp_path, &target_path).await?;
        Ok(())
    }

    fn load_from_disk(base_dir: &str) -> HashMap<String, Task> {
        let path = Path::new(base_dir).join("Memory.log");
        let Ok(content) = std::fs::read_to_string(path) else {
            return HashMap::new();
        };

        let mut tasks = HashMap::new();
        for (line_no, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<Task>(trimmed) {
                Ok(mut task) => {
                    if task.created_at.trim().is_empty() {
                        task.created_at = task
                            .events
                            .first()
                            .map(|event| event.occurred_at.clone())
                            .unwrap_or_default();
                    }
                    if Self::is_legacy_dirty_completed_task(&task) {
                        tracing::warn!(
                            task_id = %task.id,
                            title = %task.title,
                            pack_id = ?task.pack_id,
                            "skipping legacy dirty completed task without request/event metadata"
                        );
                        continue;
                    }
                    rebuild_task_snapshot_from_events(&mut task);
                    // Old versions recorded images as Other. Repair only the
                    // category; never move/rename historical downloaded files.
                    use crate::parsers::policy::output_category::{OutputCategory, is_image_path};
                    if matches!(task.output_category, None | Some(OutputCategory::Other))
                        && is_image_path(&task.path)
                    {
                        task.output_category = Some(OutputCategory::Image);
                    }
                    tasks.insert(task.id.clone(), task);
                }
                Err(error) => {
                    tracing::warn!(line_no = line_no + 1, %error, "failed to load persisted task line");
                }
            }
        }
        tracing::info!(task_count = tasks.len(), "loaded persisted tasks");
        tasks
    }

    fn is_legacy_dirty_completed_task(task: &Task) -> bool {
        task.request_id.is_none()
            && task.events.is_empty()
            && task.event_cursor == 0
            && task.received_bytes == 0
            && task.status == crate::core::models::TaskStatus::Completed
            && task.url.starts_with("gd3+hls://")
    }

    fn redacted_task_for_persistence(task: &Task) -> Task {
        let mut redacted = task.clone();
        redacted.cookie_str = None;
        for stage in &mut redacted.stages {
            stage.headers.retain(|key, _| {
                let lower = key.to_ascii_lowercase();
                lower != "cookie" && lower != "authorization" && !lower.ends_with("token")
            });
        }
        redacted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::{ArtifactState, PackId, TaskEventPayload};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_data_dir(name: &str) -> String {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("gd3-task-service-{name}-{unique}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    fn test_task(id: &str) -> Task {
        Task::new(
            id.to_string(),
            "video".to_string(),
            "https://example.com/video.mp4".to_string(),
            PackId::Http,
            0,
            "downloads/video.mp4".to_string(),
        )
    }

    #[tokio::test]
    async fn task_snapshot_is_reduced_from_events() {
        let service = TaskService::new(&temp_data_dir("reduce"));
        service.add_task(test_task("task-reduce")).await.unwrap();

        service
            .update_task_status("task-reduce", TaskStatus::Downloading, 0.25, 1024, 2048)
            .await;
        let task = service.get_task("task-reduce").await.unwrap();
        assert_eq!(task.status, TaskStatus::Downloading);
        assert_eq!(task.progress, 0.25);
        assert_eq!(task.speed, 1024);
        assert_eq!(task.received_bytes, 2048);
        assert!(task.events.iter().any(|event| matches!(
            event.payload,
            TaskEventPayload::TaskDownloadProgressUpdated { .. }
        )));

        service
            .update_task_status("task-reduce", TaskStatus::Downloading, 0.251, 1024, 2050)
            .await;
        let unchanged = service.get_task("task-reduce").await.unwrap();
        assert_eq!(unchanged.progress, 0.25);
        assert_eq!(unchanged.received_bytes, 2048);
    }

    #[test]
    fn old_image_records_are_reclassified_without_changing_paths() {
        use crate::parsers::policy::output_category::OutputCategory;
        let root = temp_data_dir("legacy-image");
        let mut photo = test_task("photo");
        photo.path = "E:/selected/原图.jpg".into();
        photo.output_category = Some(OutputCategory::Other);
        std::fs::write(
            Path::new(&root).join("Memory.log"),
            serde_json::to_string(&photo).unwrap(),
        )
        .unwrap();
        let loaded = TaskService::load_from_disk(&root);
        assert_eq!(loaded["photo"].output_category, Some(OutputCategory::Image));
        assert_eq!(loaded["photo"].path, photo.path);
    }

    #[tokio::test]
    async fn terminal_events_keep_output_path_and_failure_details() {
        let service = TaskService::new(&temp_data_dir("terminal"));
        service.add_task(test_task("task-terminal")).await.unwrap();

        service
            .complete_task("task-terminal", "downloads/final.mp4")
            .await;
        let completed = service.get_task("task-terminal").await.unwrap();
        assert_eq!(completed.status, TaskStatus::Completed);
        assert_eq!(completed.progress, 1.0);
        assert_eq!(completed.path, "downloads/final.mp4");
        assert_eq!(completed.artifact_state, ArtifactState::Available);
        assert!(completed.events.iter().any(|event| matches!(
            &event.payload,
            TaskEventPayload::TaskCompleted { output_path } if output_path == "downloads/final.mp4"
        )));

        service.add_task(test_task("task-failed")).await.unwrap();
        service
            .fail_task(
                "task-failed",
                ErrorPayload {
                    code: "execution_failed".to_string(),
                    message: "network closed".to_string(),
                    retryable: false,
                    stage: Some("execution".to_string()),
                },
            )
            .await;
        let failed = service.get_task("task-failed").await.unwrap();
        assert_eq!(failed.status, TaskStatus::Failed);
        assert!(failed.events.iter().any(|event| matches!(
            &event.payload,
            TaskEventPayload::TaskFailed { failure }
                if failure.code == "execution_failed"
                    && failure.message == "network closed"
                    && failure.stage.as_deref() == Some("execution")
        )));
    }

    #[tokio::test]
    async fn update_task_output_path_updates_snapshot_and_persistence() {
        let dir = temp_data_dir("path-update");
        let service = TaskService::new(&dir);
        service.add_task(test_task("task-path")).await.unwrap();
        service
            .complete_task("task-path", "downloads/original.mp4")
            .await;

        let updated = service
            .update_task_output_path("task-path", "downloads/moved.mp4".to_string())
            .await
            .unwrap();
        assert_eq!(updated.path, "downloads/moved.mp4");
        assert_eq!(updated.artifact_state, ArtifactState::Moved);
        assert_eq!(
            service.get_task("task-path").await.unwrap().path,
            "downloads/moved.mp4"
        );

        service.schedule_flush().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let persisted = std::fs::read_to_string(Path::new(&dir).join("Memory.log")).unwrap();
        assert!(persisted.contains("downloads/moved.mp4"));
    }

    #[tokio::test]
    async fn new_tasks_have_backend_creation_time_and_unknown_artifact_state() {
        let service = TaskService::new(&temp_data_dir("created-at"));
        service.add_task(test_task("task-created")).await.unwrap();

        let task = service.get_task("task-created").await.unwrap();
        assert!(!task.created_at.trim().is_empty());
        assert!(chrono::DateTime::parse_from_rfc3339(&task.created_at).is_ok());
        assert_eq!(task.artifact_state, ArtifactState::Unknown);
    }

    #[tokio::test]
    async fn missing_artifact_update_keeps_task_in_snapshot() {
        let service = TaskService::new(&temp_data_dir("missing-artifact"));
        service.add_task(test_task("task-missing")).await.unwrap();
        service
            .complete_task("task-missing", "downloads/final.mp4")
            .await;

        let updated = service
            .mark_task_artifact_missing("task-missing", "downloads/final.mp4".to_string())
            .await
            .unwrap();

        assert_eq!(updated.status, TaskStatus::Completed);
        assert_eq!(updated.path, "downloads/final.mp4");
        assert_eq!(updated.artifact_state, ArtifactState::Missing);
        assert!(service.get_task("task-missing").await.is_some());
    }

    #[tokio::test]
    async fn loading_from_disk_rebuilds_snapshot_from_events() {
        let dir = temp_data_dir("load-reducer");
        let mut task = test_task("task-load");
        task.status = TaskStatus::Failed;
        task.progress = 0.12;
        task.event_cursor = 1;
        task.events.push(crate::core::models::TaskEvent {
            event_id: 1,
            task_id: task.id.clone(),
            request_id: None,
            trace_id: None,
            occurred_at: chrono::Utc::now().to_rfc3339(),
            payload: TaskEventPayload::TaskCompleted {
                output_path: "downloads/rebuilt.mp4".to_string(),
            },
        });
        let line = serde_json::to_string(&task).unwrap();
        std::fs::write(Path::new(&dir).join("Memory.log"), format!("{line}\n")).unwrap();

        let loaded = TaskService::new(&dir);
        let rebuilt = loaded.get_task("task-load").await.unwrap();
        assert_eq!(rebuilt.status, TaskStatus::Completed);
        assert_eq!(rebuilt.progress, 1.0);
    }
}
