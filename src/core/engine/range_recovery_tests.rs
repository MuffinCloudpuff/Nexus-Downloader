use super::*;
use axum::{Router, body::Body, http::HeaderMap, routing::get};
use futures_util::StreamExt;
use std::sync::{Mutex, atomic::AtomicUsize};

#[tokio::test]
async fn validated_resume_preserves_existing_prefix_and_fresh_download_truncates() {
    let path = std::env::temp_dir().join(format!(
        "nexus-resume-prefix-{}-{}.bin",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let file = prepare_sparse_direct_output_file(&path, 1024, false)
        .await
        .unwrap();
    crate::core::engine::buffered_writer::platform_write_all_at(&file, 0, &[42; 128]).unwrap();
    drop(file);
    drop(
        prepare_sparse_direct_output_file(&path, 1024, true)
            .await
            .unwrap(),
    );
    assert_eq!(&tokio::fs::read(&path).await.unwrap()[..128], &[42; 128]);
    drop(
        prepare_sparse_direct_output_file(&path, 1024, false)
            .await
            .unwrap(),
    );
    assert_eq!(tokio::fs::read(&path).await.unwrap(), vec![0; 1024]);
    tokio::fs::remove_file(path).await.unwrap();
}

#[tokio::test]
async fn stalled_douyin_range_reopens_at_exact_offset_without_corruption() {
    let bytes = Arc::new((0..512 * 1024).map(|n| (n % 251) as u8).collect::<Vec<_>>());
    let attempts = Arc::new(AtomicUsize::new(0));
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let router = Router::new().route(
        "/video",
        get({
            let (bytes, attempts, ranges) = (bytes.clone(), attempts.clone(), ranges.clone());
            move |headers: HeaderMap| {
                let (bytes, attempts, ranges) = (bytes.clone(), attempts.clone(), ranges.clone());
                async move {
                    let range = headers["range"]
                        .to_str()
                        .unwrap()
                        .strip_prefix("bytes=")
                        .unwrap();
                    let (start, end) = range.split_once('-').unwrap();
                    let (start, end) = (
                        start.parse::<usize>().unwrap(),
                        end.parse::<usize>().unwrap(),
                    );
                    ranges.lock().unwrap().push(start);
                    let body = if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        Body::from_stream(
                            futures_util::stream::once(async move {
                                Ok::<_, std::io::Error>(bytes[start..start + 64 * 1024].to_vec())
                            })
                            .chain(futures_util::stream::pending()),
                        )
                    } else {
                        Body::from(bytes[start..=end].to_vec())
                    };
                    (
                        StatusCode::PARTIAL_CONTENT,
                        [(
                            "content-range",
                            format!("bytes {start}-{end}/{}", 512 * 1024),
                        )],
                        body,
                    )
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = Client::builder()
        .no_proxy()
        .resolve("fixture.douyinvod.com", addr)
        .build()
        .unwrap();
    let url = format!("http://fixture.douyinvod.com:{}/video", addr.port());
    let dir = std::env::temp_dir().join(format!(
        "nexus-range-recovery-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let output = dir.join("video.bin");
    let file = Arc::new(
        std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&output)
            .unwrap(),
    );
    file.set_len(bytes.len() as u64).unwrap();
    let store = Arc::new(crate::core::resume::ResumeStore::new(
        dir.join("resumes").to_str().unwrap(),
    ));
    store.init().await.unwrap();
    let mut manifest =
        crate::core::resume::ResumeManifest::new("recovery".into(), output.display().to_string());
    manifest.stages.insert(
        url.clone(),
        crate::core::resume::StageManifest {
            stage_id: url.clone(),
            url: url.clone(),
            source_fingerprint: String::new(),
            headers_hash: String::new(),
            total_size: bytes.len() as u64,
            chunks: build_direct_write_chunks(&output, bytes.len() as u64, 1),
            status: crate::core::resume::ChunkStatus::Pending,
        },
    );
    let manifest = Arc::new(tokio::sync::Mutex::new(manifest));
    let counter = Arc::new(AtomicU64::new(0));
    let token = tokio_util::sync::CancellationToken::new();
    let mut job = SegmentJob {
        index: 0,
        start: 0,
        end: bytes.len() as u64 - 1,
        bytes_written: 0,
        attempts: 0,
    };
    let started = std::time::Instant::now();
    let (status, saved) = fetch_range_segment(
        client.clone(),
        &url,
        HeaderMap::new(),
        file.clone(),
        counter.clone(),
        manifest.clone(),
        store.clone(),
        token.clone(),
        &job,
        2 * 1024 * 1024,
    )
    .await
    .unwrap();
    assert_eq!(status, SegmentFetchStatus::Recoverable);
    assert_eq!(saved, 64 * 1024);
    assert!(started.elapsed().as_secs() < 6);
    job.bytes_written = saved;
    let (status, _) = fetch_range_segment(
        client,
        &url,
        HeaderMap::new(),
        file.clone(),
        counter.clone(),
        manifest,
        store.clone(),
        token,
        &job,
        2 * 1024 * 1024,
    )
    .await
    .unwrap();
    assert_eq!(status, SegmentFetchStatus::Completed);
    assert_eq!(*ranges.lock().unwrap(), vec![0, 64 * 1024]);
    assert_eq!(counter.load(Ordering::Relaxed), bytes.len() as u64);
    assert_eq!(tokio::fs::read(&output).await.unwrap(), *bytes);
    drop(file);
    drop(store);
    server.abort();
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[test]
fn douyin_uses_short_leases_and_boundary_checked_host_matching() {
    let policy = range_segment_policy_for_url("https://v11-weba.douyinvod.com/media-video-1");
    assert_eq!(policy.kind, RangeSegmentPolicyKind::Douyin);
    assert_eq!(policy.lease_bytes, 2 * 1024 * 1024);
    assert_eq!(
        plan_range_segment_count_with_policy(57202016, 256, policy),
        14
    );
    assert_eq!(
        range_segment_policy_for_url("https://fakedouyinvod.com/file").kind,
        RangeSegmentPolicyKind::Generic
    );
}
