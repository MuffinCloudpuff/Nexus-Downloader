use serde::{Deserialize, Serialize};
use std::sync::{OnceLock, RwLock};

use crate::protocol::settings::AppSettingsDto;

pub const MIN_CHUNK_CONNECTIONS: u32 = 8;
pub const MAX_CHUNK_CONNECTIONS: u32 = 256;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub download_folder: String,
    pub max_task_num: u32,
    pub max_chunk_connections: u32,
    pub enable_speed_limitation: bool,
    pub speed_limitation: u64, // Bytes per second
    pub proxy_server: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            download_folder: "./downloads".to_string(),
            max_task_num: 3,
            max_chunk_connections: MAX_CHUNK_CONNECTIONS,
            enable_speed_limitation: false,
            speed_limitation: 4 * 1024 * 1024, // 默认 4MB/s
            proxy_server: "Auto".to_string(),
        }
    }
}

pub static CONFIG: OnceLock<RwLock<AppConfig>> = OnceLock::new();

pub fn get_config() -> AppConfig {
    CONFIG
        .get()
        .and_then(|config| config.read().ok().map(|guard| guard.clone()))
        .unwrap_or_default()
}

pub fn init_config(config: AppConfig) {
    let _ = CONFIG.set(RwLock::new(config));
}

pub fn update_config(config: AppConfig) {
    if let Some(existing) = CONFIG.get()
        && let Ok(mut guard) = existing.write()
    {
        *guard = config;
        return;
    }
    init_config(config);
}

impl From<AppSettingsDto> for AppConfig {
    fn from(settings: AppSettingsDto) -> Self {
        Self {
            download_folder: settings.download_folder,
            max_task_num: settings.max_task_num,
            max_chunk_connections: settings.max_chunk_connections,
            enable_speed_limitation: settings.enable_speed_limitation,
            speed_limitation: settings.speed_limitation,
            proxy_server: settings.proxy_server,
        }
    }
}
