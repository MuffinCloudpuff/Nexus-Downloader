use crate::core::engine::range_segments::{
    DEFAULT_MIN_SPLIT_BYTES, SegmentFetchStatus, SegmentJob, SegmentQueue, chunk_for_split,
};
use crate::core::engine::range_tuner::{self, RangeTunerHistory, RangeTunerRuntime};
use crate::core::models::{Task, TaskStatus};
use crate::parsers::policy::{quality, retry};
use crate::services::config::{MAX_CHUNK_CONNECTIONS, MIN_CHUNK_CONNECTIONS, get_config};
use crate::services::runtime_metrics_service::ActiveChunkConnectionGuard;
use crate::services::site_performance_store::{site_key_from_url, site_performance_store};
use anyhow::{Context, Result};
use reqwest::{Client, StatusCode, header};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Notify, Semaphore};
use tracing::info;

const RESUME_PROGRESS_SAVE_INTERVAL_BYTES: u64 = 8 * 1024 * 1024;
const TARGET_RANGE_SEGMENT_BYTES: u64 = 20 * 1024 * 1024;
const MIN_RANGE_SEGMENT_BYTES: u64 = 8 * 1024 * 1024;
const RANGE_LEASE_BYTES: u64 = 16 * 1024 * 1024;
const SMALL_FILE_SINGLE_RANGE_BYTES: u64 = 20 * 1024 * 1024;
const GOOGLEVIDEO_TARGET_RANGE_SEGMENT_BYTES: u64 = 4 * 1024 * 1024;
const GOOGLEVIDEO_MIN_RANGE_SEGMENT_BYTES: u64 = 1024 * 1024;
const GOOGLEVIDEO_RANGE_LEASE_BYTES: u64 = 2 * 1024 * 1024;
const SEGMENTS_PER_ACTIVE_CONNECTION: usize = 4;
const SITE_HISTORY_MIN_BYTES: u64 = 64 * 1024 * 1024;
const SITE_HISTORY_MIN_SECONDS: f64 = 10.0;

pub struct UniversalDownloader {
    client: Client,
}

struct HlsTrack<'a> {
    url: &'a str,
    content: &'a str,
    directory: PathBuf,
    progress_range: (f64, f64),
    received_before: u64,
}

// AUDIO names a rendition group, not necessarily an audio stream inside the
// selected video playlist. Never discard that relationship when selecting quality.
fn hls_audio_rendition(
    master: &m3u8_rs::MasterPlaylist,
    variant: &m3u8_rs::VariantStream,
    base: &reqwest::Url,
) -> Result<(Option<String>, bool)> {
    if let Some(group) = &variant.audio {
        let rendition = master
            .alternatives
            .iter()
            .filter(|media| {
                media.media_type == m3u8_rs::AlternativeMediaType::Audio && media.group_id == *group
            })
            .max_by_key(|media| (media.default, media.autoselect))
            .with_context(|| format!("HLS audio group {group} is missing"))?;
        // No URI means this rendition is muxed into the video playlist.
        let audio_url = rendition
            .uri
            .as_ref()
            .map(|uri| base.join(uri))
            .transpose()?;
        return Ok((audio_url.map(|url| url.to_string()), true));
    }
    let requires_audio = variant.codecs.as_deref().is_some_and(|codecs| {
        codecs.split(',').any(|codec| {
            let codec = codec.trim().to_ascii_lowercase();
            [
                "mp4a", "aac", "ac-3", "ec-3", "opus", "vorbis", "flac", "alac",
            ]
            .iter()
            .any(|prefix| codec.starts_with(prefix))
        })
    });
    Ok((None, requires_audio))
}

fn map_progress(progress_start: f64, progress_end: f64, local_progress: f64) -> f64 {
    let start = progress_start.clamp(0.0, 1.0);
    let end = progress_end.clamp(start, 1.0);
    let local = local_progress.clamp(0.0, 1.0);
    start + (end - start) * local
}

fn request_headers(headers_map: &HashMap<String, String>) -> header::HeaderMap {
    let mut req_headers = header::HeaderMap::new();
    for (key, value) in headers_map {
        if let Ok(name) = header::HeaderName::from_bytes(key.as_bytes())
            && let Ok(value) = header::HeaderValue::from_str(value)
        {
            req_headers.insert(name, value);
        }
    }
    req_headers
}

fn url_host_for_log(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_else(|| "invalid".to_string())
}

fn short_log_text(value: &[u8]) -> String {
    let text = String::from_utf8_lossy(value).replace(['\r', '\n'], " ");
    text.chars().take(400).collect()
}

fn insert_raw_range_headers(headers: &mut header::HeaderMap, start: u64, end: u64) -> Result<()> {
    headers.insert(
        header::RANGE,
        header::HeaderValue::from_str(&format!("bytes={start}-{end}"))?,
    );
    headers.insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("identity"),
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeSegmentPolicyKind {
    Generic,
    GoogleVideo,
}

#[derive(Debug, Clone, Copy)]
struct RangeSegmentPolicy {
    kind: RangeSegmentPolicyKind,
    target_segment_bytes: u64,
    min_segment_bytes: u64,
    lease_bytes: u64,
}

impl RangeSegmentPolicy {
    fn generic() -> Self {
        Self {
            kind: RangeSegmentPolicyKind::Generic,
            target_segment_bytes: TARGET_RANGE_SEGMENT_BYTES,
            min_segment_bytes: MIN_RANGE_SEGMENT_BYTES,
            lease_bytes: RANGE_LEASE_BYTES,
        }
    }

    fn googlevideo() -> Self {
        Self {
            kind: RangeSegmentPolicyKind::GoogleVideo,
            target_segment_bytes: GOOGLEVIDEO_TARGET_RANGE_SEGMENT_BYTES,
            min_segment_bytes: GOOGLEVIDEO_MIN_RANGE_SEGMENT_BYTES,
            lease_bytes: GOOGLEVIDEO_RANGE_LEASE_BYTES,
        }
    }
}

fn range_segment_policy_for_url(url: &str) -> RangeSegmentPolicy {
    let host = url_host_for_log(url);
    if host == "googlevideo.com" || host.ends_with(".googlevideo.com") {
        RangeSegmentPolicy::googlevideo()
    } else {
        RangeSegmentPolicy::generic()
    }
}

fn plan_range_segment_count_with_policy(
    total_size: u64,
    max_active_connections: usize,
    policy: RangeSegmentPolicy,
) -> usize {
    if total_size <= SMALL_FILE_SINGLE_RANGE_BYTES {
        return 1;
    }

    let target_segments = std::cmp::max(1, total_size.div_ceil(policy.target_segment_bytes));
    let max_segments_by_connections =
        std::cmp::max(1, max_active_connections * SEGMENTS_PER_ACTIVE_CONNECTION) as u64;
    let max_segments_by_min_size = std::cmp::max(1, total_size.div_ceil(policy.min_segment_bytes));
    target_segments
        .min(max_segments_by_connections)
        .min(max_segments_by_min_size)
        .max(1) as usize
}

#[allow(dead_code)]
fn plan_range_segment_count(total_size: u64, max_active_connections: usize) -> usize {
    plan_range_segment_count_with_policy(
        total_size,
        max_active_connections,
        RangeSegmentPolicy::generic(),
    )
}

fn should_update_site_history(received: u64, elapsed_secs: f64, speed: u64) -> bool {
    received >= SITE_HISTORY_MIN_BYTES && elapsed_secs >= SITE_HISTORY_MIN_SECONDS && speed > 0
}

fn session_speed_bytes_per_sec(
    total_received: u64,
    initial_received: u64,
    elapsed_secs: f64,
) -> u64 {
    if elapsed_secs <= 0.0 {
        return 0;
    }
    (total_received.saturating_sub(initial_received) as f64 / elapsed_secs) as u64
}

fn instant_speed_bytes_per_sec(
    current_received: u64,
    previous_received: u64,
    elapsed_secs: f64,
) -> u64 {
    if elapsed_secs <= 0.0 {
        return 0;
    }
    (current_received.saturating_sub(previous_received) as f64 / elapsed_secs) as u64
}

fn build_direct_write_chunks(
    output_path: &Path,
    total_size: u64,
    num_chunks: usize,
) -> Vec<crate::core::resume::ChunkManifest> {
    let chunk_size = total_size / num_chunks as u64;
    let output_path = output_path.to_string_lossy().into_owned();

    (0..num_chunks)
        .map(|i| {
            let start = i as u64 * chunk_size;
            let end = if i == num_chunks - 1 {
                total_size - 1
            } else {
                (i as u64 + 1) * chunk_size - 1
            };
            crate::core::resume::ChunkManifest {
                index: i as u32,
                start,
                end,
                path: output_path.clone(),
                status: crate::core::resume::ChunkStatus::Pending,
                bytes_written: 0,
                checksum: None,
            }
        })
        .collect()
}

async fn flush_std_file(file: Arc<std::fs::File>) -> Result<()> {
    tokio::task::spawn_blocking(move || file.sync_data())
        .await
        .context("output file flush task panicked")??;
    Ok(())
}

#[cfg(windows)]
fn mark_sparse_best_effort(path: &Path) {
    match Command::new("fsutil")
        .arg("sparse")
        .arg("setflag")
        .arg(path)
        .status()
    {
        Ok(status) if status.success() => {
            tracing::info!(output_path = %path.display(), "marked output file as sparse");
        }
        Ok(status) => {
            tracing::warn!(
                output_path = %path.display(),
                status = %status,
                "failed to mark output file as sparse"
            );
        }
        Err(error) => {
            tracing::warn!(
                output_path = %path.display(),
                %error,
                "failed to run fsutil sparse setflag"
            );
        }
    }
}

#[cfg(not(windows))]
fn mark_sparse_best_effort(_path: &Path) {}

async fn prepare_sparse_direct_output_file(
    output_path: &Path,
    total_size: u64,
) -> Result<Arc<std::fs::File>> {
    let output_path = output_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<Arc<std::fs::File>> {
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(&output_path)?;
        mark_sparse_best_effort(&output_path);
        file.set_len(total_size)?;
        Ok(Arc::new(file))
    })
    .await
    .context("output file preparation task panicked")?
}

