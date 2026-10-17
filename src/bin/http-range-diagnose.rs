use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::{Client, Version, header};
use serde::Serialize;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, BufWriter};

#[derive(Clone, Copy, Debug)]
enum HttpMode {
    Default,
    Http1Only,
}

#[derive(Clone, Copy, Debug)]
enum ClientMode {
    Shared,
    PerWorker,
}

#[derive(Clone, Copy, Debug)]
enum WriteMode {
    Null,
    Direct,
    Part,
}

#[derive(Clone, Debug)]
struct Config {
    url: String,
    headers_path: Option<PathBuf>,
    output: PathBuf,
    workers: usize,
    http_mode: HttpMode,
    client_mode: ClientMode,
    write_mode: WriteMode,
    byte_limit: Option<u64>,
    start_offset: Option<u64>,
    tail_bytes: Option<u64>,
    worker_timeout_ms: u64,
    chunk_bytes: usize,
}

struct WorkerContext {
    worker_id: usize,
    client: Client,
    url: String,
    headers: header::HeaderMap,
    output: PathBuf,
    write_mode: WriteMode,
    chunk_bytes: usize,
    range: RangePlan,
    total_downloaded: Arc<AtomicU64>,
}

#[derive(Debug)]
struct RangePlan {
    start: u64,
    end: u64,
}

#[derive(Debug, Serialize)]
struct WorkerSummary {
    worker_id: usize,
    range_start: u64,
    range_end: u64,
    http_version: String,
    status: String,
    first_byte_ms: u128,
    read_bytes: u64,
    read_ms: u128,
    write_ms: u128,
    bps: f64,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct RunSummary {
    workers: usize,
    http_mode: String,
    client_mode: String,
    write_mode: String,
    total_size: u64,
    range_start: u64,
    planned_bytes: u64,
    downloaded_bytes: u64,
    elapsed_ms: u128,
    aggregate_bps: f64,
    worker_summaries: Vec<WorkerSummary>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env()?;
    let headers = load_headers(config.headers_path.as_deref()).await?;
    let client = build_client(config.http_mode)?;
    let total_size = probe_size(&client, &config.url, &headers).await?;
    let (range_start, planned_size) = selected_range_window(&config, total_size)?;
    let ranges = build_ranges(range_start, planned_size, config.workers);

    prepare_output(&config, planned_size).await?;

    let started = Instant::now();
    let total_downloaded = Arc::new(AtomicU64::new(0));
    let mut join_set = tokio::task::JoinSet::new();

    for (worker_id, range) in ranges.iter().enumerate() {
        let worker_client = match config.client_mode {
            ClientMode::Shared => client.clone(),
            ClientMode::PerWorker => build_client(config.http_mode)?,
        };
        let url = config.url.clone();
        let headers = headers.clone();
        let output = config.output.clone();
        let write_mode = config.write_mode;
        let chunk_bytes = config.chunk_bytes;
        let worker_timeout_ms = config.worker_timeout_ms;
        let total_downloaded = total_downloaded.clone();
        let range = RangePlan {
            start: range.start,
            end: range.end,
        };
        let range_start = range.start;
        let range_end = range.end;

        join_set.spawn(async move {
            match tokio::time::timeout(
                Duration::from_millis(worker_timeout_ms),
                run_worker(WorkerContext {
                    worker_id,
                    client: worker_client,
                    url,
                    headers,
                    output,
                    write_mode,
                    chunk_bytes,
                    range,
                    total_downloaded,
                }),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Ok(WorkerSummary {
                    worker_id,
                    range_start,
                    range_end,
                    http_version: "timeout".to_string(),
                    status: "timeout".to_string(),
                    first_byte_ms: worker_timeout_ms as u128,
                    read_bytes: 0,
                    read_ms: worker_timeout_ms as u128,
                    write_ms: 0,
                    bps: 0.0,
                    error: Some(format!("worker timed out after {worker_timeout_ms} ms")),
                }),
            }
        });
    }

    let mut worker_summaries = Vec::new();
    while let Some(result) = join_set.join_next().await {
        worker_summaries.push(result??);
    }
    worker_summaries.sort_by_key(|summary| summary.worker_id);

    let elapsed = started.elapsed();
    let downloaded = total_downloaded.load(Ordering::Relaxed);
    let summary = RunSummary {
        workers: config.workers,
        http_mode: format!("{:?}", config.http_mode),
        client_mode: format!("{:?}", config.client_mode),
        write_mode: format!("{:?}", config.write_mode),
        total_size,
        range_start,
        planned_bytes: planned_size,
        downloaded_bytes: downloaded,
        elapsed_ms: elapsed.as_millis(),
        aggregate_bps: bytes_per_second(downloaded, elapsed),
        worker_summaries,
    };

    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

impl Config {
    fn from_env() -> Result<Self> {
        let url = env::var("NEXUS_DIAG_URL").context("NEXUS_DIAG_URL is required")?;
        let headers_path = env::var("NEXUS_DIAG_HEADERS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        let output = env::var("NEXUS_DIAG_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("target/http-range-diagnose.bin"));
        let workers = parse_env("NEXUS_DIAG_WORKERS", 16usize)?;
        let byte_limit = parse_optional_bytes("NEXUS_DIAG_LIMIT_BYTES")?;
        let start_offset = parse_optional_bytes("NEXUS_DIAG_START_OFFSET")?;
        let tail_bytes = parse_optional_bytes("NEXUS_DIAG_TAIL_BYTES")?;
        let worker_timeout_ms = parse_env("NEXUS_DIAG_WORKER_TIMEOUT_MS", 60_000_u64)?;
        let chunk_bytes = parse_env("NEXUS_DIAG_CHUNK_BYTES", 64 * 1024usize)?;

        Ok(Self {
            url,
            headers_path,
            output,
            workers: workers.max(1),
            http_mode: parse_http_mode(&env::var("NEXUS_DIAG_HTTP_MODE").unwrap_or_default())?,
            client_mode: parse_client_mode(
                &env::var("NEXUS_DIAG_CLIENT_MODE").unwrap_or_default(),
            )?,
            write_mode: parse_write_mode(&env::var("NEXUS_DIAG_WRITE_MODE").unwrap_or_default())?,
            byte_limit,
            start_offset,
            tail_bytes,
            worker_timeout_ms,
            chunk_bytes: chunk_bytes.max(4096),
        })
    }
}

fn parse_env<T>(name: &str, default: T) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value.parse()?),
        _ => Ok(default),
    }
}

