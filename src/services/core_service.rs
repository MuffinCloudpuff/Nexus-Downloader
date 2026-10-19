use crate::core::engine::executor::{DownloadExecutor, ExecutionFailure};
use crate::core::execution_plan::ExecutionPlan;
use crate::core::models::{ErrorPayload, ExecutionKind, Task, TaskStatus};
use crate::protocol::metrics::RuntimeMetrics;
use crate::services::config::get_config;
use crate::services::runtime_metrics_service;
use crate::services::task_artifact_service;
use crate::services::task_service::{TaskChangeEvent, TaskService};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore, broadcast};
use tokio::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum TaskControlError {
    NotFound,
    InvalidState {
        command: &'static str,
        status: TaskStatus,
    },
    UnsupportedOperation {
        command: &'static str,
        execution_kind: ExecutionKind,
    },
    CleanupFailed {
        command: &'static str,
        message: String,
    },
}

impl std::fmt::Display for TaskControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskControlError::NotFound => write!(f, "task not found"),
            TaskControlError::InvalidState { command, status } => {
                write!(f, "cannot {command} task while status is {status}")
            }
            TaskControlError::UnsupportedOperation {
                command,
                execution_kind,
            } => write!(f, "{command} is not supported for {execution_kind:?} tasks"),
            TaskControlError::CleanupFailed { command, message } => {
                write!(f, "{command} cleanup failed: {message}")
            }
        }
    }
}

impl std::error::Error for TaskControlError {}

pub struct TaskQueueState {
    pub running_tasks: std::collections::HashMap<String, tokio_util::sync::CancellationToken>,
    pub waiting_tasks: Vec<Task>,
}

#[derive(Clone)]
struct SchedulerParts {
    task_service: Arc<TaskService>,
    concurrency_limiter: Arc<Semaphore>,
    queue_state: Arc<RwLock<TaskQueueState>>,
    resume_store: Arc<crate::core::resume::ResumeStore>,
}

pub struct CoreService {
    task_service: Arc<TaskService>,
    // 严格限制最大并发任务数
    concurrency_limiter: Arc<Semaphore>,
    // 统一锁：同时包含 running_tasks 和 waiting_tasks，防死锁
    queue_state: Arc<RwLock<TaskQueueState>>,
    // 幂等性映射: request_id -> task_id
    request_id_map: Arc<RwLock<std::collections::HashMap<String, String>>>,
    // 断点恢复存储
    resume_store: Arc<crate::core::resume::ResumeStore>,
    deferred_tasks: tokio::sync::Mutex<std::collections::HashSet<String>>,
}

impl CoreService {
    pub fn new(task_service: Arc<TaskService>, app_data_dir: &str) -> Self {
        let max_tasks = get_config().max_task_num as usize;
        Self {
            task_service,
            deferred_tasks: tokio::sync::Mutex::new(std::collections::HashSet::new()),
            concurrency_limiter: Arc::new(Semaphore::new(max_tasks)),
            queue_state: Arc::new(RwLock::new(TaskQueueState {
                running_tasks: std::collections::HashMap::new(),
                waiting_tasks: Vec::new(),
            })),
            request_id_map: Arc::new(RwLock::new(std::collections::HashMap::new())),
            resume_store: Arc::new(crate::core::resume::ResumeStore::new(&format!(
                "{}/resumes",
                app_data_dir
            ))),
        }
    }

    pub async fn init(&self) -> Result<()> {
        self.resume_store.init().await?;

        // 应用重启后启动恢复扫描
        let tasks = self.task_service.get_tasks().await;
        for mut task in tasks {
            if let Some(request_id) = task.request_id.clone() {
                self.request_id_map
                    .write()
                    .await
                    .insert(request_id, task.id.clone());
            }
            if matches!(
                task.status,
                TaskStatus::Downloading
                    | TaskStatus::Queued
                    | TaskStatus::Resuming
                    | TaskStatus::Merging
            ) {
                tracing::info!("🔄 Auto-resuming uncompleted task: {}", task.title);
                task.status = TaskStatus::Queued;
                self.queue_state.write().await.waiting_tasks.push(task);
            }
        }
        self.schedule().await;

        Ok(())
    }