fn stage_uses_direct_output(
    stage: &crate::core::resume::StageManifest,
    output_path: &Path,
) -> bool {
    let output_path = output_path.to_string_lossy();
    stage.chunks.iter().all(|chunk| {
        chunk.path == output_path && chunk.bytes_written <= chunk.end - chunk.start + 1
    })
}

fn stage_covers_total_range(stage: &crate::core::resume::StageManifest) -> bool {
    if stage.total_size == 0 || stage.chunks.is_empty() {
        return false;
    }

    let mut ranges: Vec<(u64, u64)> = stage
        .chunks
        .iter()
        .map(|chunk| (chunk.start, chunk.end))
        .collect();
    ranges.sort_by_key(|(start, _)| *start);

    let mut expected_start = 0_u64;
    for (start, end) in ranges {
        if start != expected_start || end < start {
            return false;
        }
        expected_start = match end.checked_add(1) {
            Some(next) => next,
            None => return false,
        };
    }

    expected_start == stage.total_size
}

async fn output_file_has_expected_len(output_path: &Path, total_size: u64) -> bool {
    tokio::fs::metadata(output_path)
        .await
        .map(|metadata| metadata.len() == total_size)
        .unwrap_or(false)
}

async fn save_chunk_progress(
    manifest: &Arc<tokio::sync::Mutex<crate::core::resume::ResumeManifest>>,
    resume_store: &Arc<crate::core::resume::ResumeStore>,
    url: &str,
    chunk_index: usize,
    status: crate::core::resume::ChunkStatus,
    bytes_written: u64,
) {
    let mut manifest = manifest.lock().await;
    if let Some(stage) = manifest.stages.get_mut(url)
        && let Some(chunk) = stage.chunks.get_mut(chunk_index)
    {
        chunk.status = status;
        chunk.bytes_written = bytes_written;
    }
    let _ = resume_store.save_manifest(&manifest).await;
}

async fn append_manifest_split_chunk(
    manifest: &Arc<tokio::sync::Mutex<crate::core::resume::ResumeManifest>>,
    resume_store: &Arc<crate::core::resume::ResumeStore>,
    url: &str,
    output_path: &str,
    original_index: usize,
    job: &SegmentJob,
) {
    let mut manifest = manifest.lock().await;
    if let Some(stage) = manifest.stages.get_mut(url) {
        if let Some(original) = stage.chunks.get_mut(original_index) {
            original.end = job.start.saturating_sub(1);
            original.status = crate::core::resume::ChunkStatus::Pending;
        }
        stage.chunks.push(chunk_for_split(output_path, job));
    }
    let _ = resume_store.save_manifest(&manifest).await;
}

