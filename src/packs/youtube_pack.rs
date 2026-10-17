use crate::core::interfaces::{CreateTaskRequest, FeaturePack};
use crate::core::models::{ExecutionKind, PackId, StageType, Task, TaskStatus};
use crate::core::resume::ResumeStore;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq)]
enum StreamProtocol {
    Http,
    Hls,
}

fn stream_protocol(format: &serde_json::Value) -> anyhow::Result<StreamProtocol> {
    let url = format["url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing stream URL"))?;
    let protocol = format["protocol"].as_str().unwrap_or("");
    // A YouTube HLS manifest may have ext=mp4 and no .m3u8 suffix. Its transport
    // protocol, not its eventual container extension, determines the downloader.
    if matches!(protocol, "m3u8" | "m3u8_native")
        || url.contains("/api/manifest/hls_")
        || url
            .split('?')
            .next()
            .is_some_and(|path| path.ends_with(".m3u8"))
    {
        return Ok(StreamProtocol::Hls);
    }
    anyhow::ensure!(
        matches!(protocol, "http" | "https" | ""),
        "Unsupported YouTube stream protocol: {protocol}"
    );
    Ok(StreamProtocol::Http)
}

fn stream_headers(
    format: &serde_json::Value,
    metadata: &serde_json::Value,
) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    for source in [metadata.get("http_headers"), format.get("http_headers")] {
        if let Some(object) = source.and_then(|value| value.as_object()) {
            for (key, value) in object {
                if let Some(value) = value.as_str() {
                    headers.insert(key.clone(), value.to_owned());
                }
            }
        }
    }
    headers
}

pub struct YoutubeDownloader;

impl Default for YoutubeDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl YoutubeDownloader {
    pub fn new() -> Self {
        Self
    }

    fn sanitize_youtube_url(url: &str) -> String {
        if let Some(query_start) = url.find('?') {
            let base = &url[..query_start];
            let query = &url[query_start + 1..];
            let clean_params: Vec<&str> = query
                .split('&')
                .filter(|param| {
                    let key = param.split('=').next().unwrap_or("");
                    key == "v" || key == "t"
                })
                .collect();
            if clean_params.is_empty() {
                base.to_string()
            } else {
                format!("{}?{}", base, clean_params.join("&"))
            }
        } else {
            url.to_string()
        }
    }

    pub async fn ensure_yt_dlp() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        crate::services::yt_dlp_update_service::ensure_yt_dlp().await?;
        Ok(())
    }
}

#[async_trait]
impl FeaturePack for YoutubeDownloader {
    fn pack_id(&self) -> PackId {
        PackId::Youtube
    }

    fn priority(&self) -> u32 {
        80
    }

    fn matches(&self, req: &CreateTaskRequest) -> bool {
        let mut is_youtube = req.url.contains("youtube.com") || req.url.contains("youtu.be");
        if !is_youtube {
            for (k, v) in &req.headers {
                if (k.to_lowercase() == "referer" || k.to_lowercase() == "origin")
                    && (v.contains("youtube.com") || v.contains("youtu.be"))
                {
                    is_youtube = true;
                }
            }
        }
        if !is_youtube {
            for res in &req.resources {
                if let Some(headers) = res.get("headers")
                    && let Some(obj) = headers.as_object()
                {
                    for (k, v) in obj {
                        if (k.to_lowercase() == "referer" || k.to_lowercase() == "origin")
                            && let Some(s) = v.as_str()
                            && (s.contains("youtube.com") || s.contains("youtu.be"))
                        {
                            is_youtube = true;
                        }
                    }
                }
            }
        }
        is_youtube
    }

