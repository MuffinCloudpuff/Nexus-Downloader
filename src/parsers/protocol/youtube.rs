use crate::core::execution_plan::{
    ExecutionKind, ExecutionPlan, ExecutionStage, MergePolicy, OutputTarget, ResourceBudget,
    ResumePolicy, StageKind,
};
use crate::parsers::interfaces::{ProtocolParser, SiteResolution};
use anyhow::Result;
use async_trait::async_trait;

pub struct YoutubeProtocolParser;

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

#[async_trait]
impl ProtocolParser for YoutubeProtocolParser {
    fn id(&self) -> &'static str {
        "youtube"
    }

    fn priority(&self) -> u32 {
        90
    }

    fn matches(&self, resolution: &SiteResolution) -> bool {
        resolution.url.contains("youtube.com") || resolution.url.contains("youtu.be")
    }

    async fn parse(&self, task_id: &str, resolution: &SiteResolution) -> Result<ExecutionPlan> {
        let stage = ExecutionStage {
            stage_id: "youtube_page".to_string(),
            kind: StageKind::Video,
            url: sanitize_youtube_url(&resolution.url),
            headers: resolution.headers.clone(),
            expected_size: None,
            checksum: None,
            output_part: "youtube".to_string(),
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
            kind: ExecutionKind::YouTubeExternal,
            stages: vec![stage],
            resume_policy: ResumePolicy::Disabled,
            merge_policy: MergePolicy::None,
            resource_budget: ResourceBudget {
                max_connections: 1,
                max_bandwidth_bytes_per_sec: None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn youtube_protocol_uses_external_execution_kind() {
        let parser = YoutubeProtocolParser;
        let resolution = SiteResolution {
            url: "https://www.youtube.com/shorts/p2a-OwUcZEM?feature=share".to_string(),
            title: "Y-video".to_string(),
            headers: Default::default(),
            resources: vec![],
        };

        assert!(parser.matches(&resolution));
        let plan = parser.parse("task-youtube", &resolution).await.unwrap();

        assert_eq!(plan.kind, ExecutionKind::YouTubeExternal);
        assert_eq!(
            plan.stages[0].url,
            "https://www.youtube.com/shorts/p2a-OwUcZEM"
        );
    }
}