async fn fetch_range_segment(
    client: Client,
    url: &str,
    base_headers: header::HeaderMap,
    output_file: Arc<std::fs::File>,
    downloaded_bytes: Arc<AtomicU64>,
    manifest: Arc<tokio::sync::Mutex<crate::core::resume::ResumeManifest>>,
    resume_store: Arc<crate::core::resume::ResumeStore>,
    token: tokio_util::sync::CancellationToken,
    job: &SegmentJob,
    lease_bytes: u64,
) -> Result<(SegmentFetchStatus, u64)> {
    let expected_size = job.expected_size();
    let mut next_offset = job.next_offset();
    let mut local_downloaded = job.completed_bytes();
    let mut last_persisted = local_downloaded;
    let lease_end = job.lease_end(lease_bytes);
    let lease_size = lease_end.saturating_sub(next_offset).saturating_add(1);

    save_chunk_progress(
        &manifest,
        &resume_store,
        url,
        job.index,
        crate::core::resume::ChunkStatus::Downloading,
        local_downloaded,
    )
    .await;

    let mut chunk_headers = base_headers;
    insert_raw_range_headers(&mut chunk_headers, next_offset, lease_end)?;

    let mut buffered_writer = crate::core::engine::buffered_writer::AsyncBufferedWriter::new(
        output_file.clone(),
        1024 * 1024,
        next_offset,
    );

    let mut res = match client.get(url).headers(chunk_headers).send().await {
        Ok(res) => res,
        Err(error) => {
            tracing::warn!(
                chunk_index = job.index,
                range_start = job.start,
                range_end = job.end,
                next_offset,
                lease_end,
                attempts = job.attempts,
                %error,
                "http range segment recoverable request failure"
            );
            return Ok((SegmentFetchStatus::Recoverable, local_downloaded));
        }
    };

    if res.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        let status = res.status();
        tracing::error!(
            chunk_index = job.index,
            range_start = job.start,
            range_end = job.end,
            next_offset,
            %status,
            "http range worker fatal non-partial response"
        );
        anyhow::bail!("Range segment {} returned HTTP {}", job.index, status);
    }

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                let _ = buffered_writer.flush().await;
                save_chunk_progress(
                    &manifest,
                    &resume_store,
                    url,
                    job.index,
                    crate::core::resume::ChunkStatus::Pending,
                    local_downloaded,
                )
                .await;
                anyhow::bail!("任务已取消");
            }
            chunk_res = res.chunk() => {
                match chunk_res {
                    Ok(Some(chunk)) => {
                        let remaining_lease = lease_end.saturating_sub(next_offset).saturating_add(1);
                        let bytes = if chunk.len() as u64 > remaining_lease {
                            &chunk[..remaining_lease as usize]
                        } else {
                            &chunk
                        };
                        if bytes.is_empty() {
                            continue;
                        }

                        buffered_writer.write(bytes).await?;
                        let written = bytes.len() as u64;
                        local_downloaded += written;
                        next_offset += written;
                        downloaded_bytes.fetch_add(written, Ordering::Relaxed);

                        if local_downloaded - last_persisted >= RESUME_PROGRESS_SAVE_INTERVAL_BYTES {
                            buffered_writer.flush().await?;
                            save_chunk_progress(
                                &manifest,
                                &resume_store,
                                url,
                                job.index,
                                crate::core::resume::ChunkStatus::Downloading,
                                local_downloaded,
                            )
                            .await;
                            last_persisted = local_downloaded;
                        }

                        if local_downloaded >= expected_size {
                            buffered_writer.flush().await?;
                            save_chunk_progress(
                                &manifest,
                                &resume_store,
                                url,
                                job.index,
                                crate::core::resume::ChunkStatus::Completed,
                                expected_size,
                            )
                            .await;
                            return Ok((SegmentFetchStatus::Completed, expected_size));
                        }

                        if next_offset > lease_end {
                            buffered_writer.flush().await?;
                            save_chunk_progress(
                                &manifest,
                                &resume_store,
                                url,
                                job.index,
                                crate::core::resume::ChunkStatus::Pending,
                                local_downloaded,
                            )
                            .await;
                            tracing::debug!(
                                chunk_index = job.index,
                                range_start = job.start,
                                range_end = job.end,
                                lease_end,
                                lease_size,
                                local_downloaded,
                                remaining_bytes = expected_size.saturating_sub(local_downloaded),
                                "http range segment lease completed"
                            );
                            return Ok((SegmentFetchStatus::LeaseCompleted, local_downloaded));
                        }
                    }
                    Ok(None) => {
                        tracing::warn!(
                            chunk_index = job.index,
                            range_start = job.start,
                            range_end = job.end,
                            next_offset,
                            lease_end,
                            expected_size,
                            local_downloaded,
                            attempts = job.attempts,
                            "http range segment recoverable short body"
                        );
                        buffered_writer.flush().await?;
                        save_chunk_progress(
                            &manifest,
                            &resume_store,
                            url,
                            job.index,
                            crate::core::resume::ChunkStatus::Pending,
                            local_downloaded,
                        )
                        .await;
                        return Ok((SegmentFetchStatus::Recoverable, local_downloaded));
                    }
                    Err(error) => {
                        tracing::warn!(
                            chunk_index = job.index,
                            range_start = job.start,
                            range_end = job.end,
                            next_offset,
                            lease_end,
                            expected_size,
                            local_downloaded,
                            attempts = job.attempts,
                            %error,
                            "http range segment recoverable read failure"
                        );
                        buffered_writer.flush().await?;
                        save_chunk_progress(
                            &manifest,
                            &resume_store,
                            url,
                            job.index,
                            crate::core::resume::ChunkStatus::Pending,
                            local_downloaded,
                        )
                        .await;
                        return Ok((SegmentFetchStatus::Recoverable, local_downloaded));
                    }
                }
            }
        }
    }
}

impl Default for UniversalDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl UniversalDownloader {
    pub fn new() -> Self {
        let client = Client::builder()
            .http1_only()
            .no_gzip()
            .no_brotli()
            .no_zstd()
            .no_deflate()
            .build()
            .unwrap_or_else(|error| {
                tracing::warn!(
                    %error,
                    "failed to build raw-byte HTTP client; using default client"
                );
                Client::new()
            });
        Self { client }
    }

    /// 执行大文件多线程 Range 分块下载 (支持断点续传)
    pub async fn download_chunked(
        &self,
        url: &str,
        output_path: &Path,
        headers_map: HashMap<String, String>,
        task_id: &str,
        task_service: &crate::services::task_service::TaskService,
        resume_store: Arc<crate::core::resume::ResumeStore>,
        token: tokio_util::sync::CancellationToken,
    ) -> Result<()> {
        self.download_chunked_with_progress_range(
            url,
            output_path,
            headers_map,
            task_id,
            task_service,
            resume_store,
            token,
            0.0,
            1.0,
            None,
        )
        .await
    }

    /// 执行分块下载，并把本次下载进度映射到任务整体进度区间。
    pub async fn download_chunked_with_progress_range(
        &self,
        url: &str,
        output_path: &Path,
        headers_map: HashMap<String, String>,
        task_id: &str,
        task_service: &crate::services::task_service::TaskService,
        resume_store: Arc<crate::core::resume::ResumeStore>,
        token: tokio_util::sync::CancellationToken,
        progress_start: f64,
        progress_end: f64,
        final_status: Option<TaskStatus>,
    ) -> Result<()> {
        self.download_chunked_impl(
            url,
            output_path,
            headers_map,
            task_id,
            Some(task_service),
            resume_store,
            token,
            progress_start,
            progress_end,
            final_status,
            None,
        )
        .await
    }

    /// Internal tool updates use the same range/resume engine without adding a
    /// user task or emitting task-list events to the desktop/browser clients.
    pub async fn download_internal_file(
        &self,
        url: &str,
        output_path: &Path,
        headers: HashMap<String, String>,
        max_size: u64,
        token: tokio_util::sync::CancellationToken,
    ) -> Result<()> {
        let _cancel_on_drop = token.clone().drop_guard();
        let directory = output_path
            .parent()
            .context("internal download has no directory")?;
        let resumes = directory.join("resumes");
        let resume_store = Arc::new(crate::core::resume::ResumeStore::new(
            &resumes.to_string_lossy(),
        ));
        resume_store.init().await?;
        self.download_chunked_impl(
            url,
            output_path,
            headers,
            "tool-update",
            None,
            resume_store.clone(),
            token,
            0.0,
            1.0,
            None,
            Some(max_size),
        )
        .await?;
        resume_store.delete_manifest("tool-update").await?;
        Ok(())
    }

    async fn download_chunked_impl(
        &self,
        url: &str,
        output_path: &Path,
        headers_map: HashMap<String, String>,
        task_id: &str,
        task_service: Option<&crate::services::task_service::TaskService>,
        resume_store: Arc<crate::core::resume::ResumeStore>,
        token: tokio_util::sync::CancellationToken,
        progress_start: f64,
        progress_end: f64,
        final_status: Option<TaskStatus>,
        max_size: Option<u64>,
    ) -> Result<()> {
        let req_headers = request_headers(&headers_map);

        let mut probe_headers = req_headers.clone();
        insert_raw_range_headers(&mut probe_headers, 0, 0)?;
        let probe_res = self.client.get(url).headers(probe_headers).send().await?;
        let probe_status = probe_res.status();
        if probe_status != StatusCode::PARTIAL_CONTENT {
            tracing::warn!(
                task_id,
                url_host = %url_host_for_log(url),
                status = %probe_status.as_u16(),
                "http range probe rejected before chunk planning"
            );
            anyhow::bail!(
                "Range probe returned HTTP {}; cannot start chunked download",
                probe_status
            );
        }
        let total_size = probe_res
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|val| val.to_str().ok())
            .and_then(|val| val.rsplit_once('/'))
            .and_then(|(_, total)| total.parse::<u64>().ok())
            .or_else(|| {
                probe_res
                    .headers()
                    .get(header::CONTENT_LENGTH)
                    .and_then(|val| val.to_str().ok())
                    .and_then(|val| val.parse::<u64>().ok())
            })
            .context("无法获取 Content-Length，无法进行分块下载")?;
        drop(probe_res);

