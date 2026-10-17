use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum TaskStatus {
    #[default]
    Queued,
    Parsing,
    Resuming,
    Downloading,
    Merging,
    Completed,
    Failed,
    Canceled,
    Paused,
    #[serde(untagged)]
    Other(String),
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            TaskStatus::Queued => "queued",
            TaskStatus::Parsing => "parsing",
            TaskStatus::Resuming => "resuming",
            TaskStatus::Downloading => "downloading",
            TaskStatus::Merging => "merging",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Canceled => "canceled",
            TaskStatus::Paused => "paused",
            TaskStatus::Other(value) => value.as_str(),
        };
        write!(f, "{}", value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ArtifactState {
    #[default]
    Unknown,
    Available,
    Missing,
    Moved,
}

impl std::fmt::Display for ArtifactState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            ArtifactState::Unknown => "unknown",
            ArtifactState::Available => "available",
            ArtifactState::Missing => "missing",
            ArtifactState::Moved => "moved",
        };
        write!(f, "{}", value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PackId {
    BilibiliDash,
    Youtube,
    Hls,
    Generic,
    GenericMerge,
    Http,
    #[serde(untagged)]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StageType {
    Video,
    Audio,
    Merge,
    File,
    HttpDownload,
    #[serde(untagged)]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    DashMediaMerge,
    Youtube,
    Hls,
    HttpFile,
    BitTorrent,
    #[serde(untagged)]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub title: String,
    pub url: String,
    pub pack_id: PackId,
    pub file_size: u64,
    pub path: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub artifact_state: ArtifactState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_category: Option<crate::parsers::policy::output_category::OutputCategory>,
    pub stages: Vec<Stage>,
    #[serde(skip_serializing, skip_deserializing)]
    pub cookie_str: Option<String>,
    #[serde(default)]
    pub execution_kind: Option<ExecutionKind>,
    // 运行时状态字段（部分将逐步被事件替代，但为兼容快照保留）
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default)]
    pub progress: f64, // 0.0 ~ 1.0
    #[serde(default)]
    pub speed: u64, // bytes/sec
    #[serde(default)]
    pub received_bytes: u64,
    // 10分架构新增：事件游标和存储
    #[serde(default)]
    pub event_cursor: u64,
    #[serde(default)]
    pub events: Vec<TaskEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stage {
    pub stage_index: u32,
    pub stage_type: StageType,
    pub url: String,
    pub file_size: u64,
    pub headers: HashMap<String, String>,
    pub block_num: u32,
}

impl Task {
    pub fn new(
        id: String,
        title: String,
        url: String,
        pack_id: PackId,
        file_size: u64,
        path: String,
    ) -> Self {
        Self {
            id,
            request_id: None,
            trace_id: None,
            payload_fingerprint: None,
            source: None,
            title,
            url,
            pack_id,
            file_size,
            path,
            created_at: chrono::Utc::now().to_rfc3339(),
            artifact_state: ArtifactState::Unknown,
            output_category: None,
            stages: Vec::new(),
            cookie_str: None,
            execution_kind: None,
            status: TaskStatus::Queued,
            progress: 0.0,
            speed: 0,
            received_bytes: 0,
            event_cursor: 0,
            events: Vec::new(),
        }
    }

    pub fn add_stage(&mut self, stage: Stage) {
        self.stages.push(stage);
    }

    pub fn with_execution_kind(mut self, execution_kind: ExecutionKind) -> Self {
        self.execution_kind = Some(execution_kind);
        self
    }

    pub fn resolved_execution_kind(&self) -> ExecutionKind {
        self.execution_kind
            .clone()
            .unwrap_or_else(|| default_execution_kind(&self.pack_id))
    }
}

fn default_execution_kind(pack_id: &PackId) -> ExecutionKind {
    match pack_id {
        PackId::BilibiliDash | PackId::GenericMerge => ExecutionKind::DashMediaMerge,
        PackId::Youtube => ExecutionKind::Youtube,
        PackId::Hls => ExecutionKind::Hls,
        PackId::Other(value) if value == "bittorrent" => ExecutionKind::BitTorrent,
        PackId::Generic | PackId::Http | PackId::Other(_) => ExecutionKind::HttpFile,
    }
}

// ==========================================
// 10分架构：事件模型协议
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskEventPayload {
    TaskAccepted,
    TaskQueued,
    TaskParsingStarted,
    TaskResumingStarted,
    TaskStageChanged {
        stage: String,
    },
    TaskDownloadProgressUpdated {
        stage: String,
        progress: f64,
        received_bytes: u64,
        total_bytes: u64,
        speed_bytes_per_sec: u64,
    },
    TaskMergeStarted,
    TaskCompleted {
        output_path: String,
    },
    TaskArtifactAvailable {
        output_path: String,
    },
    TaskArtifactMissing {
        output_path: String,
    },
    TaskArtifactMoved {
        old_path: String,
        new_path: String,
    },
    TaskFailed {
        failure: ErrorPayload,
    },
    TaskCanceled,
    TaskDeleted,
    TaskPaused,
    TaskResumed,
    TaskRetried {
        previous_status: TaskStatus,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEvent {
    pub event_id: u64,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    pub occurred_at: String, // ISO8601
    #[serde(flatten)]
    pub payload: TaskEventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
}
