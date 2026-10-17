use crate::core::interfaces::{CreateTaskRequest, FeaturePack};
use crate::core::models::{ExecutionKind, PackId, StageType, Task};
use crate::services::task_service::TaskService;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;

pub struct HlsDownloader;

impl Default for HlsDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl HlsDownloader {
    pub fn new() -> Self {
        Self
    }

    pub async fn download_with_progress(
        &self,
        task: Task,
        task_service: &TaskService,
        token: tokio_util::sync::CancellationToken,
    ) -> Result<String> {
        let engine = crate::core::engine::downloader::UniversalDownloader::new();
        engine.download_hls(task, task_service, token).await
    }
}

#[async_trait]
impl FeaturePack for HlsDownloader {
    fn pack_id(&self) -> PackId {
        PackId::Hls
    }

    fn priority(&self) -> u32 {
        50
    }

    fn matches(&self, req: &CreateTaskRequest) -> bool {
        let is_twitter = req.url.contains("twimg.com")
            || req.url.contains("twitter.com")
            || req.url.contains("x.com");
        let is_mp4 = req.url.contains(".mp4") && !req.url.contains(".m3u8");
        (is_twitter && !is_mp4) || req.url.ends_with(".m3u8") || req.url.contains(".m3u8?")
    }

    async fn parse(&self, req: &CreateTaskRequest) -> Result<Task> {
        let mut task = Task::new(
            req.request_id.clone(),
            req.title.clone(),
            "gd3+hls://download".to_string(),
            self.pack_id(),
            0,
            "./downloads".to_string(),
        );
        task.execution_kind = Some(ExecutionKind::Hls);
        task.cookie_str = req.decrypted_cookies.clone();
        task.add_stage(crate::core::models::Stage {
            stage_index: 0,
            stage_type: StageType::Video,
            url: req.url.clone(),
            file_size: 10000,
            headers: HashMap::new(),
            block_num: 1,
        });
        Ok(task)
    }
}