        anyhow::ensure!(
            total_size > 0 && max_size.is_none_or(|limit| total_size <= limit),
            "download size exceeds allowed limit or is empty"
        );

        info!("📦 Target file size: {} bytes", total_size);

        let max_connections = get_config()
            .max_chunk_connections
            .clamp(MIN_CHUNK_CONNECTIONS, MAX_CHUNK_CONNECTIONS)
            as usize;
        let range_segment_policy = range_segment_policy_for_url(url);
        let mut num_chunks =
            plan_range_segment_count_with_policy(total_size, max_connections, range_segment_policy);
        let max_active_connections = max_connections.min(num_chunks);
        let site_key = site_key_from_url(url);
        let site_history = match site_performance_store() {
            Some(store) => store.get(&site_key).await,
            None => None,
        };
        let tuner_history = site_history.as_ref().map(|record| RangeTunerHistory {
            last_effective_target: record.last_effective_target,
            last_effective_speed_bytes_per_sec: record.last_effective_speed_bytes_per_sec,
        });
        let initial_active_connections =
            range_tuner::initial_target(max_active_connections, tuner_history);
        let chunk_size = total_size / num_chunks as u64;
        info!(
            task_id,
            url_host = %url_host_for_log(url),
            site_key = %site_key,
            total_size,
            planned_range_segments = num_chunks,
            range_segment_policy = ?range_segment_policy.kind,
            target_segment_bytes = range_segment_policy.target_segment_bytes,
            min_segment_bytes = range_segment_policy.min_segment_bytes,
            lease_bytes = range_segment_policy.lease_bytes,
            initial_active_connections,
            max_active_connections,
            chunk_size,
            max_connections,
            history_target = ?site_history.as_ref().map(|record| record.last_effective_target),
            history_speed = ?site_history.as_ref().map(|record| record.last_effective_speed_bytes_per_sec),
            "http range chunk plan created"
        );

        let headers_hash = crate::core::resume::headers_fingerprint(&headers_map);
        let mut manifest = crate::core::resume::ResumeManifest::new(
            task_id.to_string(),
            output_path.to_string_lossy().into_owned(),
        );
        let mut should_create_stage = true;

        let output_len_is_valid = output_file_has_expected_len(output_path, total_size).await;

        if let Ok(Some(mut existing)) = resume_store.load_manifest(task_id).await {
            let stage_is_valid = existing
                .stages
                .get(url)
                .map(|stage| {
                    stage.matches_source(url, total_size, &headers_hash)
                        && stage_covers_total_range(stage)
                        && stage_uses_direct_output(stage, output_path)
                        && output_len_is_valid
                })
                .unwrap_or(false);

            if stage_is_valid {
                info!(
                    "📦 Resuming from direct-write manifest for task {}",
                    task_id
                );
                manifest = existing;
                should_create_stage = false;
            } else {
                info!(
                    "⚠️ Manifest invalid, stale, or not direct-write; creating new manifest for task {}",
                    task_id
                );
                existing.stages.remove(url);
                manifest = existing;
            }
        }

        if should_create_stage {
            manifest.stages.insert(
                url.to_string(),
                crate::core::resume::StageManifest {
                    stage_id: url.to_string(),
                    url: url.to_string(),
                    source_fingerprint: crate::core::resume::source_fingerprint(
                        url,
                        total_size,
                        &headers_hash,
                    ),
                    headers_hash,
                    total_size,
                    chunks: build_direct_write_chunks(output_path, total_size, num_chunks),
                    status: crate::core::resume::ChunkStatus::Pending,
                },
            );
            let _ = resume_store.save_manifest(&manifest).await;
        }

        let output_file = prepare_sparse_direct_output_file(output_path, total_size).await?;

        let stage_manifest = manifest.stages.get(url).unwrap().clone();
        num_chunks = stage_manifest.chunks.len();
        let max_active_connections = max_connections.min(num_chunks);

        let range_tuner = RangeTunerRuntime::new(max_active_connections, tuner_history);
        let mut join_set = tokio::task::JoinSet::new();

        let start_time = std::time::Instant::now();
        let initial_received: u64 = stage_manifest
            .chunks
            .iter()
            .map(|chunk| (chunk.end - chunk.start + 1).min(chunk.bytes_written))
            .sum();
        let downloaded_bytes = Arc::new(std::sync::atomic::AtomicU64::new(initial_received));
        let adaptive_controller = tokio::spawn(range_tuner::run_controller(
            range_tuner.clone(),
            downloaded_bytes.clone(),
            total_size,
            task_id.to_string(),
            url_host_for_log(url),
            token.clone(),
        ));
        let initial_status = if initial_received > 0 {
            TaskStatus::Resuming
        } else {
            TaskStatus::Downloading
        };
        if let Some(task_service) = task_service {
            task_service
                .update_task_status(
                    task_id,
                    initial_status,
                    map_progress(
                        progress_start,
                        progress_end,
                        initial_received as f64 / total_size as f64,
                    ),
                    0,
                    initial_received,
                )
                .await;
        }

        let manifest_mtx = Arc::new(tokio::sync::Mutex::new(manifest));
        let output_path_string = output_path.to_string_lossy().into_owned();
        let segment_queue = Arc::new(tokio::sync::Mutex::new(SegmentQueue::from_chunks(
            &stage_manifest.chunks,
            DEFAULT_MIN_SPLIT_BYTES,
        )));
        let segment_notify = Arc::new(Notify::new());
        let worker_count = max_active_connections.max(1);