fn parse_optional_bytes(name: &str) -> Result<Option<u64>> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(Some(value.parse()?)),
        _ => Ok(None),
    }
}

fn selected_range_window(config: &Config, total_size: u64) -> Result<(u64, u64)> {
    if config.start_offset.is_some() && config.tail_bytes.is_some() {
        bail!("NEXUS_DIAG_START_OFFSET and NEXUS_DIAG_TAIL_BYTES are mutually exclusive");
    }

    let planned_size = config
        .tail_bytes
        .or(config.byte_limit)
        .unwrap_or(total_size)
        .min(total_size);

    let start = if let Some(tail_bytes) = config.tail_bytes {
        total_size.saturating_sub(tail_bytes.min(total_size))
    } else {
        config.start_offset.unwrap_or(0)
    };

    if start >= total_size {
        bail!("diagnostic range start {start} is outside total size {total_size}");
    }

    let available = total_size - start;
    Ok((start, planned_size.min(available)))
}

fn parse_http_mode(value: &str) -> Result<HttpMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "default" => Ok(HttpMode::Default),
        "http1" | "http1_only" => Ok(HttpMode::Http1Only),
        other => bail!("unsupported NEXUS_DIAG_HTTP_MODE: {other}"),
    }
}

fn parse_client_mode(value: &str) -> Result<ClientMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "shared" => Ok(ClientMode::Shared),
        "per_worker" | "per-worker" => Ok(ClientMode::PerWorker),
        other => bail!("unsupported NEXUS_DIAG_CLIENT_MODE: {other}"),
    }
}

fn parse_write_mode(value: &str) -> Result<WriteMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "null" => Ok(WriteMode::Null),
        "direct" => Ok(WriteMode::Direct),
        "part" | "part_file" | "part-file" => Ok(WriteMode::Part),
        other => bail!("unsupported NEXUS_DIAG_WRITE_MODE: {other}"),
    }
}

fn build_client(http_mode: HttpMode) -> Result<Client> {
    let mut builder = Client::builder()
        .no_gzip()
        .no_brotli()
        .no_zstd()
        .no_deflate()
        .danger_accept_invalid_certs(true);

    if matches!(http_mode, HttpMode::Http1Only) {
        builder = builder.http1_only();
    }

    Ok(builder.build()?)
}

async fn load_headers(path: Option<&Path>) -> Result<header::HeaderMap> {
    let Some(path) = path else {
        return Ok(header::HeaderMap::new());
    };
    let text = tokio::fs::read_to_string(path).await?;
    let source: HashMap<String, String> = serde_json::from_str(&text)?;
    let mut headers = header::HeaderMap::new();

    for (key, value) in source {
        let name = header::HeaderName::from_bytes(key.as_bytes())?;
        let value = header::HeaderValue::from_str(&value)?;
        headers.insert(name, value);
    }

    Ok(headers)
}

async fn probe_size(client: &Client, url: &str, headers: &header::HeaderMap) -> Result<u64> {
    let mut probe_headers = headers.clone();
    insert_range_headers(&mut probe_headers, 0, 0)?;

    let res = client.get(url).headers(probe_headers).send().await?;
    if res.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        bail!("range probe returned {}, expected 206", res.status());
    }

    res.headers()
        .get(header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit_once('/'))
        .and_then(|(_, total)| total.parse::<u64>().ok())
        .context("range probe did not return parseable Content-Range")
}

