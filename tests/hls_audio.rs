//! Offline end-to-end regressions. Requires ffmpeg + ffprobe on PATH, as does
//! the installed downloader. Media is synthesized locally, never fetched online.
use axum::{Router, http::StatusCode, response::IntoResponse};
use gd3_daemon::core::engine::downloader::UniversalDownloader;
use gd3_daemon::core::models::{PackId, Stage, StageType, Task};
use gd3_daemon::core::resume::ResumeStore;
use gd3_daemon::packs::bili_pack::dash_downloader::DashDownloader;
use gd3_daemon::services::task_artifact_service::{cleanup_task_artifacts, task_work_dir};
use gd3_daemon::services::task_service::TaskService;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

fn media_command(tool: &str, args: &[&str], output: &Path) -> Output {
    let mut command = Command::new(tool);
    // FFmpeg writes the fMP4 init file relative to its working directory.
    command.current_dir(output.parent().unwrap());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let result = command
        .args(args)
        .arg(output)
        .output()
        .expect("ffmpeg/ffprobe must be installed");
    assert!(
        result.status.success(),
        "{tool}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    result
}

#[derive(Clone, Copy)]
enum Mode {
    Separate,
    MissingAudio,
    InvalidAudio,
    Muxed,
    VideoOnly,
    DirectMedia,
    MissingMuxedAudio,
}

struct Fixture {
    root: PathBuf,
    task: Task,
    requests: Arc<Mutex<Vec<String>>>,
    workspace_observed: Arc<std::sync::atomic::AtomicBool>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new(mode: Mode) -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = format!(
            "hls-audio-test-{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap(),
            NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(&id);
        let mut files = HashMap::new();
        for (directory, source_args) in [
            (
                "video",
                vec![
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=blue:s=64x64:r=10:d=1",
                    "-an",
                    "-c:v",
                    "libx264",
                    "-g",
                    "10",
                ],
            ),
            (
                "audio",
                vec![
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=880:duration=1",
                    "-vn",
                    "-c:a",
                    "aac",
                ],
            ),
            (
                "muxed",
                vec![
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=blue:s=64x64:r=10:d=1",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=880:duration=1",
                    "-c:v",
                    "libx264",
                    "-g",
                    "10",
                    "-c:a",
                    "aac",
                ],
            ),
        ] {
            let dir = root.join(directory);
            std::fs::create_dir_all(&dir).unwrap();
            let mut args = vec!["-y", "-nostdin", "-hide_banner", "-loglevel", "error"];
            args.extend(source_args);
            args.extend([
                "-f",
                "hls",
                "-hls_time",
                "1",
                "-hls_playlist_type",
                "vod",
                "-hls_segment_type",
                "fmp4",
            ]);
            media_command("ffmpeg", &args, &dir.join("index.m3u8"));
            for entry in std::fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                files.insert(
                    format!("/{directory}/{}", entry.file_name().to_string_lossy()),
                    std::fs::read(entry.path()).unwrap(),
                );
            }
        }
        // Also expose direct fMP4 resources for the Bilibili/captured-DASH path.
        for directory in ["video", "audio"] {
            let mut joined = files[&format!("/{directory}/init.mp4")].clone();
            let mut paths: Vec<_> = files
                .keys()
                .filter(|name| {
                    name.starts_with(&format!("/{directory}/")) && name.ends_with(".m4s")
                })
                .cloned()
                .collect();
            paths.sort();
            for name in paths {
                joined.extend_from_slice(&files[&name]);
            }
            files.insert(format!("/{directory}.mp4"), joined);
        }
        let separate_master = "#EXTM3U\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"low\",NAME=\"wrong\",DEFAULT=YES,URI=\"wrong.m3u8\"\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"high\",NAME=\"audio\",DEFAULT=YES,URI=\"audio/index.m3u8\"\n\
            #EXT-X-STREAM-INF:BANDWIDTH=10,AUDIO=\"low\"\nwrong-video.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=1000,CODECS=\"avc1.64001F,mp4a.40.2\",AUDIO=\"high\"\nvideo/index.m3u8\n";
        let (master, source) = match mode {
            Mode::Muxed => (
                "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000,CODECS=\"avc1.64001F,mp4a.40.2\"\nmuxed/index.m3u8\n",
                "/master.m3u8",
            ),
            Mode::VideoOnly => (
                "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000,CODECS=\"avc1.64001F\"\nvideo/index.m3u8\n",
                "/master.m3u8",
            ),
            Mode::DirectMedia => ("", "/muxed/index.m3u8"),
            Mode::MissingMuxedAudio => (
                "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000,CODECS=\"avc1.64001F,mp4a.40.2\"\nvideo/index.m3u8\n",
                "/master.m3u8",
            ),
            _ => (separate_master, "/master.m3u8"),
        };
        files.insert("/master.m3u8".to_owned(), master.as_bytes().to_vec());
        if matches!(mode, Mode::MissingAudio) {
            files.remove("/audio/index.m3u8");
        }
        if matches!(mode, Mode::InvalidAudio) {
            // A valid playlist/segments with *no audio* must not be accepted.
            let replacements: Vec<_> = files
                .iter()
                .filter(|(path, _)| path.starts_with("/video/"))
                .map(|(path, bytes)| (path.replacen("/video/", "/audio/", 1), bytes.clone()))
                .collect();
            files.extend(replacements);
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let workspace_observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = workspace_observed.clone();
        let expected_workspace = root.join(format!(".{id}.parts"));
        let request_log = requests.clone();
        let files = Arc::new(files);
        let router = Router::new().fallback(
            move |uri: axum::http::Uri, headers: axum::http::HeaderMap| {
                let files = files.clone();
                let requests = request_log.clone();
                let observed = observed.clone();
                let expected_workspace = expected_workspace.clone();
                async move {
                    if expected_workspace.is_dir() {
                        observed.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    requests.lock().unwrap().push(uri.path().to_owned());
                    if headers.get("x-media-test").and_then(|v| v.to_str().ok())
                        != Some("inherited")
                    {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    match files.get(uri.path()) {
                        Some(body) => {
                            if let Some(range) = headers
                                .get("range")
                                .and_then(|v| v.to_str().ok())
                                .and_then(|v| v.strip_prefix("bytes="))
                            {
                                let (start, end) = range.split_once('-').unwrap();
                                let start: usize = start.parse().unwrap();
                                let end: usize =
                                    end.parse().unwrap_or(body.len() - 1).min(body.len() - 1);
                                (
                                    StatusCode::PARTIAL_CONTENT,
                                    [(
                                        "content-range",
                                        format!("bytes {start}-{end}/{}", body.len()),
                                    )],
                                    body[start..=end].to_vec(),
                                )
                                    .into_response()
                            } else {
                                (StatusCode::OK, [("accept-ranges", "bytes")], body.clone())
                                    .into_response()
                            }
                        }
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut task = Task::new(
            id,
            "HLS audio regression".to_owned(),
            format!("http://{address}{source}"),
            PackId::Hls,
            0,
            root.join("result.mp4").display().to_string(),
        );
        task.add_stage(Stage {
            stage_index: 0,
            stage_type: StageType::Video,
            url: task.url.clone(),
            file_size: 0,
            headers: HashMap::from([("x-media-test".to_owned(), "inherited".to_owned())]),
            block_num: 1,
        });
        Self {
            root,
            task,
            requests,
            workspace_observed,
            server,
        }
    }

    async fn download(&self) -> anyhow::Result<String> {
        let service = TaskService::new(self.root.to_str().unwrap());
        tokio::time::timeout(
            std::time::Duration::from_secs(20),
            UniversalDownloader::new().download_hls(
                self.task.clone(),
                &service,
                CancellationToken::new(),
            ),
        )
        .await?
    }

    fn codecs(&self) -> Vec<String> {
        let result = media_command(
            "ffprobe",
            &[
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type",
                "-of",
                "json",
            ],
            Path::new(&self.task.path),
        );
        let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        json["streams"]
            .as_array()
            .unwrap()
            .iter()
            .map(|stream| stream["codec_type"].as_str().unwrap().to_owned())
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn hls_separate_fmp4_audio_is_downloaded_and_mapped_into_output() {
    let fixture = Fixture::new(Mode::Separate).await;
    fixture.download().await.unwrap();
    assert_eq!(fixture.codecs(), ["video", "audio"]);
    assert!(
        fixture
            .workspace_observed
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    assert!(!task_work_dir(&fixture.task).unwrap().exists());
    assert!(!Path::new("temp").join(&fixture.task.id).exists());
    let requests = fixture.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .any(|path| path.starts_with("/audio/") && path.ends_with(".m4s"))
    );
    assert!(!requests.iter().any(|path| path.contains("wrong")));
}

#[tokio::test]
async fn hls_unavailable_audio_is_failure_and_does_not_replace_existing_output() {
    let fixture = Fixture::new(Mode::MissingAudio).await;
    std::fs::write(&fixture.task.path, b"previous output").unwrap();
    assert!(fixture.download().await.is_err());
    let work_dir = task_work_dir(&fixture.task).unwrap();
    assert_eq!(work_dir.parent(), Some(fixture.root.as_path()));
    assert!(work_dir.join("hls/video/combined.mp4").exists());
    assert!(!Path::new("temp").join(&fixture.task.id).exists());
    assert_eq!(
        std::fs::read(&fixture.task.path).unwrap(),
        b"previous output"
    );
}

#[tokio::test]
async fn hls_audio_playlist_without_audio_is_failure_not_silent_success() {
    let fixture = Fixture::new(Mode::InvalidAudio).await;
    let error = fixture.download().await.unwrap_err().to_string();
    assert!(error.contains("required audio"), "{error}");
    assert!(!Path::new(&fixture.task.path).exists());
}

#[tokio::test]
async fn hls_muxed_audio_is_preserved() {
    let fixture = Fixture::new(Mode::Muxed).await;
    fixture.download().await.unwrap();
    assert_eq!(fixture.codecs(), ["video", "audio"]);
}

#[tokio::test]
async fn hls_direct_media_playlist_keeps_embedded_audio() {
    let fixture = Fixture::new(Mode::DirectMedia).await;
    fixture.download().await.unwrap();
    assert_eq!(fixture.codecs(), ["video", "audio"]);
}

#[tokio::test]
async fn hls_intentionally_video_only_stream_is_still_supported() {
    let fixture = Fixture::new(Mode::VideoOnly).await;
    fixture.download().await.unwrap();
    assert_eq!(fixture.codecs(), ["video"]);
}

#[tokio::test]
async fn hls_declared_muxed_audio_must_exist() {
    let fixture = Fixture::new(Mode::MissingMuxedAudio).await;
    assert!(
        fixture
            .download()
            .await
            .unwrap_err()
            .to_string()
            .contains("required audio")
    );
    assert!(!Path::new(&fixture.task.path).exists());
}

#[tokio::test]
async fn deleting_failed_hls_task_removes_adjacent_workspace() {
    let fixture = Fixture::new(Mode::InvalidAudio).await;
    assert!(fixture.download().await.is_err());
    assert!(task_work_dir(&fixture.task).unwrap().exists());
    let resume_store = ResumeStore::new(fixture.root.join("resumes").to_str().unwrap());
    resume_store.init().await.unwrap();
    cleanup_task_artifacts(&fixture.task, &resume_store)
        .await
        .unwrap();
    assert!(!task_work_dir(&fixture.task).unwrap().exists());
    assert!(fixture.root.exists());
}

#[tokio::test]
async fn bilibili_dash_uses_target_workspace_and_cleans_it_after_merge() {
    let fixture = Fixture::new(Mode::Separate).await;
    let mut task = fixture.task.clone();
    let origin = task.url.trim_end_matches("/master.m3u8").to_string();
    task.stages = vec![
        Stage {
            url: format!("{origin}/video.mp4"),
            ..task.stages[0].clone()
        },
        Stage {
            stage_index: 1,
            stage_type: StageType::Audio,
            url: format!("{origin}/audio.mp4"),
            ..task.stages[0].clone()
        },
    ];
    let service = TaskService::new(fixture.root.to_str().unwrap());
    let store = Arc::new(ResumeStore::new(
        fixture.root.join("resumes").to_str().unwrap(),
    ));
    store.init().await.unwrap();
    DashDownloader::new()
        .download_and_merge(task.clone(), &service, store, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(fixture.codecs(), ["video", "audio"]);
    assert!(
        fixture
            .workspace_observed
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    assert!(!task_work_dir(&task).unwrap().exists());
    assert!(!Path::new("temp").join(&task.id).exists());
}