        for worker_id in 0..worker_count {
            let output_file = output_file.clone();
            let client = self.client.clone();
            let url_clone = url.to_string();
            let base_headers = req_headers.clone();
            let db_counter = downloaded_bytes.clone();
            let resume_store_clone = resume_store.clone();
            let manifest_clone = manifest_mtx.clone();
            let token_clone = token.clone();
            let range_tuner_clone = range_tuner.clone();
            let queue_clone = segment_queue.clone();
            let notify_clone = segment_notify.clone();
            let output_path_clone = output_path_string.clone();
            let task_id_clone = task_id.to_string();
            let lease_bytes = range_segment_policy.lease_bytes;

            join_set.spawn(async move {
                loop {
                    if token_clone.is_cancelled() {
                        return Err(anyhow::anyhow!("任务已取消"));
                    }

                    let job = loop {
                        if token_clone.is_cancelled() {
                            return Err(anyhow::anyhow!("任务已取消"));
                        }

                        let maybe_job = {
                            let mut queue = queue_clone.lock().await;
                            if let Some(job) = queue.pop_next() {
                                Some(job)
                            } else if queue.is_finished() {
                                return Ok::<(), anyhow::Error>(());
                            } else {
                                None
                            }
                        };

                        if let Some(job) = maybe_job {
                            break job;
                        }

                        tokio::select! {
                            _ = token_clone.cancelled() => return Err(anyhow::anyhow!("任务已取消")),
                            _ = notify_clone.notified() => {}
                        }
                    };

                    let _permit = range_tuner_clone.acquire(&token_clone).await?;
                    let _active_chunk = ActiveChunkConnectionGuard::enter();
                    tracing::debug!(
                        task_id = %task_id_clone,
                        worker_id,
                        chunk_index = job.index,
                        range_start = job.start,
                        range_end = job.end,
                        next_offset = job.next_offset(),
                        attempts = job.attempts,
                        "http range worker picked segment"
                    );

                    let fetch_result = fetch_range_segment(
                        client.clone(),
                        &url_clone,
                        base_headers.clone(),
                        output_file.clone(),
                        db_counter.clone(),
                        manifest_clone.clone(),
                        resume_store_clone.clone(),
                        token_clone.clone(),
                        &job,
                        lease_bytes,
                    )
                    .await;

                    match fetch_result {
                        Ok((SegmentFetchStatus::Completed, _)) => {
                            {
                                let mut queue = queue_clone.lock().await;
                                queue.complete(job.index);
                            }
                            notify_clone.notify_waiters();
                        }
                        Ok((SegmentFetchStatus::LeaseCompleted, bytes_written)) => {
                            let split_job = {
                                let mut queue = queue_clone.lock().await;
                                queue.complete_lease(job.clone(), bytes_written)
                            };
                            if let Some(split_job) = split_job {
                                append_manifest_split_chunk(
                                    &manifest_clone,
                                    &resume_store_clone,
                                    &url_clone,
                                    &output_path_clone,
                                    job.index,
                                    &split_job,
                                )
                                .await;
                                tracing::info!(
                                    task_id = %task_id_clone,
                                    worker_id,
                                    original_chunk_index = job.index,
                                    new_chunk_index = split_job.index,
                                    split_start = split_job.start,
                                    split_end = split_job.end,
                                    "http range segment split"
                                );
                            }
                            notify_clone.notify_waiters();
                        }
                        Ok((SegmentFetchStatus::Recoverable, bytes_written)) => {
                            let split_job = {
                                let mut queue = queue_clone.lock().await;
                                queue.requeue(job.clone(), bytes_written)
                            };
                            if let Some(split_job) = split_job {
                                append_manifest_split_chunk(
                                    &manifest_clone,
                                    &resume_store_clone,
                                    &url_clone,
                                    &output_path_clone,
                                    job.index,
                                    &split_job,
                                )
                                .await;
                                tracing::info!(
                                    task_id = %task_id_clone,
                                    worker_id,
                                    original_chunk_index = job.index,
                                    new_chunk_index = split_job.index,
                                    split_start = split_job.start,
                                    split_end = split_job.end,
                                    "http range segment split"
                                );
                            }
                            tracing::info!(
                                task_id = %task_id_clone,
                                worker_id,
                                chunk_index = job.index,
                                bytes_written,
                                remaining_bytes = job.expected_size().saturating_sub(bytes_written),
                                attempts = job.attempts.saturating_add(1),
                                "http range segment requeued"
                            );
                            notify_clone.notify_waiters();
                        }
                        Err(error) => {
                            {
                                let mut queue = queue_clone.lock().await;
                                queue.complete(job.index);
                            }
                            tracing::error!(
                                task_id = %task_id_clone,
                                worker_id,
                                chunk_index = job.index,
                                %error,
                                "http range worker fatal"
                            );
                            return Err(error);
                        }
                    }
                }
            });
        }

        // 实时监控与汇报进度。不能只在超时时刷新，否则小文件会在 UI 看到 0% 后直接完成。
        let report_interval = tokio::time::Duration::from_millis(250);
        let mut last_report_at = std::time::Instant::now() - report_interval;
        let mut last_speed_sample_at = std::time::Instant::now();
        let mut last_report_received = initial_received;
        loop {
            match tokio::time::timeout(report_interval, join_set.join_next()).await {
                Ok(Some(res)) => {
                    res??; // 一块下载完成
                    if last_report_at.elapsed() >= tokio::time::Duration::from_millis(80) {
                        let received = downloaded_bytes
                            .load(std::sync::atomic::Ordering::Relaxed)
                            .min(total_size);
                        let report_elapsed = last_speed_sample_at.elapsed().as_secs_f64();
                        let speed = instant_speed_bytes_per_sec(
                            received,
                            last_report_received,
                            report_elapsed,
                        );
                        let local_progress = received as f64 / total_size as f64;
                        if let Some(task_service) = task_service {
                            task_service
                                .update_task_status(
                                    task_id,
                                    TaskStatus::Downloading,
                                    map_progress(progress_start, progress_end, local_progress),
                                    speed,
                                    received,
                                )
                                .await;
                        }
                        last_report_at = std::time::Instant::now();
                        last_speed_sample_at = last_report_at;
                        last_report_received = received;
                    }
                }
                Ok(None) => {
                    break; // 所有块下载完毕
                }
                Err(_) => {
                    let received = downloaded_bytes
                        .load(std::sync::atomic::Ordering::Relaxed)
                        .min(total_size);
                    let report_elapsed = last_speed_sample_at.elapsed().as_secs_f64();
                    let speed =
                        instant_speed_bytes_per_sec(received, last_report_received, report_elapsed);
                    let local_progress = received as f64 / total_size as f64;
                    if let Some(task_service) = task_service {
                        task_service
                            .update_task_status(
                                task_id,
                                TaskStatus::Downloading,
                                map_progress(progress_start, progress_end, local_progress),
                                speed,
                                received,
                            )
                            .await;
                    }
                    last_report_at = std::time::Instant::now();
                    last_speed_sample_at = last_report_at;
                    last_report_received = received;
                }
            }
        }
        adaptive_controller.abort();
        let _ = adaptive_controller.await;

        info!(
            output_path = %output_path.display(),
            "http range direct-write download completed"
        );
        flush_std_file(output_file).await?;
        let received = downloaded_bytes
            .load(std::sync::atomic::Ordering::Relaxed)
            .min(total_size);
        let elapsed = start_time.elapsed().as_secs_f64();
        let session_received = received.saturating_sub(initial_received);
        let final_speed = session_speed_bytes_per_sec(received, initial_received, elapsed);
        let tuner_outcome = range_tuner.outcome();
        if should_update_site_history(
            session_received,
            elapsed,
            tuner_outcome.best_speed_bytes_per_sec,
        ) && let Some(store) = site_performance_store()
        {
            match store
                .update_effective_target(
                    site_key.clone(),
                    tuner_outcome.best_target,
                    tuner_outcome.best_speed_bytes_per_sec,
                )
                .await
            {
                Ok(record) => {
                    tracing::info!(
                        task_id,
                        site_key = %record.site_key,
                        last_effective_target = record.last_effective_target,
                        last_effective_speed_bytes_per_sec = record.last_effective_speed_bytes_per_sec,
                        sample_count = record.sample_count,
                        initial_target = tuner_outcome.initial_target,
                        "site range performance history updated"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        task_id,
                        site_key = %site_key,
                        %error,
                        "failed to update site range performance history"
                    );
                }
            }
        }
        if let Some(task_service) = task_service {
            if let Some(status) = final_status {
                task_service
                    .update_task_status(
                        task_id,
                        status,
                        progress_end.clamp(0.0, 1.0),
                        final_speed,
                        received,
                    )
                    .await;
            } else {
                task_service
                    .update_task_status(
                        task_id,
                        TaskStatus::Downloading,
                        progress_end.clamp(0.0, 1.0),
                        final_speed,
                        received,
                    )
                    .await;
            }
        }