    async fn parse(&self, req: &CreateTaskRequest) -> anyhow::Result<Task> {
        Self::ensure_yt_dlp()
            .await
            .map_err(|e| anyhow::anyhow!(e))?;

        let mut target_url = req.url.clone();
        if !target_url.contains("youtube.com") && !target_url.contains("youtu.be") {
            for (k, v) in &req.headers {
                if (k.to_lowercase() == "referer" || k.to_lowercase() == "origin")
                    && (v.contains("youtube.com") || v.contains("youtu.be"))
                {
                    target_url = v.clone();
                }
            }
        }

        let clean_url = Self::sanitize_youtube_url(&target_url);
        tracing::info!(
            "🔍 Accepted YouTube URL without pre-extraction: {} (cleaned: {})",
            target_url,
            clean_url
        );
        let title = if req.title.trim().is_empty() {
            "YouTube Video".to_string()
        } else {
            req.title.clone()
        };

        let mut task = Task::new(
            req.request_id.clone(),
            title,
            "gd3+ytdlp://download".to_string(),
            self.pack_id(),
            0,
            "./downloads".to_string(),
        );
        task.execution_kind = Some(ExecutionKind::Youtube);

        task.cookie_str = req.decrypted_cookies.clone();

        task.add_stage(crate::core::models::Stage {
            stage_index: 0,
            stage_type: StageType::Video,
            url: clean_url,
            file_size: 10000,
            headers: std::collections::HashMap::new(),
            block_num: 1,
        });

        Ok(task)
    }
}

