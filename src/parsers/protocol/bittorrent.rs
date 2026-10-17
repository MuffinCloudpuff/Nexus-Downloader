use crate::core::execution_plan::{
    ExecutionKind, ExecutionPlan, ExecutionStage, MergePolicy, OutputTarget, ResourceBudget,
    ResumePolicy, StageKind,
};
use crate::parsers::interfaces::{ProtocolParser, SiteResolution};
use anyhow::Result;
use async_trait::async_trait;

pub struct BitTorrentProtocolParser;

#[async_trait]
impl ProtocolParser for BitTorrentProtocolParser {
    fn id(&self) -> &'static str {
        "bittorrent"
    }
    fn priority(&self) -> u32 {
        50
    }

    fn matches(&self, resolution: &SiteResolution) -> bool {
        resolution.url.starts_with("magnet:") || resolution.url.ends_with(".torrent")
    }

    async fn parse(&self, task_id: &str, resolution: &SiteResolution) -> Result<ExecutionPlan> {
        let stage = ExecutionStage {
            stage_id: "torrent_meta".to_string(),
            kind: StageKind::TorrentMeta,
            url: resolution.url.clone(),
            headers: resolution.headers.clone(),
            expected_size: None,
            checksum: None,
            output_part: "torrent".to_string(),
        };

        Ok(ExecutionPlan {
            task_id: task_id.to_string(),
            request_id: None,
            trace_id: None,
            source: None,
            title: resolution.title.clone(),
            output_category: None,
            output: OutputTarget::Directory {
                path: task_id.to_string(),
            },
            kind: ExecutionKind::BitTorrent,
            stages: vec![stage],
            resume_policy: ResumePolicy::Disabled, // BitTorrent has its own resume logic
            merge_policy: MergePolicy::None,
            resource_budget: ResourceBudget {
                max_connections: 50,
                max_bandwidth_bytes_per_sec: None,
            },
        })
    }
}
