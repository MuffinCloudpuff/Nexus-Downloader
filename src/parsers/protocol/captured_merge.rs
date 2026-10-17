use crate::core::execution_plan::{
    ExecutionKind, ExecutionPlan, ExecutionStage, MergePolicy, OutputTarget, ResourceBudget,
    ResumePolicy, StageKind,
};
use crate::parsers::interfaces::{ProtocolParser, SiteResolution};
use crate::parsers::policy::output_naming;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;

pub struct CapturedMergeProtocolParser;

#[async_trait]
impl ProtocolParser for CapturedMergeProtocolParser {
    fn id(&self) -> &'static str {
        "captured_merge"
    }
    fn priority(&self) -> u32 {
        80
    }

    fn matches(&self, resolution: &SiteResolution) -> bool {
        // Just a basic check, if we have multiple resources it might be a merge task
        resolution.resources.len() > 1
    }

    async fn parse(&self, task_id: &str, resolution: &SiteResolution) -> Result<ExecutionPlan> {
        let mut stages = vec![];
        for (index, res) in resolution.resources.iter().enumerate() {
            let stage_id = res
                .kind
                .clone()
                .unwrap_or_else(|| format!("resource_{}", index));
            let url = if res.url.trim().is_empty() {
                anyhow::bail!("Missing url in resource")
            } else {
                res.url.clone()
            };
            let size = res.size.unwrap_or(0);

            let kind = match stage_id.as_str() {
                "video" => StageKind::Video,
                "audio" => StageKind::Audio,
                _ => StageKind::GenericFile,
            };
            stages.push(ExecutionStage {
                stage_id,
                kind,
                url,
                headers: stage_headers(&resolution.headers, &res.headers),
                expected_size: Some(size),
                checksum: None,
                output_part: format!("part_{}", index),
            });
        }

        Ok(ExecutionPlan {
            task_id: task_id.to_string(),
            request_id: None,
            trace_id: None,
            source: None,
            title: resolution.title.clone(),
            output_category: None,
            output: OutputTarget::File {
                path: output_naming::captured_merge_output_path(
                    &resolution.title,
                    &resolution.url,
                    task_id,
                ),
            },
            kind: ExecutionKind::DashMediaMerge, // Map to existing executor kind
            stages,
            resume_policy: ResumePolicy::Chunked {
                chunk_size: 2 * 1024 * 1024,
            },
            merge_policy: MergePolicy::FfmpegMux {
                video_stage: "video".to_string(),
                audio_stage: "audio".to_string(),
            },
            resource_budget: ResourceBudget {
                max_connections: 4,
                max_bandwidth_bytes_per_sec: None,
            },
        })
    }
}

fn stage_headers(
    resolution_headers: &HashMap<String, String>,
    resource_headers: &HashMap<String, String>,
) -> HashMap<String, String> {
    if resource_headers.is_empty() {
        return resolution_headers.clone();
    }

    let mut headers = resource_headers.clone();
    if !headers.keys().any(|key| key.eq_ignore_ascii_case("cookie"))
        && let Some((_, cookie)) = resolution_headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("cookie"))
    {
        headers.insert("Cookie".to_string(), cookie.clone());
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::task::CapturedResource;
    use std::collections::HashMap;

    #[tokio::test]
    async fn output_path_is_title_first_and_uses_bilibili_video_id() {
        let parser = CapturedMergeProtocolParser;
        let resolution = SiteResolution {
            url: "https://www.bilibili.com/video/BV15d7D6UE4i/?vd_source=abc".to_string(),
            title: "B-世界杯期间不抓移民？那这世界杯不是白办了吗".to_string(),
            headers: HashMap::new(),
            resources: vec![
                CapturedResource {
                    kind: Some("video".to_string()),
                    url: "https://example.com/video.m4s".to_string(),
                    ..Default::default()
                },
                CapturedResource {
                    kind: Some("audio".to_string()),
                    url: "https://example.com/audio.m4s".to_string(),
                    ..Default::default()
                },
            ],
        };

        let plan = parser
            .parse("req_page_media_344ade53", &resolution)
            .await
            .unwrap();

        match plan.output {
            OutputTarget::File { path } => {
                assert_eq!(
                    path,
                    "B-世界杯期间不抓移民？那这世界杯不是白办了吗-BV15d7D6UE4i.mp4"
                );
            }
            OutputTarget::Directory { .. } => panic!("captured merge should produce a file path"),
        }
    }

    #[test]
    fn resource_headers_keep_own_values_and_inherit_resolution_cookie() {
        let resolution_headers = HashMap::from([
            ("Cookie".to_string(), "SESSDATA=browser".to_string()),
            ("User-Agent".to_string(), "ua".to_string()),
        ]);
        let resource_headers = HashMap::from([(
            "Referer".to_string(),
            "https://www.bilibili.com".to_string(),
        )]);

        let headers = stage_headers(&resolution_headers, &resource_headers);

        assert_eq!(
            headers.get("Referer"),
            Some(&"https://www.bilibili.com".to_string())
        );
        assert_eq!(headers.get("Cookie"), Some(&"SESSDATA=browser".to_string()));
        assert_eq!(headers.get("User-Agent"), None);
    }
}