    pub async fn get_task_by_request_id(&self, request_id: &str) -> Option<Task> {
        let guard = self.request_id_map.read().await;
        if let Some(task_id) = guard.get(request_id) {
            self.task_service.get_task(task_id).await
        } else {
            None
        }
    }

    pub async fn get_task(&self, task_id: &str) -> Option<Task> {
        self.task_service.get_task(task_id).await
    }

    pub async fn get_tasks(&self) -> Vec<Task> {
        self.task_service.get_tasks().await
    }

    pub async fn runtime_metrics(&self) -> RuntimeMetrics {
        let tasks = self.task_service.get_tasks().await;
        let queue_state = self.queue_state.read().await;
        let running_tasks = queue_state.running_tasks.len();
        let queued_tasks = tasks
            .iter()
            .filter(|task| matches!(task.status, TaskStatus::Queued))
            .count();
        let total_speed_bytes_per_sec = tasks.iter().map(|task| task.speed).sum();
        let config = get_config();

        RuntimeMetrics {
            running_tasks,
            queued_tasks,
            total_speed_bytes_per_sec,
            active_chunk_connections: Some(runtime_metrics_service::active_chunk_connections()),
            max_task_num: config.max_task_num,
            max_chunk_connections: config.max_chunk_connections,
        }
    }