impl YoutubeDownloader {
    pub async fn download(task: &Task) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let dummy_service =
            std::sync::Arc::new(crate::services::task_service::TaskService::new("./data"));
        let dummy_resume = std::sync::Arc::new(ResumeStore::new("./data/resumes"));
        let dummy_token = tokio_util::sync::CancellationToken::new();
        Self::download_with_progress(task, &dummy_service, dummy_resume, dummy_token)
            .await
            .map(|_| ())
    }

    pub async fn download_with_progress(
        task: &Task,
        task_service: &crate::services::task_service::TaskService,
        resume_store: Arc<ResumeStore>,
        token: tokio_util::sync::CancellationToken,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let raw_url = &task
            .stages
            .first()
            .ok_or("YouTube task has no source URL")?
            .url;
        let url = Self::sanitize_youtube_url(raw_url);
        tracing::info!("🚀 Extracting real URLs via yt-dlp for: {}", url);

        task_service
            .update_task_status(&task.id, TaskStatus::Parsing, 0.0, 0, 0)
            .await;

        let executable = tokio::select! {
            _ = token.cancelled() => return Err("任务已取消".into()),
            executable = crate::services::yt_dlp_update_service::ensure_yt_dlp() => executable?,
        };
        tracing::info!(path = %executable.display(), "using project-managed yt-dlp");
        let mut cmd = crate::services::yt_dlp_update_service::hidden_command(&executable);
        cmd.arg("--ignore-config")
            .arg("--dump-json")
            .arg("--no-playlist")
            .arg(&url);

        // A portable Deno next to the daemon is not necessarily on PATH. Official
        // yt-dlp needs it to solve YouTube's JS challenges; otherwise it uses PATH.
        if let Some(deno) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("deno.exe")))
            && deno.is_file()
        {
            cmd.arg("--js-runtimes")
                .arg(format!("deno:{}", deno.display()));
        }

        let output = tokio::select! {
            _ = token.cancelled() => return Err("任务已取消".into()),
            output = cmd.output() => output?,
        };
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("yt-dlp extraction failed: {}", err).into());
        }
        if !output.stderr.is_empty() {
            tracing::warn!(message = %String::from_utf8_lossy(&output.stderr), "yt-dlp extraction warnings");
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        let parsed: serde_json::Value = serde_json::from_str(&json_str)?;

        let engine = crate::core::engine::downloader::UniversalDownloader::new();

        let final_output = Path::new(&task.path).to_path_buf();
        let base_dir = crate::services::task_artifact_service::task_work_dir(task)?;
        tokio::fs::create_dir_all(&base_dir).await?;

        let safe_title = crate::utils::to_safe_filename(&task.title, "Y-youtube_video");

        let mut downloaded_files = Vec::new();

        let formats: Vec<&serde_json::Value> = if let Some(formats) = parsed
            .get("requested_formats")
            .and_then(|value| value.as_array())
            .filter(|formats| !formats.is_empty())
        {
            formats.iter().collect()
        } else {
            vec![&parsed]
        };
        for (index, format) in formats.into_iter().enumerate() {
            let protocol = stream_protocol(format)?;
            let stream_url = format["url"].as_str().ok_or("Missing stream URL")?;
            let ext = format["ext"].as_str().unwrap_or("mp4");
            let headers = stream_headers(format, &parsed);
            let temp_file = base_dir.join(format!("{safe_title}_{index}.{ext}"));
            tracing::info!(task_id = %task.id, stream_index = index, ?protocol,
                format_id = ?format["format_id"].as_str(), "YouTube stream download started");
            match protocol {
                StreamProtocol::Http => {
                    engine
                        .download_chunked(
                            stream_url,
                            &temp_file,
                            headers,
                            &task.id,
                            task_service,
                            resume_store.clone(),
                            token.clone(),
                        )
                        .await?
                }
                StreamProtocol::Hls => {
                    let mut stream_task = task.clone();
                    stream_task.path = temp_file.to_string_lossy().into_owned();
                    stream_task.stages = vec![crate::core::models::Stage {
                        stage_index: 0,
                        stage_type: StageType::Video,
                        url: stream_url.to_owned(),
                        file_size: 0,
                        headers,
                        block_num: 1,
                    }];
                    engine
                        .download_hls(stream_task, task_service, token.clone())
                        .await?;
                }
            }
            downloaded_files.push(temp_file);
        }

        task_service
            .update_task_status(&task.id, TaskStatus::Merging, 1.0, 0, 0)
            .await;
        if !downloaded_files.is_empty() {
            // Remux even a single stream so the output container matches the
            // destination extension (a WebM stream must not just be renamed MP4).
            let mut merge_cmd =
                crate::services::yt_dlp_update_service::hidden_command(Path::new("ffmpeg"));
            merge_cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"]);
            for file in &downloaded_files {
                merge_cmd.arg("-i").arg(file.to_str().unwrap());
            }
            merge_cmd
                .arg("-c:v")
                .arg("copy")
                .arg("-c:a")
                .arg("copy")
                .arg(final_output.to_str().unwrap())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());

            let output = tokio::select! {
                _ = token.cancelled() => return Err("任务已取消".into()),
                output = merge_cmd.output() => output?,
            };
            if !output.status.success() {
                let error = String::from_utf8_lossy(&output.stderr);
                return Err(format!("FFmpeg merge failed: {}", error.trim()).into());
            }

            crate::services::task_artifact_service::cleanup_task_temporary_files(task).await?;
        }

        tracing::info!("✅ YouTube download completed entirely in Rust Engine!");
        Ok(final_output.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mp4_container_does_not_make_an_hls_manifest_a_direct_file() {
        let format = json!({"url": "https://manifest.googlevideo.com/api/manifest/hls_playlist/id/video", "ext": "mp4", "protocol": "m3u8_native"});
        assert_eq!(stream_protocol(&format).unwrap(), StreamProtocol::Hls);
        let direct = json!({"url": "https://cdn.googlevideo.com/videoplayback?id=video", "ext": "mp4", "protocol": "https"});
        assert_eq!(stream_protocol(&direct).unwrap(), StreamProtocol::Http);
    }

    #[test]
    fn manifest_urls_are_not_saved_as_media_when_protocol_is_missing() {
        let format = json!({"url": "https://manifest.googlevideo.com/api/manifest/hls_playlist/id/video", "ext": "mp4"});
        assert_eq!(stream_protocol(&format).unwrap(), StreamProtocol::Hls);
        let format = json!({"url": "https://example.test/playlist.m3u8?signature=example", "protocol": "https"});
        assert_eq!(stream_protocol(&format).unwrap(), StreamProtocol::Hls);
        assert!(stream_protocol(&json!({"url": "https://example.test/manifest.mpd", "protocol": "http_dash_segments"})).is_err());
    }

    #[test]
    fn format_headers_override_defaults_without_losing_referer() {
        let metadata = json!({"http_headers": {"Referer": "https://www.youtube.com/", "User-Agent": "default"}});
        let format = json!({"http_headers": {"User-Agent": "stream-specific"}});
        let headers = stream_headers(&format, &metadata);
        assert_eq!(headers["Referer"], "https://www.youtube.com/");
        assert_eq!(headers["User-Agent"], "stream-specific");
    }
}
