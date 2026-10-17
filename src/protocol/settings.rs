use serde::{Deserialize, Serialize};

use super::errors::TaskError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CategoryDownloadPaths {
    pub video: String,
    pub audio: String,
    pub document: String,
    pub archive: String,
    pub program: String,
    pub other: String,
}

impl Default for CategoryDownloadPaths {
    fn default() -> Self {
        Self {
            video: "./downloads/video".to_string(),
            audio: "./downloads/audio".to_string(),
            document: "./downloads/document".to_string(),
            archive: "./downloads/archive".to_string(),
            program: "./downloads/program".to_string(),
            other: "./downloads/other".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadDirectoryMode {
    Flat,
    Categorized,
    SystemDefault,
    Custom,
}

impl Default for DownloadDirectoryMode {
    fn default() -> Self {
        Self::Categorized
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppSettingsDto {
    pub download_folder: String,
    pub max_task_num: u32,
    pub max_chunk_connections: u32,
    pub enable_speed_limitation: bool,
    pub speed_limitation: u64,
    pub proxy_server: String,
    pub directory_mode: DownloadDirectoryMode,
    pub categorized_root: String,
    #[serde(alias = "category_paths")]
    pub custom_paths: CategoryDownloadPaths,
    pub system_paths: CategoryDownloadPaths,
}

impl Default for AppSettingsDto {
    fn default() -> Self {
        Self {
            download_folder: "./downloads".to_string(),
            max_task_num: 3,
            max_chunk_connections: crate::services::config::MAX_CHUNK_CONNECTIONS,
            enable_speed_limitation: false,
            speed_limitation: 4 * 1024 * 1024,
            proxy_server: "Auto".to_string(),
            directory_mode: DownloadDirectoryMode::default(),
            categorized_root: "./downloads".to_string(),
            custom_paths: CategoryDownloadPaths::default(),
            system_paths: CategoryDownloadPaths::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PatchAppSettingsDto {
    pub download_folder: Option<String>,
    pub max_task_num: Option<u32>,
    pub max_chunk_connections: Option<u32>,
    pub enable_speed_limitation: Option<bool>,
    pub speed_limitation: Option<u64>,
    pub proxy_server: Option<String>,
    pub directory_mode: Option<DownloadDirectoryMode>,
    pub categorized_root: Option<String>,
    #[serde(alias = "category_paths")]
    pub custom_paths: Option<PatchCategoryDownloadPaths>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PatchCategoryDownloadPaths {
    pub video: Option<String>,
    pub audio: Option<String>,
    pub document: Option<String>,
    pub archive: Option<String>,
    pub program: Option<String>,
    pub other: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsUpdateResp {
    pub settings: AppSettingsDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,
}
