use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::parsers::policy::output_category::OutputCategory;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    HttpFile,
    Hls,
    DashMediaMerge,
    CapturedMediaMerge,
    YouTubeExternal,
    BitTorrent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub task_id: String,
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
    pub source: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_category: Option<OutputCategory>,
    pub output: OutputTarget,
    pub kind: ExecutionKind,
    pub stages: Vec<ExecutionStage>,
    pub resume_policy: ResumePolicy,
    pub merge_policy: MergePolicy,
    pub resource_budget: ResourceBudget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum OutputTarget {
    File { path: String },
    Directory { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionStage {
    pub stage_id: String,
    pub kind: StageKind,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub expected_size: Option<u64>,
    pub checksum: Option<String>,
    pub output_part: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    Video,
    Audio,
    GenericFile,
    TorrentMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum MergePolicy {
    None,
    FfmpegMux {
        video_stage: String,
        audio_stage: String,
    },
    ConcatSegments,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ResumePolicy {
    Disabled,
    Chunked { chunk_size: u64 },
    ManifestBased,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceBudget {
    pub max_connections: u32,
    pub max_bandwidth_bytes_per_sec: Option<u64>,
}

impl ExecutionPlan {
    pub fn from_task(task: crate::core::models::Task) -> Self {
        use crate::core::models::{ExecutionKind as RuntimeExecutionKind, StageType};

        let kind = match task.resolved_execution_kind() {
            RuntimeExecutionKind::DashMediaMerge => ExecutionKind::DashMediaMerge,
            RuntimeExecutionKind::Youtube => ExecutionKind::YouTubeExternal,
            RuntimeExecutionKind::Hls => ExecutionKind::Hls,
            RuntimeExecutionKind::HttpFile => ExecutionKind::HttpFile,
            RuntimeExecutionKind::BitTorrent => ExecutionKind::BitTorrent,
            RuntimeExecutionKind::Other(_) => ExecutionKind::HttpFile,
        };

        let stages = task
            .stages
            .iter()
            .map(|stage| ExecutionStage {
                stage_id: format!("stage_{}", stage.stage_index),
                kind: match stage.stage_type {
                    StageType::Video => StageKind::Video,
                    StageType::Audio => StageKind::Audio,
                    StageType::HttpDownload
                    | StageType::File
                    | StageType::Merge
                    | StageType::Other(_) => StageKind::GenericFile,
                },
                url: stage.url.clone(),
                headers: stage.headers.clone(),
                expected_size: Some(stage.file_size).filter(|size| *size > 0),
                checksum: None,
                output_part: format!("part_{}", stage.stage_index),
            })
            .collect();

        Self {
            task_id: task.id.clone(),
            request_id: task.request_id.clone(),
            trace_id: task.trace_id.clone(),
            source: task.source.clone(),
            title: task.title.clone(),
            output_category: task.output_category.clone(),
            output: OutputTarget::File {
                path: task.path.clone(),
            },
            kind,
            stages,
            resume_policy: ResumePolicy::ManifestBased,
            merge_policy: MergePolicy::None,
            resource_budget: ResourceBudget {
                max_connections: 4,
                max_bandwidth_bytes_per_sec: None,
            },
        }
    }

    pub fn into_task(self, original_url: String) -> crate::core::models::Task {
        use crate::core::models::{PackId, Stage, StageType, Task};

        let mut total_size = 0;
        let mut task_stages = Vec::new();
        for (i, st) in self.stages.into_iter().enumerate() {
            total_size += st.expected_size.unwrap_or(0);
            let stage_type = match st.kind {
                StageKind::Video => StageType::Video,
                StageKind::Audio => StageType::Audio,
                _ => StageType::File,
            };
            task_stages.push(Stage {
                stage_index: i as u32,
                stage_type,
                url: st.url,
                file_size: st.expected_size.unwrap_or(0),
                headers: st.headers,
                block_num: 1,
            });
        }

        let pack_id = match self.kind {
            ExecutionKind::DashMediaMerge => PackId::BilibiliDash,
            ExecutionKind::CapturedMediaMerge => PackId::GenericMerge,
            ExecutionKind::YouTubeExternal => PackId::Youtube,
            ExecutionKind::Hls => PackId::Hls,
            ExecutionKind::HttpFile => PackId::Http,
            _ => PackId::Generic,
        };

        let path_str = match &self.output {
            OutputTarget::File { path } => path.clone(),
            OutputTarget::Directory { path } => path.clone(),
        };

        let mut task = Task::new(
            self.task_id,
            self.title,
            original_url,
            pack_id,
            total_size,
            path_str,
        );
        task.request_id = self.request_id;
        task.trace_id = self.trace_id;
        task.source = self.source;
        task.output_category = self.output_category;
        task.execution_kind = Some(match self.kind {
            ExecutionKind::DashMediaMerge => crate::core::models::ExecutionKind::DashMediaMerge,
            ExecutionKind::CapturedMediaMerge => crate::core::models::ExecutionKind::DashMediaMerge, // reuse Dash logic for generic merge
            ExecutionKind::YouTubeExternal => crate::core::models::ExecutionKind::Youtube,
            ExecutionKind::Hls => crate::core::models::ExecutionKind::Hls,
            ExecutionKind::HttpFile => crate::core::models::ExecutionKind::HttpFile,
            ExecutionKind::BitTorrent => crate::core::models::ExecutionKind::BitTorrent,
        });
        task.stages = task_stages;
        task
    }
}
