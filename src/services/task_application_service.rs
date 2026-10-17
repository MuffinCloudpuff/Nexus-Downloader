use crate::parsers::policy::{output_category, output_destination};
use crate::parsers::registry::ParserRegistry;
use crate::protocol::errors::{ErrorCode, TaskError};
use crate::protocol::task::{CreateTaskAccepted, CreateTaskCommand, TaskStatus};
use crate::services::core_service::{CoreService, TaskControlError};
use crate::services::settings_service::SettingsService;
use std::sync::Arc;

pub struct TaskApplicationService {
    core_service: Arc<CoreService>,
    parser_registry: Arc<ParserRegistry>,
    settings_service: Arc<SettingsService>,
}

impl TaskApplicationService {
    pub fn new(
        core_service: Arc<CoreService>,
        parser_registry: Arc<ParserRegistry>,
        settings_service: Arc<SettingsService>,
    ) -> Self {
        Self {
            core_service,
            parser_registry,
            settings_service,
        }
    }

    pub async fn create_task(
        &self,
        cmd: CreateTaskCommand,
    ) -> Result<CreateTaskAccepted, TaskError> {
        let req_id = cmd.request_id.clone();
        let trace_id = cmd
            .client_context
            .as_ref()
            .and_then(|context| context.trace_id.clone());
        tracing::info!(
            request_id = %req_id,
            trace_id = ?trace_id,
            source = ?cmd.source,
            title = %cmd.title,
            header_count = cmd.headers.len(),
            resource_count = cmd.resources.len(),
            has_cookies = cmd.decrypted_cookies.is_some(),
            "application create_task started"
        );
        if req_id.trim().is_empty() {
            return Err(TaskError {
                code: ErrorCode::InvalidRequest,
                message: "request_id is required".to_string(),
                retryable: false,
                stage: Some("gateway".to_string()),
            });
        }
        if cmd.url.trim().is_empty() {
            return Err(TaskError {
                code: ErrorCode::InvalidRequest,
                message: "payload.url is required".to_string(),
                retryable: false,
                stage: Some("gateway".to_string()),
            });
        }

        // Idempotency check
        if let Some(task) = self.core_service.get_task_by_request_id(&req_id).await {
            if task.payload_fingerprint.as_deref() != Some(cmd.payload_fingerprint.as_str()) {
                tracing::warn!(
                    request_id = %req_id,
                    existing_task_id = %task.id,
                    "create_task idempotency conflict"
                );
                return Err(TaskError {
                    code: ErrorCode::IdempotencyConflict,
                    message: "request_id already belongs to a different create_task payload"
                        .to_string(),
                    retryable: false,
                    stage: Some("idempotency".to_string()),
                });
            }
            tracing::info!(
                request_id = %req_id,
                trace_id = ?trace_id,
                task_id = %task.id,
                status = ?task.status,
                event_cursor = task.event_cursor,
                "create_task idempotency replay"
            );
            return Ok(CreateTaskAccepted {
                request_id: req_id,
                trace_id,
                task_id: task.id.clone(),
                status: task.status,
                event_cursor: task.event_cursor,
            });
        }

        let url = cmd.url.clone();
        match self.parser_registry.resolve(&cmd.request_id, &cmd).await {
            Ok(plan) => {
                let settings = self.settings_service.get().await;
                let output_category = output_category::classify_output(&cmd, &plan);
                let plan = output_destination::apply_output_destination(
                    &cmd,
                    plan,
                    &output_category,
                    &settings,
                );
                let task_id = plan.task_id.clone();
                tracing::info!(
                    request_id = %cmd.request_id,
                    trace_id = ?trace_id,
                    task_id = %task_id,
                    execution_kind = ?plan.kind,
                    output_category = %output_category.as_str(),
                    output_path = %output_destination::output_path(&plan).display(),
                    stage_count = plan.stages.len(),
                    "create_task resolved execution plan"
                );
                let mut task = plan.into_task(url);
                task.request_id = Some(cmd.request_id.clone());
                task.trace_id = trace_id.clone();
                task.payload_fingerprint = Some(cmd.payload_fingerprint.clone());
                task.source = cmd.source.clone();
                task.cookie_str = cmd.decrypted_cookies.clone();

                self.core_service
                    .add_task(task, Some(cmd.request_id.clone()))
                    .await;

                Ok(CreateTaskAccepted {
                    request_id: cmd.request_id,
                    trace_id,
                    task_id,
                    status: TaskStatus::Queued,
                    event_cursor: 0,
                })
            }
            Err(e) => {
                tracing::error!(
                    request_id = %cmd.request_id,
                    stage = "parsing",
                    error = %e,
                    "create_task parsing failed"
                );
                Err(TaskError {
                    code: ErrorCode::ParseFailed,
                    message: e.to_string(),
                    retryable: false,
                    stage: Some("parsing".to_string()),
                })
            }
        }
    }

