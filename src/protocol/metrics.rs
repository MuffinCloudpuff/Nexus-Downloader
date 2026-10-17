use crate::protocol::errors::TaskError;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeMetrics {
    pub running_tasks: usize,
    pub queued_tasks: usize,
    pub total_speed_bytes_per_sec: u64,
    pub active_chunk_connections: Option<u32>,
    pub max_task_num: u32,
    pub max_chunk_connections: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoragePathStats {
    pub path: String,
    pub exists: bool,
    pub writable: bool,
    pub available_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub error: Option<TaskError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppVolumeStats {
    pub source_path: String,
    pub volume_root: String,
    pub available_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub error: Option<TaskError>,
}