    pub async fn get_task_events_since(
        &self,
        task_id: &str,
        from_event_id: u64,
    ) -> Option<Vec<crate::core::models::TaskEvent>> {
        self.task_service
            .get_events_since(task_id, from_event_id)
            .await
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<crate::core::models::TaskEvent> {
        self.task_service.subscribe_events()
    }

    pub async fn add_task(&self, task: Task, request_id: Option<String>) {
        if let Some(req_id) = request_id {
            let mut map_guard = self.request_id_map.write().await;
            map_guard.insert(req_id, task.id.clone());
        }
        tracing::info!(
            task_id = %task.id,
            request_id = ?task.request_id,
            trace_id = ?task.trace_id,
            pack_id = ?task.pack_id,
            execution_kind = ?task.execution_kind,
            stage_count = task.stages.len(),
            "task accepted into core service"
        );
        let _ = self.enqueue_task(task).await;
    }

    pub async fn enqueue_task(&self, task: Task) -> Result<()> {
        // 1. 保存到 TaskService（持久化）
        self.task_service.add_task(task.clone()).await?;

        // 2. 压入等待队列
        let waiting_len = {
            let mut state = self.queue_state.write().await;
            state.waiting_tasks.push(task.clone());
            state.waiting_tasks.len()
        };
        tracing::info!(
            task_id = %task.id,
            request_id = ?task.request_id,
            trace_id = ?task.trace_id,
            waiting_len,
            "task queued for scheduler"
        );

        // 3. 尝试调度
        self.schedule().await;
        Ok(())
    }

    pub async fn add_deferred_browser_task(&self, task: Task) -> Result<()> {
        self.deferred_tasks.lock().await.insert(task.id.clone());
        self.task_service.add_task(task.clone()).await?;
        if let Some(request_id) = task.request_id {
            self.request_id_map
                .write()
                .await
                .insert(request_id, task.id);
        }
        Ok(())
    }

    pub async fn start_deferred_browser_task(&self, id: &str) -> Result<(), TaskControlError> {
        let task = self
            .task_service
            .get_task(id)
            .await
            .ok_or(TaskControlError::NotFound)?;
        let deferred = self.deferred_tasks.lock().await.remove(id);
        if !deferred
            && task.source.as_deref() == Some("download")
            && matches!(
                task.status,
                TaskStatus::Queued | TaskStatus::Downloading | TaskStatus::Completed
            )
        {
            return Ok(()); // An activation retry must never enqueue a duplicate.
        }
        if task.status != TaskStatus::Queued || !deferred {
            return Err(TaskControlError::InvalidState {
                command: "start",
                status: task.status,
            });
        }
        self.queue_state.write().await.waiting_tasks.push(task);
        self.schedule().await;
        Ok(())
    }

    /// Register an already materialized browser file without scheduling a
    /// second download. Use the normal event/persistence stream for every UI.
    pub async fn add_completed_browser_task(&self, task: Task) -> Result<Task> {
        let id = task.id.clone();
        let path = task.path.clone();
        self.task_service.add_task(task.clone()).await?;
        self.task_service.complete_task(&id, path).await;
        if let Some(request_id) = task.request_id {
            self.request_id_map
                .write()
                .await
                .insert(request_id, id.clone());
        }
        self.task_service
            .get_task(&id)
            .await
            .ok_or_else(|| anyhow::anyhow!("imported task disappeared"))
    }

    pub async fn get_task_snapshot(&self) -> Vec<serde_json::Value> {
        let tasks = self.task_service.get_tasks().await;
        tasks
            .iter()
            .map(|t| {
                serde_json::json!({
                    "taskId": t.id,
                    "title": t.title,
                    "status": t.status,
                    "progress": t.progress,
                    "receivedBytes": t.received_bytes,
                    "fileSize": t.file_size,
                    "speed": t.speed,
                    "createdAt": 0,
                    "canPause": false,
                    "canOpenFile": false,
                    "canOpenFolder": false,
                    "fileExt": "",
                    "packName": t.pack_id
                })
            })
            .collect()
    }

    pub fn subscribe_task_changes(&self) -> broadcast::Receiver<TaskChangeEvent> {
        self.task_service.subscribe_task_changes()
    }

    // Legacy compatibility if needed
    pub async fn update_task_status(
        &self,
        task_id: &str,
        status: TaskStatus,
        progress: f64,
        speed: u64,
        received_bytes: u64,
    ) {
        self.task_service
            .update_task_status(task_id, status, progress, speed, received_bytes)
            .await;
    }

    pub async fn schedule(&self) {
        Self::schedule_with_parts(self.scheduler_parts()).await;
    }

    pub async fn cancel_task(&self, task_id: &str) -> Result<(), TaskControlError> {
        let Some(task) = self.task_service.get_task(task_id).await else {
            return Err(TaskControlError::NotFound);
        };
        if matches!(
            task.status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Canceled
        ) {
            return Err(TaskControlError::InvalidState {
                command: "cancel",
                status: task.status,
            });
        }
        if let Some(token) = self.queue_state.read().await.running_tasks.get(task_id) {
            token.cancel();
        }
        self.queue_state
            .write()
            .await
            .waiting_tasks
            .retain(|t| t.id != task_id);
        self.task_service
            .update_task_status(task_id, TaskStatus::Canceled, 0.0, 0, 0)
            .await;
        Ok(())
    }

    pub async fn delete_task_artifacts(&self, task_id: &str) -> Result<(), TaskControlError> {
        let Some(task) = self.task_service.get_task(task_id).await else {
            return Err(TaskControlError::NotFound);
        };

        if !matches!(
            task.status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Canceled
        ) {
            self.stop_task_for_artifact_cleanup(&task).await?;
        }

        self.wait_until_not_running(task_id, Duration::from_secs(10))
            .await?;

        let cleanup = task_artifact_service::cleanup_task_artifacts(&task, &self.resume_store)
            .await
            .map_err(|error| TaskControlError::CleanupFailed {
                command: "delete",
                message: error.to_string(),
            })?;
        tracing::info!(
            task_id = %task.id,
            request_id = ?task.request_id,
            removed_paths = ?cleanup.removed_paths,
            skipped_paths = ?cleanup.skipped_paths,
            "task artifacts deleted"
        );

        if let Some(request_id) = task.request_id.as_ref() {
            self.request_id_map.write().await.remove(request_id);
        }
        self.task_service.remove_task(task_id).await;
        self.deferred_tasks.lock().await.remove(task_id);
        Ok(())
    }

    pub async fn retry_task(&self, task_id: &str) -> Result<(), TaskControlError> {
        let Some(task) = self.task_service.get_task(task_id).await else {
            return Err(TaskControlError::NotFound);
        };
        // Blob/data URLs may already have been revoked. Never delete the only
        // saved copy in an attempt to re-download an imported browser file.
        if task.resolved_execution_kind() == ExecutionKind::Other("browser_local".into()) {
            return Err(TaskControlError::UnsupportedOperation {
                command: "retry",
                execution_kind: task.resolved_execution_kind(),
            });
        }
        if !matches!(
            task.status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Canceled
        ) {
            return Err(TaskControlError::InvalidState {
                command: "retry",
                status: task.status,
            });
        }

        let cleanup = task_artifact_service::cleanup_task_artifacts(&task, &self.resume_store)
            .await
            .map_err(|error| TaskControlError::CleanupFailed {
                command: "retry",
                message: error.to_string(),
            })?;
        tracing::info!(
            task_id = %task.id,
            request_id = ?task.request_id,
            removed_paths = ?cleanup.removed_paths,
            skipped_paths = ?cleanup.skipped_paths,
            "task artifacts cleaned before retry"
        );

        let Some(task) = self.task_service.reset_task_for_retry(task_id).await else {
            return Err(TaskControlError::NotFound);
        };
        self.queue_state
            .write()
            .await
            .waiting_tasks
            .retain(|t| t.id != task_id);
        self.queue_state.write().await.waiting_tasks.push(task);
        self.schedule().await;
        Ok(())
    }

    pub async fn pause_task(&self, task_id: &str) -> Result<(), TaskControlError> {
        let Some(task) = self.task_service.get_task(task_id).await else {
            return Err(TaskControlError::NotFound);
        };
        if !matches!(
            task.status,
            TaskStatus::Queued
                | TaskStatus::Parsing
                | TaskStatus::Resuming
                | TaskStatus::Downloading
                | TaskStatus::Merging
        ) {
            return Err(TaskControlError::InvalidState {
                command: "pause",
                status: task.status,
            });
        }
        if let Some(token) = self.queue_state.read().await.running_tasks.get(task_id) {
            token.cancel();
        }
        self.queue_state
            .write()
            .await
            .waiting_tasks
            .retain(|t| t.id != task_id);
        self.task_service
            .update_task_status(
                task_id,
                TaskStatus::Paused,
                task.progress,
                task.speed,
                task.received_bytes,
            )
            .await;
        Ok(())
    }

    pub async fn resume_task(&self, task_id: &str) -> Result<(), TaskControlError> {
        let Some(mut task) = self.task_service.get_task(task_id).await else {
            return Err(TaskControlError::NotFound);
        };
        if !matches!(task.status, TaskStatus::Paused) {
            return Err(TaskControlError::InvalidState {
                command: "resume",
                status: task.status,
            });
        }
        let execution_kind = task.resolved_execution_kind();
        if !task_supports_resume(&execution_kind) {
            return Err(TaskControlError::UnsupportedOperation {
                command: "resume",
                execution_kind,
            });
        }
        task.status = TaskStatus::Queued;
        self.task_service
            .update_task_status(
                task_id,
                TaskStatus::Queued,
                task.progress,
                task.speed,
                task.received_bytes,
            )
            .await;
        self.queue_state.write().await.waiting_tasks.push(task);
        self.schedule().await;
        Ok(())
    }

    async fn stop_task_for_artifact_cleanup(&self, task: &Task) -> Result<(), TaskControlError> {
        if let Some(token) = self.queue_state.read().await.running_tasks.get(&task.id) {
            token.cancel();
        }
        self.queue_state
            .write()
            .await
            .waiting_tasks
            .retain(|t| t.id != task.id);
        self.task_service
            .update_task_status(&task.id, TaskStatus::Canceled, 0.0, 0, 0)
            .await;
        Ok(())
    }

    async fn wait_until_not_running(
        &self,
        task_id: &str,
        timeout: Duration,
    ) -> Result<(), TaskControlError> {
        let deadline = Instant::now() + timeout;
        while self
            .queue_state
            .read()
            .await
            .running_tasks
            .contains_key(task_id)
        {
            if Instant::now() >= deadline {
                return Err(TaskControlError::CleanupFailed {
                    command: "delete",
                    message: "task worker did not stop before cleanup".to_string(),
                });
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(())
    }

    fn scheduler_parts(&self) -> SchedulerParts {
        SchedulerParts {
            task_service: self.task_service.clone(),
            concurrency_limiter: self.concurrency_limiter.clone(),
            queue_state: self.queue_state.clone(),
            resume_store: self.resume_store.clone(),
        }
    }

    async fn schedule_with_parts(parts: SchedulerParts) {
        let max_tasks = get_config().max_task_num as usize;
        let mut to_spawn = Vec::new();

        {
            let mut state = parts.queue_state.write().await;
            while state.running_tasks.len() < max_tasks && !state.waiting_tasks.is_empty() {
                let task = state.waiting_tasks.remove(0);
                let token = tokio_util::sync::CancellationToken::new();
                state.running_tasks.insert(task.id.clone(), token.clone());
                to_spawn.push((task, token));
            }
        }

        for (task, token) in to_spawn {
            Self::spawn_runtime_task(task, token, parts.clone());
        }
    }

    fn spawn_runtime_task(
        task: Task,
        token: tokio_util::sync::CancellationToken,
        parts: SchedulerParts,
    ) {
        tokio::spawn(async move {
            let task_id = task.id.clone();
            let _permit = parts
                .concurrency_limiter
                .clone()
                .acquire_owned()
                .await
                .unwrap();
            if token.is_cancelled() {
                parts
                    .queue_state
                    .write()
                    .await
                    .running_tasks
                    .remove(&task_id);
                Self::schedule_with_parts(parts.clone()).await;
                return;
            }

            tracing::info!(
                task_id = %task.id,
                request_id = ?task.request_id,
                trace_id = ?task.trace_id,
                pack_type = ?task.pack_id,
                execution_kind = ?task.execution_kind,
                stage_count = task.stages.len(),
                "task started"
            );
            parts
                .task_service
                .update_task_status(&task.id, TaskStatus::Downloading, task.progress, 0, 0)
                .await;

            let executor = DownloadExecutor::new(parts.resume_store.clone());
            let plan = ExecutionPlan::from_task(task.clone());
            let result = async {
                let output_path = executor
                    .execute(plan, &parts.task_service, token.clone())
                    .await?;
                let mut completed_task = task.clone();
                completed_task.path = output_path.clone();
                task_artifact_service::cleanup_task_temporary_files(&completed_task).await?;
                parts.resume_store.delete_manifest(&task.id).await?;
                Ok::<String, anyhow::Error>(output_path)
            }
            .await;
            match result {
                Ok(output_path) => {
                    tracing::info!(
                        task_id = %task.id,
                        request_id = ?task.request_id,
                        trace_id = ?task.trace_id,
                        pack_type = ?task.pack_id,
                        output_path = %output_path,
                        "task completed"
                    );
                    parts
                        .task_service
                        .complete_task(&task.id, output_path)
                        .await;
                }
                Err(error) => {
                    let structured_failure = error.downcast_ref::<ExecutionFailure>().cloned();
                    let failure_code = structured_failure
                        .as_ref()
                        .map(|failure| failure.code)
                        .unwrap_or("execution_failed");
                    let failure_stage = structured_failure
                        .as_ref()
                        .map(|failure| failure.stage)
                        .unwrap_or("execution");
                    let retryable = structured_failure
                        .as_ref()
                        .map(|failure| failure.retryable)
                        .unwrap_or(false);
                    tracing::error!(
                        task_id = %task.id,
                        request_id = ?task.request_id,
                        trace_id = ?task.trace_id,
                        stage = failure_stage,
                        failure_code,
                        retryable,
                        %error,
                        "task failed"
                    );
                    let current = parts.task_service.get_task(&task.id).await;
                    if let Some(current_task) = current
                        && current_task.status != TaskStatus::Paused
                        && current_task.status != TaskStatus::Canceled
                    {
                        parts
                            .task_service
                            .fail_task(
                                &task.id,
                                ErrorPayload {
                                    code: failure_code.to_string(),
                                    message: error.to_string(),
                                    retryable,
                                    stage: Some(failure_stage.to_string()),
                                },
                            )
                            .await;
                    }
                }
            }

            {
                let mut state = parts.queue_state.write().await;
                state.running_tasks.remove(&task_id);
            }
            Self::schedule_with_parts(parts.clone()).await;
        });
    }
}

fn task_supports_resume(execution_kind: &ExecutionKind) -> bool {
    matches!(
        execution_kind,
        ExecutionKind::HttpFile | ExecutionKind::DashMediaMerge
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::{PackId, Stage, StageType};
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_data_dir(name: &str) -> String {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("gd3-core-service-{name}-{unique}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    fn task_with_kind(id: &str, execution_kind: ExecutionKind, status: TaskStatus) -> Task {
        let pack_id = match execution_kind {
            ExecutionKind::Hls => PackId::Hls,
            ExecutionKind::Youtube => PackId::Youtube,
            ExecutionKind::DashMediaMerge => PackId::BilibiliDash,
            ExecutionKind::HttpFile => PackId::Http,
            ExecutionKind::BitTorrent => PackId::Other("bittorrent".to_string()),
            ExecutionKind::Other(_) => PackId::Other("other".to_string()),
        };
        let mut task = Task::new(
            id.to_string(),
            "video".to_string(),
            "https://example.com/video.mp4".to_string(),
            pack_id,
            0,
            "downloads/video.mp4".to_string(),
        );
        task.status = status;
        task.execution_kind = Some(execution_kind);
        task.add_stage(Stage {
            stage_index: 0,
            stage_type: StageType::File,
            url: "https://example.com/video.mp4".to_string(),
            file_size: 100,
            headers: HashMap::new(),
            block_num: 1,
        });
        task
    }

    async fn service_with_task_status(task: Task, status: TaskStatus) -> CoreService {
        let task_service = Arc::new(TaskService::new(&temp_data_dir(&task.id)));
        let task_id = task.id.clone();
        task_service.add_task(task).await.unwrap();
        match status {
            TaskStatus::Completed => {
                task_service
                    .complete_task(&task_id, "downloads/video.mp4")
                    .await
            }
            TaskStatus::Failed => {
                task_service
                    .fail_task(
                        &task_id,
                        ErrorPayload {
                            code: "execution_failed".to_string(),
                            message: "failed".to_string(),
                            retryable: false,
                            stage: Some("execution".to_string()),
                        },
                    )
                    .await;
            }
            other => {
                let current = task_service.get_task(&task_id).await.unwrap();
                task_service
                    .update_task_status(
                        &task_id,
                        other,
                        current.progress,
                        current.speed,
                        current.received_bytes,
                    )
                    .await;
            }
        }
        let app_data_dir = temp_data_dir(&task_id);
        CoreService::new(task_service, &app_data_dir)
    }

    #[tokio::test]
    async fn control_commands_return_explicit_state_errors() {
        let app_data_dir = temp_data_dir("missing");
        let service = CoreService::new(Arc::new(TaskService::new(&app_data_dir)), &app_data_dir);
        assert_eq!(
            service.cancel_task("missing").await.unwrap_err(),
            TaskControlError::NotFound
        );

        let completed = service_with_task_status(
            task_with_kind("completed", ExecutionKind::HttpFile, TaskStatus::Queued),
            TaskStatus::Completed,
        )
        .await;
        assert_eq!(
            completed.cancel_task("completed").await.unwrap_err(),
            TaskControlError::InvalidState {
                command: "cancel",
                status: TaskStatus::Completed,
            }
        );
    }

    #[tokio::test]
    async fn resume_requires_paused_resumable_execution_kind() {
        let hls = service_with_task_status(
            task_with_kind("hls", ExecutionKind::Hls, TaskStatus::Queued),
            TaskStatus::Paused,
        )
        .await;
        assert_eq!(
            hls.resume_task("hls").await.unwrap_err(),
            TaskControlError::UnsupportedOperation {
                command: "resume",
                execution_kind: ExecutionKind::Hls,
            }
        );

        let http = service_with_task_status(
            task_with_kind("http", ExecutionKind::HttpFile, TaskStatus::Queued),
            TaskStatus::Paused,
        )
        .await;
        http.resume_task("http").await.unwrap();
        let resumed = http.get_task("http").await.unwrap();
        assert_eq!(resumed.status, TaskStatus::Queued);
    }
}
