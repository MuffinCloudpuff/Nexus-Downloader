use crate::core::execution_plan::{
    ExecutionKind, ExecutionPlan, ExecutionStage, MergePolicy, OutputTarget, ResourceBudget,
    ResumePolicy, StageKind,
};
use crate::parsers::interfaces::{ProtocolParser, SiteResolution};
use anyhow::Result;
use async_trait::async_trait;

pub struct HlsProtocolParser;

#[async_trait]
impl ProtocolParser for HlsProtocolParser {
    fn id(&self) -> &'static str {
        "hls"
    }
    fn priority(&self) -> u32 {
        60
    }

    fn matches(&self, resolution: &SiteResolution) -> bool {
        resolution.url.ends_with(".m3u8") || resolution.url.contains(".m3u8?")
    }

    async fn parse(&self, task_id: &str, resolution: &SiteResolution) -> Result<ExecutionPlan> {
        let stage = ExecutionStage {
            stage_id: "playlist".to_string(),
            kind: StageKind::Video,
            url: resolution.url.clone(),
            headers: resolution.headers.clone(),
            expected_size: None,
            checksum: None,
            output_part: "playlist".to_string(),
        };

        Ok(ExecutionPlan {
            task_id: task_id.to_string(),
            request_id: None,
            trace_id: None,
            source: None,
            title: resolution.title.clone(),
            output_category: None,
            output: OutputTarget::File {
                path: format!("{task_id}.mp4"),
            },
            kind: ExecutionKind::Hls,
            stages: vec![stage],
            resume_policy: ResumePolicy::ManifestBased,
            merge_policy: MergePolicy::None,
            resource_budget: ResourceBudget {
                max_connections: 4,
                max_bandwidth_bytes_per_sec: None,
            },
        })
    }
}
