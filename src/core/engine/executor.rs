use crate::core::engine::downloader::UniversalDownloader;
use crate::core::execution_plan::{ExecutionKind, ExecutionPlan};
use crate::core::models::Task;
use crate::packs::bili_pack::dash_downloader::DashDownloader;
use crate::packs::hls_pack::HlsDownloader;
use crate::packs::youtube_pack::YoutubeDownloader;
use crate::services::task_service::TaskService;
use anyhow::{Context, Result};
use std::path::Path;

use crate::core::resume::ResumeStore;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ExecutionFailure {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    pub stage: &'static str,
}

impl ExecutionFailure {
    fn unsupported_protocol(message: impl Into<String>) -> Self {
        Self {
            code: "unsupported_protocol",
            message: message.into(),
            retryable: false,
            stage: "execution",
        }
    }
}

impl std::fmt::Display for ExecutionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ExecutionFailure {}

pub struct DownloadExecutor {
    resume_store: Arc<ResumeStore>,
}

impl DownloadExecutor {
    pub fn new(resume_store: Arc<ResumeStore>) -> Self {
        Self { resume_store }
    }

    pub async fn execute(
        &self,
        plan: ExecutionPlan,
        task_service: &TaskService,
        token: tokio_util::sync::CancellationToken,
    ) -> Result<String> {
        let original_url = plan
            .stages
            .first()
            .map(|stage| stage.url.clone())
            .unwrap_or_default();
        let task = plan.clone().into_task(original_url);

        match plan.kind {
            ExecutionKind::DashMediaMerge | ExecutionKind::CapturedMediaMerge => {
                let downloader = DashDownloader::new();
                downloader
                    .download_and_merge(task, task_service, self.resume_store.clone(), token)
                    .await
            }
            ExecutionKind::YouTubeExternal => YoutubeDownloader::download_with_progress(
                &task,
                task_service,
                self.resume_store.clone(),
                token,
            )
            .await
            .map_err(|e| anyhow::anyhow!(e)),
            ExecutionKind::Hls => {
                let downloader = HlsDownloader::new();
                downloader
                    .download_with_progress(task, task_service, token)
                    .await
            }
            ExecutionKind::HttpFile => {
                execute_http_file(task, task_service, self.resume_store.clone(), token).await
            }
            ExecutionKind::BitTorrent => Err(ExecutionFailure::unsupported_protocol(
                "BitTorrent execution is not available in this daemon build",
            )
            .into()),
        }
    }
}

async fn execute_http_file(
    task: Task,
    task_service: &TaskService,
    resume_store: Arc<ResumeStore>,
    token: tokio_util::sync::CancellationToken,
) -> Result<String> {
    let first_stage = task
        .stages
        .first()
        .context("HTTP task has no downloadable stage")?;
    let output_file = Path::new(&task.path).to_path_buf();

    tokio::fs::create_dir_all(output_file.parent().unwrap_or_else(|| Path::new("."))).await?;

    let engine = UniversalDownloader::new();
    engine
        .download_chunked(
            &first_stage.url,
            &output_file,
            first_stage.headers.clone(),
            &task.id,
            task_service,
            resume_store,
            token,
        )
        .await?;
    Ok(output_file.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::{ExecutionKind as RuntimeExecutionKind, PackId, Stage, StageType};
    use std::collections::HashMap;

    fn task_with_execution_kind(execution_kind: RuntimeExecutionKind) -> Task {
        let mut task = Task::new(
            "task-1".to_string(),
            "video".to_string(),
            "https://example.com/video.mp4".to_string(),
            PackId::Http,
            0,
            "./downloads".to_string(),
        );
        task.execution_kind = Some(execution_kind);
        task.add_stage(Stage {
            stage_index: 0,
            stage_type: StageType::Video,
            url: "https://example.com/video.mp4".to_string(),
            file_size: 0,
            headers: HashMap::new(),
            block_num: 1,
        });
        task
    }

    #[test]
    fn execution_plan_uses_task_execution_kind() {
        assert_eq!(
            ExecutionPlan::from_task(task_with_execution_kind(
                RuntimeExecutionKind::DashMediaMerge
            ))
            .kind,
            ExecutionKind::DashMediaMerge
        );
        assert_eq!(
            ExecutionPlan::from_task(task_with_execution_kind(RuntimeExecutionKind::Youtube)).kind,
            ExecutionKind::YouTubeExternal
        );
        assert_eq!(
            ExecutionPlan::from_task(task_with_execution_kind(RuntimeExecutionKind::Hls)).kind,
            ExecutionKind::Hls
        );
        assert_eq!(
            ExecutionPlan::from_task(task_with_execution_kind(RuntimeExecutionKind::HttpFile)).kind,
            ExecutionKind::HttpFile
        );
        assert_eq!(
            ExecutionPlan::from_task(task_with_execution_kind(RuntimeExecutionKind::BitTorrent))
                .kind,
            ExecutionKind::BitTorrent
        );
    }

    #[test]
    fn http_output_path_preserves_existing_extension() {
        let task = task_with_execution_kind(RuntimeExecutionKind::HttpFile);
        assert_eq!(Path::new(&task.path), Path::new("./downloads"));
    }

    #[tokio::test]
    async fn bittorrent_execution_returns_structured_unsupported_protocol() {
        let executor = DownloadExecutor::new(Arc::new(ResumeStore::new("data/resumes")));
        let plan =
            ExecutionPlan::from_task(task_with_execution_kind(RuntimeExecutionKind::BitTorrent));
        let task_service = TaskService::new(&std::env::temp_dir().to_string_lossy());
        let error = executor
            .execute(
                plan,
                &task_service,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .unwrap_err();
        let failure = error
            .downcast_ref::<ExecutionFailure>()
            .expect("structured execution failure");
        assert_eq!(failure.code, "unsupported_protocol");
        assert_eq!(failure.stage, "execution");
        assert!(!failure.retryable);
    }
}
