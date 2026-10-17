//! Offline browser payload -> parser -> Rust engine -> exact selected target.
use axum::{Router, routing::get};
use gd3_daemon::{
    core::{engine::executor::DownloadExecutor, resume::ResumeStore},
    parsers::{
        policy::{
            output_category::{OutputCategory, classify_output},
            output_destination::{apply_output_destination, validate_save_path},
        },
        protocol::http_file::HttpFileProtocolParser,
        registry::ParserRegistry,
    },
    protocol::{
        settings::{AppSettingsDto, DownloadDirectoryMode},
        task::{CreateTaskCommand, CreateTaskHttpReq},
    },
    services::{task_artifact_service::cleanup_task_artifacts, task_service::TaskService},
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn browser_image_download_keeps_selected_path_filename_and_bytes() {
    // A real one-pixel PNG. Equality verifies no re-encoding/conversion occurred.
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII="
    ).unwrap();
    let served = bytes.clone();
    let router = Router::new().route(
        "/image",
        get(move || {
            let body = served.clone();
            async move { ([("content-type", "image/png")], body) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/image?name=remote-name.jpg",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let root = std::env::temp_dir().join(format!(
        "nexus-browser-save-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let data = root.join("data");
    tokio::fs::create_dir_all(&data).await.unwrap();
    let service = TaskService::new(data.to_str().unwrap());
    let resumes = Arc::new(ResumeStore::new(data.join("resumes").to_str().unwrap()));
    resumes.init().await.unwrap();
    let mut registry = ParserRegistry::new();
    registry.register_site(Box::new(
        gd3_daemon::parsers::site::bilibili::BilibiliSiteParser::new(),
    ));
    registry.register_protocol(Box::new(HttpFileProtocolParser));
    for (index, mode) in [
        DownloadDirectoryMode::Flat,
        DownloadDirectoryMode::Categorized,
        DownloadDirectoryMode::Custom,
        DownloadDirectoryMode::SystemDefault,
    ]
    .into_iter()
    .enumerate()
    {
        let target = root
            .join("用户选定的位置")
            .join(format!("原图 ({index}).png"));
        let id = format!("save_path_{index}");
        let req: CreateTaskHttpReq = serde_json::from_value(serde_json::json!({
            "request_id": id, "source": "download", "title": "原图.png",
            "payload": { "url": url, "filename": "原图.png", "save_path": target,
                "mime": "image/png", "size": bytes.len(), "supports_range": false },
            "client_context": {"page_url": "https://www.bilibili.com/video/BV1test"}
        }))
        .unwrap();
        let cmd = CreateTaskCommand::from_http(req, None);
        validate_save_path(&cmd).unwrap();
        let plan = registry.resolve(&id, &cmd).await.unwrap();
        let category = classify_output(&cmd, &plan);
        assert_eq!(category, OutputCategory::Image);
        let settings = AppSettingsDto {
            directory_mode: mode,
            download_folder: root.join("should-not-be-used").display().to_string(),
            ..Default::default()
        };
        let plan = apply_output_destination(&cmd, plan, &category, &settings);
        let task = plan.clone().into_task(url.clone());
        service.add_task(task.clone()).await.unwrap();
        let output = DownloadExecutor::new(resumes.clone())
            .execute(plan, &service, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(std::path::PathBuf::from(output), target);
        assert_eq!(tokio::fs::read(&target).await.unwrap(), bytes);
        assert!(!root.join("should-not-be-used").exists());
        cleanup_task_artifacts(&task, &resumes).await.unwrap();
        assert!(!target.exists());
    }
    server.abort();
    // Only this test's uniquely named fixture directory is removed.
    tokio::fs::remove_dir_all(&root).await.unwrap();
}

#[cfg(windows)]
#[test]
fn windows_association_icons_cover_images_and_archives() {
    for extension in ["jpg", "png", "zip"] {
        let png = gd3_daemon::platform::system_icon::file_type_icon_png(extension).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "{extension}");
        let image = image::load_from_memory(&png).unwrap().to_rgba8();
        assert!(image.width() > 0 && image.pixels().any(|pixel| pixel.0[3] > 0));
    }
}

#[tokio::test]
async fn full_response_download_respects_limits_and_cancellation() {
    use gd3_daemon::core::engine::downloader::UniversalDownloader;
    use std::collections::HashMap;
    // One response has Content-Length, the other streams without a known size.
    let router = Router::new()
        .route("/fixed", get(|| async { vec![42u8; 64] }))
        .route(
            "/stream",
            get(|| async {
                axum::body::Body::from_stream(tokio_stream::iter([
                    Ok::<_, std::io::Error>(vec![42u8; 32]),
                    Ok(vec![42u8; 32]),
                ]))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let root = std::env::temp_dir().join(format!(
        "nexus-full-response-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    tokio::fs::create_dir_all(&root).await.unwrap();
    let engine = UniversalDownloader::new();
    for route in ["fixed", "stream"] {
        let path = root.join(format!("{route}.bin"));
        let result = engine
            .download_internal_file(
                &format!("{base}/{route}"),
                &path,
                HashMap::new(),
                16,
                CancellationToken::new(),
            )
            .await;
        assert!(result.unwrap_err().to_string().contains("size exceeds"));
        assert!(
            tokio::fs::metadata(&path)
                .await
                .map_or(true, |meta| meta.len() <= 16)
        );
    }
    let token = CancellationToken::new();
    token.cancel();
    let path = root.join("cancelled.bin");
    let result = engine
        .download_internal_file(&format!("{base}/fixed"), &path, HashMap::new(), 1024, token)
        .await;
    assert!(result.unwrap_err().to_string().contains("cancelled"));
    assert!(!path.exists());
    server.abort();
    // Scoped to this test's uniquely named fixture directory.
    tokio::fs::remove_dir_all(&root).await.unwrap();
}