fn insert_range_headers(headers: &mut header::HeaderMap, start: u64, end: u64) -> Result<()> {
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

fn build_ranges(start_offset: u64, total: u64, workers: usize) -> Vec<RangePlan> {
    let workers = workers.min(total.max(1) as usize);
    let base = total / workers as u64;
    let remainder = total % workers as u64;
    let mut start = start_offset;
    let mut ranges = Vec::with_capacity(workers);

    for index in 0..workers {
        let len = base + u64::from((index as u64) < remainder);
        let end = start + len - 1;
        ranges.push(RangePlan { start, end });
        start = end + 1;
    }

    ranges
}

async fn prepare_output(config: &Config, planned_size: u64) -> Result<()> {
    if let Some(parent) = config.output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    match config.write_mode {
        WriteMode::Null => {}
        WriteMode::Direct => {
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(true)
                .open(&config.output)
                .await?;
            file.set_len(planned_size).await?;
        }
        WriteMode::Part => {
            let part_dir = part_dir(&config.output);
            let _ = tokio::fs::remove_dir_all(&part_dir).await;
            tokio::fs::create_dir_all(part_dir).await?;
        }
    }

    Ok(())
}

async fn run_worker(ctx: WorkerContext) -> Result<WorkerSummary> {
    let WorkerContext {
        worker_id,
        client,
        url,
        mut headers,
        output,
        write_mode,
        chunk_bytes,
        range,
        total_downloaded,
    } = ctx;
    insert_range_headers(&mut headers, range.start, range.end)?;
    let request_started = Instant::now();
    let res = client.get(url).headers(headers).send().await?;
    let status = res.status();
    let version = version_name(res.version()).to_string();

    if status != reqwest::StatusCode::PARTIAL_CONTENT {
        let read_time = request_started.elapsed();
        return Ok(WorkerSummary {
            worker_id,
            range_start: range.start,
            range_end: range.end,
            http_version: version,
            status: status.as_u16().to_string(),
            first_byte_ms: read_time.as_millis(),
            read_bytes: 0,
            read_ms: read_time.as_millis(),
            write_ms: 0,
            bps: 0.0,
            error: Some(format!("expected 206, got {status}")),
        });
    }

    let mut stream = res.bytes_stream();
    let mut read_bytes = 0u64;
    let mut write_time = Duration::ZERO;
    let mut first_byte_ms = None;
    let mut direct_file = match write_mode {
        WriteMode::Direct => Some(OpenOptions::new().write(true).open(&output).await?),
        _ => None,
    };
    let mut part_file = match write_mode {
        WriteMode::Part => {
            let path = part_dir(&output).join(format!("chunk_{worker_id:03}.part"));
            Some(BufWriter::with_capacity(
                1024 * 1024,
                File::create(path).await?,
            ))
        }
        _ => None,
    };
    let mut next_offset = range.start;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if first_byte_ms.is_none() {
            first_byte_ms = Some(request_started.elapsed().as_millis());
        }

        for bytes in chunk.chunks(chunk_bytes) {
            let write_started = Instant::now();
            match write_mode {
                WriteMode::Null => {}
                WriteMode::Direct => {
                    let file = direct_file.as_mut().context("direct file missing")?;
                    file.seek(std::io::SeekFrom::Start(next_offset)).await?;
                    file.write_all(bytes).await?;
                }
                WriteMode::Part => {
                    let file = part_file.as_mut().context("part file missing")?;
                    file.write_all(bytes).await?;
                }
            }
            write_time += write_started.elapsed();
            next_offset += bytes.len() as u64;
            read_bytes += bytes.len() as u64;
            total_downloaded.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        }
    }

    if let Some(file) = direct_file.as_mut() {
        let write_started = Instant::now();
        file.flush().await?;
        write_time += write_started.elapsed();
    }
    if let Some(file) = part_file.as_mut() {
        let write_started = Instant::now();
        file.flush().await?;
        write_time += write_started.elapsed();
    }

    let read_time = request_started.elapsed();
    Ok(WorkerSummary {
        worker_id,
        range_start: range.start,
        range_end: range.end,
        http_version: version,
        status: status.as_u16().to_string(),
        first_byte_ms: first_byte_ms.unwrap_or_else(|| request_started.elapsed().as_millis()),
        read_bytes,
        read_ms: read_time.as_millis(),
        write_ms: write_time.as_millis(),
        bps: bytes_per_second(read_bytes, read_time),
        error: None,
    })
}

fn part_dir(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("http-range-diagnose");
    output.with_file_name(format!("{name}.parts"))
}

fn version_name(version: Version) -> &'static str {
    match version {
        Version::HTTP_09 => "HTTP/0.9",
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_11 => "HTTP/1.1",
        Version::HTTP_2 => "HTTP/2",
        Version::HTTP_3 => "HTTP/3",
        _ => "unknown",
    }
}

fn bytes_per_second(bytes: u64, elapsed: Duration) -> f64 {
    let secs = elapsed.as_secs_f64();
    if secs <= 0.0 {
        0.0
    } else {
        bytes as f64 / secs
    }
}
