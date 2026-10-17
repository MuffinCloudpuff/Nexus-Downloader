use crate::core::execution_plan::{
    ExecutionKind, ExecutionPlan, ExecutionStage, MergePolicy, OutputTarget, ResourceBudget,
    ResumePolicy, StageKind,
};
use crate::parsers::interfaces::{ProtocolParser, SiteResolution};
use crate::services::config::MAX_CHUNK_CONNECTIONS;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::{Client, StatusCode, header};
use std::time::Duration;

pub struct HttpFileProtocolParser;

#[derive(Debug, Clone, Copy, Default)]
struct HttpProbe {
    size: Option<u64>,
    supports_range: bool,
}

async fn probe_http_file(
    url: &str,
    headers: &std::collections::HashMap<String, String>,
) -> HttpProbe {
    let client = match Client::builder()
        .timeout(Duration::from_secs(15))
        .danger_accept_invalid_certs(true)
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(%error, "http file probe client build failed");
            return HttpProbe::default();
        }
    };

    let mut request = client
        .get(url)
        .header(header::RANGE, "bytes=0-0")
        .header(header::ACCEPT_ENCODING, "identity");
    for (key, value) in headers {
        request = request.header(key, value);
    }

    match request.send().await {
        Ok(response) => {
            let status = response.status();
            let headers = response.headers();
            let size_from_range = headers
                .get(header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(total_size_from_content_range);
            let size_from_length = headers
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            let supports_range = status == StatusCode::PARTIAL_CONTENT
                || headers
                    .get(header::ACCEPT_RANGES)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.to_ascii_lowercase().contains("bytes"));
            let size = if status.is_success() {
                size_from_range.or(size_from_length)
            } else {
                None
            };
            tracing::info!(
                url_host = %url_host_for_log(url),
                status = %status.as_u16(),
                size = ?size,
                supports_range,
                "http file probe completed"
            );
            HttpProbe {
                size,
                supports_range,
            }
        }
        Err(error) => {
            tracing::warn!(
                url_host = %url_host_for_log(url),
                %error,
                "http file probe failed; execution will probe again"
            );
            HttpProbe::default()
        }
    }
}

fn total_size_from_content_range(value: &str) -> Option<u64> {
    value.rsplit_once('/')?.1.trim().parse::<u64>().ok()
}

fn url_host_for_log(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_else(|| "invalid".to_string())
}

#[async_trait]
impl ProtocolParser for HttpFileProtocolParser {
    fn id(&self) -> &'static str {
        "http_file"
    }
    fn priority(&self) -> u32 {
        0
    } // Fallback

    fn matches(&self, resolution: &SiteResolution) -> bool {
        resolution.url.starts_with("http")
    }

    async fn parse(&self, task_id: &str, resolution: &SiteResolution) -> Result<ExecutionPlan> {
        let probe = probe_http_file(&resolution.url, &resolution.headers).await;
        let stage = ExecutionStage {
            stage_id: "file".to_string(),
            kind: StageKind::GenericFile,
            url: resolution.url.clone(),
            headers: resolution.headers.clone(),
            expected_size: probe.size,
            checksum: None,
            output_part: "file".to_string(),
        };

        Ok(ExecutionPlan {
            task_id: task_id.to_string(),
            request_id: None,
            trace_id: None,
            source: None,
            title: resolution.title.clone(),
            output_category: None,
            output: OutputTarget::File {
                path: format!("{task_id}.download"),
            },
            kind: ExecutionKind::HttpFile,
            stages: vec![stage],
            resume_policy: ResumePolicy::Chunked {
                chunk_size: 2 * 1024 * 1024,
            },
            merge_policy: MergePolicy::None,
            resource_budget: ResourceBudget {
                max_connections: if probe.supports_range {
                    MAX_CHUNK_CONNECTIONS
                } else {
                    1
                },
                max_bandwidth_bytes_per_sec: None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_total_size_from_content_range() {
        assert_eq!(
            total_size_from_content_range("bytes 0-0/64480785020"),
            Some(64480785020)
        );
        assert_eq!(total_size_from_content_range("bytes */123"), Some(123));
        assert_eq!(total_size_from_content_range("invalid"), None);
    }
}