        Ok(())
    }

    /// 解析 m3u8 并处理 fMP4 或 TS 的多碎片并发下载
    pub async fn download_hls(
        &self,
        task: Task,
        task_service: &crate::services::task_service::TaskService,
        token: tokio_util::sync::CancellationToken,
    ) -> Result<String> {
        tokio::select! {
            _ = token.cancelled() => anyhow::bail!("任务已取消"),
            result = self.download_hls_impl(task, task_service, token.clone()) => result,
        }
    }

    async fn download_hls_impl(
        &self,
        task: Task,
        task_service: &crate::services::task_service::TaskService,
        token: tokio_util::sync::CancellationToken,
    ) -> Result<String> {
        let task_id = task.id.clone();
        let first_stage = task
            .stages
            .first()
            .context("HLS task has no playlist stage")?;
        let raw_url = first_stage.url.clone();
        let req_headers = request_headers(&first_stage.headers);
        let raw_host = url_host_for_log(&raw_url);

        tracing::info!(
            task_id = %task_id,
            request_id = ?task.request_id,
            title = %task.title,
            url_host = %raw_host,
            header_count = first_stage.headers.len(),
            "hls download started"
        );

        let initial_resp = self
            .client
            .get(&raw_url)
            .headers(req_headers.clone())
            .send()
            .await?
            .error_for_status()
            .map_err(|error| {
                tracing::error!(
                    task_id = %task_id,
                    request_id = ?task.request_id,
                    url_host = %raw_host,
                    %error,
                    "hls master playlist request failed"
                );
                error
            })?
            .text()
            .await?;
        let parsed = m3u8_rs::parse_playlist_res(initial_resp.as_bytes()).map_err(|e| {
            tracing::error!(
                task_id = %task_id,
                request_id = ?task.request_id,
                url_host = %raw_host,
                error = ?e,
                "hls master playlist parse failed"
            );
            anyhow::anyhow!("Failed to parse m3u8: {:?}", e)
        })?;

        let mut media_playlist_url = raw_url.clone();
        let mut media_playlist_content = initial_resp;
        let mut audio_playlist_url = None;
        let mut requires_audio = false;

        if let m3u8_rs::Playlist::MasterPlaylist(master) = parsed {
            let variant_count = master.variants.len();
            let selected_index =
                quality::select_highest_bandwidth_index(&master.variants, |variant| {
                    variant.bandwidth
                });

            if let Some(index) = selected_index {
                let variant = &master.variants[index];
                let base_url = reqwest::Url::parse(&raw_url)?;
                (audio_playlist_url, requires_audio) =
                    hls_audio_rendition(&master, variant, &base_url)?;
                media_playlist_url = base_url.join(&variant.uri)?.to_string();
                tracing::info!(
                    task_id = %task_id,
                    request_id = ?task.request_id,
                    url_host = %url_host_for_log(&media_playlist_url),
                    variant_count,
                    selected_bandwidth = variant.bandwidth,
                    separate_audio = audio_playlist_url.is_some(),
                    requires_audio,
                    "hls master playlist resolved variant"
                );
                media_playlist_content = self
                    .client
                    .get(&media_playlist_url)
                    .headers(req_headers.clone())
                    .send()
                    .await?
                    .error_for_status()
                    .map_err(|error| {
                        tracing::error!(
                            task_id = %task_id,
                            request_id = ?task.request_id,
                            url_host = %url_host_for_log(&media_playlist_url),
                            %error,
                            "hls media playlist request failed"
                        );
                        error
                    })?
                    .text()
                    .await?;
            } else {
                tracing::error!(
                    task_id = %task_id,
                    request_id = ?task.request_id,
                    variant_count,
                    "hls master playlist has no variants"
                );
                anyhow::bail!("No variants found");
            }
        }

        let temp_dir = Path::new("temp").join(&task_id);
        let video_progress_end = if audio_playlist_url.is_some() {
            0.9
        } else {
            1.0
        };
        let (combined_path, mut total_received) = self
            .download_hls_track(
                &task,
                task_service,
                &token,
                &req_headers,
                HlsTrack {
                    url: &media_playlist_url,
                    content: &media_playlist_content,
                    directory: temp_dir.join("video"),
                    progress_range: (0.0, video_progress_end),
                    received_before: 0,
                },
            )
            .await?;
        let mut audio_path = None;
        if let Some(url) = &audio_playlist_url {
            tracing::info!(task_id = %task_id, "hls separate audio download started");
            let content = self
                .client
                .get(url)
                .headers(req_headers.clone())
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            let (path, received) = self
                .download_hls_track(
                    &task,
                    task_service,
                    &token,
                    &req_headers,
                    HlsTrack {
                        url,
                        content: &content,
                        directory: temp_dir.join("audio"),
                        progress_range: (video_progress_end, 1.0),
                        received_before: total_received,
                    },
                )
                .await
                .context("HLS separate audio download failed")?;
            total_received += received;
            audio_path = Some(path);
        }

        task_service
            .update_task_status(&task_id, TaskStatus::Merging, 1.0, 0, total_received)
            .await;
        let output_path = Path::new(&task.path).to_path_buf();
        tokio::fs::create_dir_all(output_path.parent().unwrap_or_else(|| Path::new("."))).await?;
        // Publish only after all mandatory tracks have been successfully remuxed.
        let remuxed_path = temp_dir.join("remuxed").with_extension(
            output_path
                .extension()
                .unwrap_or_else(|| std::ffi::OsStr::new("mp4")),
        );
        tracing::info!(task_id = %task_id, separate_audio = audio_path.is_some(), requires_audio, "hls ffmpeg remux started");
        let mut command =
            crate::services::yt_dlp_update_service::hidden_command(Path::new("ffmpeg"));
        command
            .args(["-y", "-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(&combined_path);
        if let Some(path) = &audio_path {
            // Required (not optional '?') mapping makes missing/invalid audio a failure.
            command
                .arg("-i")
                .arg(path)
                .args(["-map", "0:v:0", "-map", "1:a:0"]);
        } else if requires_audio {
            command.args(["-map", "0:v:0", "-map", "0:a:0"]);
        } else {
            command.args(["-map", "0"]);
        }
        let output = command
            .args(["-c", "copy"])
            .arg(&remuxed_path)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await?;
        if !output.status.success() {
            let detail = short_log_text(&output.stderr);
            tracing::error!(task_id = %task_id, stderr = %detail, "hls ffmpeg remux failed");
            anyhow::bail!("HLS remux failed (required audio must be present): {detail}");
        }
        tokio::fs::copy(&remuxed_path, &output_path).await?;
        tracing::info!(task_id = %task_id, received_bytes = total_received, requires_audio, "hls download completed");
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        Ok(output_path.display().to_string())
    }

    async fn download_hls_track(
        &self,
        task: &Task,
        task_service: &crate::services::task_service::TaskService,
        token: &tokio_util::sync::CancellationToken,
        req_headers: &header::HeaderMap,
        track: HlsTrack<'_>,
    ) -> Result<(PathBuf, u64)> {
        let task_id = &task.id;
        let media_playlist_url = track.url;
        let media_parsed = m3u8_rs::parse_playlist_res(track.content.as_bytes()).map_err(|e| {
            tracing::error!(
                task_id = %task_id,
                request_id = ?task.request_id,
                url_host = %url_host_for_log(&media_playlist_url),
                error = ?e,
                "hls media playlist parse failed"
            );
            anyhow::anyhow!("Failed to parse media m3u8: {:?}", e)
        })?;

        let segments = match media_parsed {
            m3u8_rs::Playlist::MediaPlaylist(m) => m.segments,
            _ => anyhow::bail!("Expected a media playlist"),
        };

        let total_segments = segments.len();
        if total_segments == 0 {
            tracing::error!(
                task_id = %task_id,
                request_id = ?task.request_id,
                url_host = %url_host_for_log(&media_playlist_url),
                "hls media playlist has no segments"
            );
            anyhow::bail!("No segments");
        }
        tracing::info!(
            task_id = %task_id,
            request_id = ?task.request_id,
            url_host = %url_host_for_log(&media_playlist_url),
            total_segments,
            "hls media playlist parsed"
        );

        let temp_dir = &track.directory;
        tokio::fs::create_dir_all(&temp_dir).await?;

        let mut init_url = None;
        if let Some(first_seg) = segments.first()
            && let Some(map) = &first_seg.map
        {
            let base_url = reqwest::Url::parse(&media_playlist_url)?;
            init_url = Some(base_url.join(&map.uri)?.to_string());
        }

        let init_path = temp_dir.join("init.mp4");
        if let Some(url) = init_url {
            tracing::info!(
                task_id = %task_id,
                request_id = ?task.request_id,
                url_host = %url_host_for_log(&url),
                "hls init segment download started"
            );
            let mut res = self
                .client
                .get(&url)
                .headers(req_headers.clone())
                .send()
                .await?
                .error_for_status()
                .map_err(|error| {
                    tracing::error!(
                        task_id = %task_id,
                        request_id = ?task.request_id,
                        url_host = %url_host_for_log(&url),
                        %error,
                        "hls init segment request failed"
                    );
                    error
                })?;
            let mut file = File::create(&init_path).await?;
            while let Some(chunk) = res.chunk().await? {
                file.write_all(&chunk).await?;
            }
            file.flush().await?;
        }

        let semaphore = Arc::new(Semaphore::new(16));
        let mut join_set = tokio::task::JoinSet::new();
        let start_time = std::time::Instant::now();
        let received_bytes = Arc::new(AtomicU64::new(0));

        for (i, segment) in segments.into_iter().enumerate() {
            let base_url = reqwest::Url::parse(&media_playlist_url)?;
            let chunk_url = base_url.join(&segment.uri)?.to_string();
            let chunk_path = temp_dir.join(format!("{:05}.ts", i));

            let permit = semaphore.clone().acquire_owned().await?;
            let client = self.client.clone();
            let headers = req_headers.clone();
            let received_counter = received_bytes.clone();

            let token_clone = token.clone();
            join_set.spawn(async move {
                if chunk_path.exists() {
                    received_counter.fetch_add(tokio::fs::metadata(&chunk_path).await?.len(), Ordering::Relaxed);
                    drop(permit);
                    return Ok::<(), anyhow::Error>(());
                }

                let partial_path = chunk_path.with_extension("part");

                let mut retries = retry::HLS_SEGMENT_RETRY_POLICY.max_attempts;
                let mut success = false;
                while retries > 0 {
                    match client.get(&chunk_url).headers(headers.clone()).send().await {
                        Ok(mut res) => {
                            if res.status().is_success() {
                            if let Ok(mut file) = File::create(&partial_path).await {
                                loop {
                                    tokio::select! {
                                        _ = token_clone.cancelled() => {
                                            return Err(anyhow::anyhow!("任务已取消"));
                                        }
                                        chunk_res = res.chunk() => {
                                            match chunk_res {
                                                Ok(Some(chunk)) => {
                                                    received_counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                                                    file.write_all(&chunk).await?;
                                                }
                                                Ok(None) => {
                                                    file.flush().await?;
                                                    success = true;
                                                    break;
                                                }
                                                Err(_) => break,
                                            }
                                        }
                                    }
                                }
                                if success {
                                    drop(file);
                                    tokio::fs::rename(&partial_path, &chunk_path).await?;
                                    break;
                                }
                            }
                            } else {
                                tracing::warn!(
                                    segment_index = i,
                                    status = %res.status(),
                                    "hls segment request returned non-success status"
                                );
                            }
                        }
                        Err(error) => {
                            tracing::warn!(
                                segment_index = i,
                                %error,
                                "hls segment request failed"
                            );
                        }
                    }
                    retries -= 1;
                    tokio::time::sleep(tokio::time::Duration::from_millis(
                        retry::HLS_SEGMENT_RETRY_POLICY.backoff_ms,
                    ))
                    .await;
                }
                drop(permit);
                if !success { anyhow::bail!("Failed chunk {}", i); }
                Ok(())
            });
        }

        let mut completed = 0;
        while let Some(res) = join_set.join_next().await {
            res??;
            completed += 1;
            let elapsed = start_time.elapsed().as_secs_f64();
            let received = received_bytes.load(Ordering::Relaxed);
            let speed = if elapsed > 0.0 {
                (received as f64 / elapsed) as u64
            } else {
                0
            };

            if completed % 5 == 0 || completed == total_segments {
                task_service
                    .update_task_status(
                        &task_id,
                        TaskStatus::Downloading,
                        map_progress(
                            track.progress_range.0,
                            track.progress_range.1,
                            completed as f64 / total_segments as f64,
                        ),
                        speed,
                        track.received_before + received,
                    )
                    .await;
            }
        }
        let total_received = received_bytes.load(Ordering::Relaxed);
        tracing::info!(
            task_id = %task_id,
            request_id = ?task.request_id,
            total_segments,
            received_bytes = total_received,
            "hls segments downloaded"
        );

        let combined_path = temp_dir.join("combined.mp4");
        let mut combined_file = File::create(&combined_path).await?;

        if init_path.exists() {
            let mut init_f = File::open(&init_path).await?;
            tokio::io::copy(&mut init_f, &mut combined_file).await?;
        }

        for i in 0..total_segments {
            let chunk_path = temp_dir.join(format!("{:05}.ts", i));
            let mut chunk_f = File::open(&chunk_path).await?;
            tokio::io::copy(&mut chunk_f, &mut combined_file).await?;
        }
        combined_file.flush().await?;
        drop(combined_file);
        Ok((combined_path, total_received))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::policy::output_naming;

    fn audio_master(audio_tags: &str, attributes: &str) -> m3u8_rs::MasterPlaylist {
        let text = format!(
            "#EXTM3U\n{audio_tags}\n#EXT-X-STREAM-INF:BANDWIDTH=1000,{attributes}\nvideo/index.m3u8\n"
        );
        match m3u8_rs::parse_playlist_res(text.as_bytes()).unwrap() {
            m3u8_rs::Playlist::MasterPlaylist(master) => master,
            _ => panic!("expected master"),
        }
    }

    #[test]
    fn hls_audio_matches_selected_group_and_prefers_default_rendition() {
        let master = audio_master(
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"low\",NAME=\"wrong\",DEFAULT=YES,URI=\"wrong.m3u8\"\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"high\",NAME=\"other\",AUTOSELECT=YES,URI=\"other.m3u8\"\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"high\",NAME=\"default\",DEFAULT=YES,URI=\"../audio/index.m3u8\"",
            "AUDIO=\"high\"",
        );
        let (url, required) = hls_audio_rendition(
            &master,
            &master.variants[0],
            &reqwest::Url::parse("https://example.test/pl/master.m3u8").unwrap(),
        )
        .unwrap();
        assert_eq!(
            url.as_deref(),
            Some("https://example.test/audio/index.m3u8")
        );
        assert!(required);
    }

    #[test]
    fn hls_missing_referenced_audio_group_is_an_error() {
        let master = audio_master("", "AUDIO=\"missing\"");
        assert!(
            hls_audio_rendition(
                &master,
                &master.variants[0],
                &reqwest::Url::parse("https://example.test/master.m3u8").unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn hls_uri_less_audio_rendition_requires_muxed_audio() {
        let master = audio_master(
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"muxed\",NAME=\"audio\"",
            "AUDIO=\"muxed\"",
        );
        assert_eq!(
            hls_audio_rendition(
                &master,
                &master.variants[0],
                &reqwest::Url::parse("https://example.test/master.m3u8").unwrap()
            )
            .unwrap(),
            (None, true)
        );
    }

    #[test]
    fn hls_codec_audio_declaration_is_not_silently_ignored() {
        for (codecs, expected) in [
            ("avc1.64001F,mp4a.40.2", true),
            ("vp09.00.10.08,opus", true),
            ("avc1.64001F", false),
        ] {
            let master = audio_master("", &format!("CODECS=\"{codecs}\""));
            assert_eq!(
                hls_audio_rendition(
                    &master,
                    &master.variants[0],
                    &reqwest::Url::parse("https://example.test/master.m3u8").unwrap()
                )
                .unwrap(),
                (None, expected)
            );
        }
    }

    #[test]
    fn range_requests_force_raw_identity_encoding() {
        let mut headers = header::HeaderMap::new();

        insert_raw_range_headers(&mut headers, 10, 20).unwrap();

        assert_eq!(headers.get(header::RANGE).unwrap(), "bytes=10-20");
        assert_eq!(headers.get(header::ACCEPT_ENCODING).unwrap(), "identity");
    }

    #[test]
    fn direct_write_chunks_target_final_output_offsets() {
        let output = Path::new("downloads/final.bin");

        let chunks = build_direct_write_chunks(output, 101, 4);

        assert_eq!(chunks.len(), 4);
        assert_eq!((chunks[0].start, chunks[0].end), (0, 24));
        assert_eq!((chunks[1].start, chunks[1].end), (25, 49));
        assert_eq!((chunks[2].start, chunks[2].end), (50, 74));
        assert_eq!((chunks[3].start, chunks[3].end), (75, 100));
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.path == "downloads/final.bin")
        );
    }

    #[test]
    fn stage_range_validation_allows_dynamic_split_chunks() {
        let stage = crate::core::resume::StageManifest {
            stage_id: "stage".to_string(),
            url: "https://example.com/file.bin".to_string(),
            source_fingerprint: String::new(),
            headers_hash: String::new(),
            total_size: 100,
            chunks: vec![
                crate::core::resume::ChunkManifest {
                    index: 0,
                    start: 0,
                    end: 49,
                    path: "file.bin".to_string(),
                    status: crate::core::resume::ChunkStatus::Pending,
                    bytes_written: 10,
                    checksum: None,
                },
                crate::core::resume::ChunkManifest {
                    index: 1,
                    start: 50,
                    end: 99,
                    path: "file.bin".to_string(),
                    status: crate::core::resume::ChunkStatus::Pending,
                    bytes_written: 0,
                    checksum: None,
                },
            ],
            status: crate::core::resume::ChunkStatus::Pending,
        };

        assert!(stage_covers_total_range(&stage));
    }

    #[test]
    fn stage_range_validation_rejects_gaps() {
        let stage = crate::core::resume::StageManifest {
            stage_id: "stage".to_string(),
            url: "https://example.com/file.bin".to_string(),
            source_fingerprint: String::new(),
            headers_hash: String::new(),
            total_size: 100,
            chunks: vec![
                crate::core::resume::ChunkManifest {
                    index: 0,
                    start: 0,
                    end: 40,
                    path: "file.bin".to_string(),
                    status: crate::core::resume::ChunkStatus::Pending,
                    bytes_written: 10,
                    checksum: None,
                },
                crate::core::resume::ChunkManifest {
                    index: 1,
                    start: 50,
                    end: 99,
                    path: "file.bin".to_string(),
                    status: crate::core::resume::ChunkStatus::Pending,
                    bytes_written: 0,
                    checksum: None,
                },
            ],
            status: crate::core::resume::ChunkStatus::Pending,
        };

        assert!(!stage_covers_total_range(&stage));
    }

    #[test]
    fn large_http_files_are_split_into_more_segments_than_active_connections() {
        let segments = plan_range_segment_count(64 * 1024 * 1024 * 1024, 128);

        assert_eq!(segments, 512);
    }

    #[test]
    fn large_http_files_can_plan_segments_for_256_connection_ceiling() {
        let segments = plan_range_segment_count(64 * 1024 * 1024 * 1024, 256);

        assert_eq!(segments, 1024);
    }

    #[test]
    fn small_http_files_do_not_create_tiny_range_segments() {
        let segments = plan_range_segment_count(32 * 1024 * 1024, 128);

        assert_eq!(segments, 2);
    }

    #[test]
    fn googlevideo_uses_finer_segments_for_medium_video_streams() {
        let policy =
            range_segment_policy_for_url("https://rr1---sn-a5meknek.googlevideo.com/videoplayback");
        let segments = plan_range_segment_count_with_policy(90_029_224, 256, policy);

        assert_eq!(policy.kind, RangeSegmentPolicyKind::GoogleVideo);
        assert_eq!(policy.lease_bytes, GOOGLEVIDEO_RANGE_LEASE_BYTES);
        assert_eq!(segments, 22);
    }

    #[test]
    fn generic_http_uses_bounded_range_leases() {
        let policy = range_segment_policy_for_url("https://cdn1.example.test/file.rar");

        assert_eq!(policy.kind, RangeSegmentPolicyKind::Generic);
        assert_eq!(policy.lease_bytes, RANGE_LEASE_BYTES);
        assert!(policy.lease_bytes <= policy.target_segment_bytes);
    }

    #[test]
    fn googlevideo_still_keeps_tiny_files_single_range() {
        let policy =
            range_segment_policy_for_url("https://rr1---sn-a5meknek.googlevideo.com/videoplayback");
        let segments = plan_range_segment_count_with_policy(10 * 1024 * 1024, 256, policy);

        assert_eq!(segments, 1);
    }

    #[test]
    fn resume_speed_uses_only_session_bytes() {
        let initial_received = 40 * 1024 * 1024 * 1024;
        let total_received = initial_received + 100 * 1024 * 1024;

        let speed = session_speed_bytes_per_sec(total_received, initial_received, 10.0);

        assert_eq!(speed, 10 * 1024 * 1024);
    }

    #[test]
    fn instant_speed_uses_only_last_report_delta() {
        let speed = instant_speed_bytes_per_sec(150 * 1024 * 1024, 100 * 1024 * 1024, 2.0);

        assert_eq!(speed, 25 * 1024 * 1024);
    }

    #[test]
    fn hls_output_path_uses_twitter_media_id() {
        let first = output_naming::hls_output_path(
            "X-a (@a)",
            "https://video.twimg.com/amplify_video/2062607626618163200/pl/m9QXQzbPfw2D8al2.m3u8?tag=27",
            "task-a",
        );
        let second = output_naming::hls_output_path(
            "X-b (@b)",
            "https://video.twimg.com/amplify_video/2059173030660857857/pl/PYxKRNeRFU50wGWI.m3u8?tag=27",
            "task-b",
        );
        let ext_tw_video = output_naming::hls_output_path(
            "X-c (@c)",
            "https://video.twimg.com/ext_tw_video/2063456657753944064/pu/pl/4KaiIKxMVoDTCAs6.m3u8?tag=12&variant_version=1",
            "task-c",
        );

        assert!(first.ends_with("X-a (@a)-2062607626618163200.mp4"));
        assert!(second.ends_with("X-b (@b)-2059173030660857857.mp4"));
        assert!(ext_tw_video.ends_with("X-c (@c)-2063456657753944064.mp4"));
        assert_ne!(first, second);
    }

    #[test]
    fn hls_output_path_falls_back_to_task_id() {
        assert!(
            output_naming::hls_output_path("X-media", "not a url", "abcdef12-3456")
                .ends_with("X-media-abcdef12.mp4")
        );
    }
}