    pub async fn pause_task(&self, task_id: &str) -> Result<(), TaskError> {
        self.core_service
            .pause_task(task_id)
            .await
            .map_err(task_control_error)
    }

    pub async fn resume_task(&self, task_id: &str) -> Result<(), TaskError> {
        self.core_service
            .resume_task(task_id)
            .await
            .map_err(task_control_error)
    }

    pub async fn cancel_task(&self, task_id: &str) -> Result<(), TaskError> {
        self.core_service
            .cancel_task(task_id)
            .await
            .map_err(task_control_error)
    }

    pub async fn delete_task_artifacts(&self, task_id: &str) -> Result<(), TaskError> {
        self.core_service
            .delete_task_artifacts(task_id)
            .await
            .map_err(task_control_error)
    }

    pub async fn retry_task(&self, task_id: &str) -> Result<(), TaskError> {
        self.core_service
            .retry_task(task_id)
            .await
            .map_err(task_control_error)
    }
}

fn task_control_error(error: TaskControlError) -> TaskError {
    match error {
        TaskControlError::NotFound => TaskError {
            code: ErrorCode::TaskNotFound,
            message: error.to_string(),
            retryable: false,
            stage: Some("control".to_string()),
        },
        TaskControlError::InvalidState { .. } => TaskError {
            code: ErrorCode::InvalidTaskState,
            message: error.to_string(),
            retryable: false,
            stage: Some("control".to_string()),
        },
        TaskControlError::UnsupportedOperation { .. } => TaskError {
            code: ErrorCode::UnsupportedOperation,
            message: error.to_string(),
            retryable: false,
            stage: Some("control".to_string()),
        },
        TaskControlError::CleanupFailed { .. } => TaskError {
            code: ErrorCode::WriteFailed,
            message: error.to_string(),
            retryable: false,
            stage: Some("artifact_cleanup".to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::protocol::bittorrent::BitTorrentProtocolParser;
    use crate::protocol::task::{CreateTaskHttpReq, CreateTaskPayload};
    use crate::services::settings_service::SettingsService;
    use crate::services::task_service::TaskService;
    use std::collections::HashMap;

    fn temp_data_dir(test_name: &str) -> String {
        let dir = std::env::temp_dir().join(format!(
            "gd3-daemon-{}-{}",
            test_name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    fn command(request_id: &str, title: &str) -> CreateTaskCommand {
        CreateTaskCommand::from_http(
            CreateTaskHttpReq {
                request_id: request_id.to_string(),
                source: Some("test".to_string()),
                title: title.to_string(),
                payload: CreateTaskPayload {
                    url: "magnet:?xt=urn:btih:0123456789abcdef".to_string(),
                    headers: HashMap::new(),
                    resources: vec![],
                    encrypted_cookies: None,
                    cookie_iv: None,
                    filename: None,
                    mime: None,
                    size: None,
                    supports_range: None,
                },
                client_context: None,
            },
            None,
        )
    }

    #[tokio::test]
    async fn create_task_is_idempotent_and_detects_conflicts() {
        let app_data_dir = temp_data_dir("idempotency");
        let task_service = Arc::new(TaskService::new(&app_data_dir));
        let core_service = Arc::new(CoreService::new(task_service, &app_data_dir));
        let mut registry = ParserRegistry::new();
        registry.register_protocol(Box::new(BitTorrentProtocolParser));
        let settings_service = Arc::new(
            SettingsService::load(
                std::env::temp_dir()
                    .join(format!("gd3-service-settings-{}.json", uuidish()))
                    .to_string_lossy()
                    .to_string(),
            )
            .await,
        );
        let service =
            TaskApplicationService::new(core_service, Arc::new(registry), settings_service);

        let first = service
            .create_task(command("req-1", "first"))
            .await
            .unwrap();
        let repeated = service
            .create_task(command("req-1", "first"))
            .await
            .unwrap();
        assert_eq!(first.task_id, repeated.task_id);

        let conflict = service
            .create_task(command("req-1", "different"))
            .await
            .unwrap_err();
        assert_eq!(conflict.code, ErrorCode::IdempotencyConflict);
    }

    fn uuidish() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
