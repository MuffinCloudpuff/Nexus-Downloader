use crate::core::models::{Task, TaskStatus};
use crate::core::resume::ResumeStore;
use crate::services::task_service::TaskService;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tracing::{error, info};

/// 严格定义通信与状态枚举 (遵循通信契约)
#[derive(Debug)]
pub enum BiliDashEvent {
    BiliVideoStageDownloading(u64),
    BiliVideoStageDownloaded,
    BiliAudioStageDownloading(u64),
    BiliAudioStageDownloaded,
    FfmpegMergeStarted,
    FfmpegMergeCompleted,
    FfmpegMergeFailed(String),
}

pub struct DashDownloader;

impl Default for DashDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl DashDownloader {
    pub fn new() -> Self {
        Self
    }

    pub async fn download_and_merge(
        &self,
        task: Task,
        task_service: &TaskService,
        resume_store: Arc<ResumeStore>,
        token: tokio_util::sync::CancellationToken,
    ) -> Result<String> {
        info!("开始执行 Bilibili DASH 下载与混流任务: {}", task.title);

        if task.stages.len() < 2 {
            anyhow::bail!("任务结构异常：未包含视频或音频的独立切片。");
        }

        let video_stage = &task.stages[0];
        let audio_stage = &task.stages[1];

        let output_file = PathBuf::from(&task.path);
        let output_parent = output_file
            .parent()
            .unwrap_or_else(|| Path::new("downloads"));
        tokio::fs::create_dir_all(output_parent).await?;

        let work_dir = Path::new("temp").join(&task.id).join("bili_dash");
        tokio::fs::create_dir_all(&work_dir).await?;

        let video_file = work_dir.join(format!("{}_video.m4s", task.id));
        let audio_file = work_dir.join(format!("{}_audio.m4s", task.id));

        let engine = crate::core::engine::downloader::UniversalDownloader::new();

        // 1. 下载 Video (由于 UniversalDownloader 内置多线程，顺序执行或异步并行均可)
        // 这里为了进度汇报清晰，暂时使用顺序下载（实际上内部已经是 16 线程拉满）
        info!("开始下载视频流...");
        engine
            .download_chunked_with_progress_range(
                &video_stage.url,
                &video_file,
                video_stage.headers.clone(),
                &task.id,
                task_service,
                resume_store.clone(),
                token.clone(),
                0.0,
                0.62,
                None,
            )
            .await
            .context("视频流下载失败")?;
        info!("{:?}", BiliDashEvent::BiliVideoStageDownloaded);

        // 2. 下载 Audio
        info!("开始下载音频流...");
        engine
            .download_chunked_with_progress_range(
                &audio_stage.url,
                &audio_file,
                audio_stage.headers.clone(),
                &task.id,
                task_service,
                resume_store.clone(),
                token.clone(),
                0.62,
                0.92,
                None,
            )
            .await
            .context("音频流下载失败")?;
        info!("{:?}", BiliDashEvent::BiliAudioStageDownloaded);

        // 3. 调用 FFmpeg 混流
        info!("{:?}", BiliDashEvent::FfmpegMergeStarted);
        task_service
            .update_task_status(&task.id, TaskStatus::Merging, 0.92, 0, 0)
            .await;

        let mut child = tokio::process::Command::new("ffmpeg")
            .arg("-y") // 覆盖输出
            .arg("-i")
            .arg(video_file.to_str().unwrap())
            .arg("-i")
            .arg(audio_file.to_str().unwrap())
            .arg("-c:v")
            .arg("copy")
            .arg("-c:a")
            .arg("copy")
            .arg(output_file.to_str().unwrap())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true) // 防孤儿进程
            .spawn()
            .context("未能启动 FFmpeg 进程，请确保已安装 FFmpeg 且在环境变量中。")?;

        let status = child.wait().await?;
        if status.success() {
            info!("{:?}", BiliDashEvent::FfmpegMergeCompleted);
            // 清理缓存
            let _ = tokio::fs::remove_dir_all(work_dir).await;
            Ok(output_file.display().to_string())
        } else {
            let err_msg = format!("FFmpeg 异常退出: {}", status);
            error!("{:?}", BiliDashEvent::FfmpegMergeFailed(err_msg.clone()));
            anyhow::bail!(err_msg)
        }
    }
}
