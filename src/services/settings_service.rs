use crate::protocol::settings::{AppSettingsDto, PatchAppSettingsDto};
use crate::services::config::{
    AppConfig, MAX_CHUNK_CONNECTIONS, MIN_CHUNK_CONNECTIONS, update_config,
};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::sync::{RwLock, broadcast};

pub struct SettingsService {
    path: PathBuf,
    settings: RwLock<AppSettingsDto>,
    change_tx: broadcast::Sender<AppSettingsDto>,
}

impl SettingsService {
    pub async fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let settings = match tokio::fs::read_to_string(&path).await {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => AppSettingsDto::default(),
        };

        let settings = normalize_settings(settings);
        update_config(AppConfig::from(settings.clone()));

        let (change_tx, _) = broadcast::channel(16);

        Self {
            path,
            settings: RwLock::new(settings),
            change_tx,
        }
    }

    pub async fn get(&self) -> AppSettingsDto {
        self.settings.read().await.clone()
    }

    pub async fn patch(&self, patch: PatchAppSettingsDto) -> Result<AppSettingsDto> {
        let mut guard = self.settings.write().await;
        let mut candidate = guard.clone();
        apply_patch(&mut candidate, patch);
        candidate = normalize_settings(candidate);
        validate_settings_paths(&candidate).await?;
        persist_settings(&self.path, &candidate).await?;
        update_config(AppConfig::from(candidate.clone()));
        *guard = candidate.clone();
        let _ = self.change_tx.send(candidate.clone());
        Ok(candidate)
    }

    pub fn subscribe_changes(&self) -> broadcast::Receiver<AppSettingsDto> {
        self.change_tx.subscribe()
    }
}

fn apply_patch(settings: &mut AppSettingsDto, patch: PatchAppSettingsDto) {
    if let Some(value) = patch.download_folder {
        settings.download_folder = value;
    }
    if let Some(value) = patch.max_task_num {
        settings.max_task_num = value;
    }
    if let Some(value) = patch.max_chunk_connections {
        settings.max_chunk_connections = value;
    }
    if let Some(value) = patch.enable_speed_limitation {
        settings.enable_speed_limitation = value;
    }
    if let Some(value) = patch.speed_limitation {
        settings.speed_limitation = value;
    }
    if let Some(value) = patch.proxy_server {
        settings.proxy_server = value;
    }
    if let Some(value) = patch.categorized_root {
        settings.categorized_root = value;
    }
    if let Some(value) = patch.directory_mode {
        settings.directory_mode = value.clone();
        if value == crate::protocol::settings::DownloadDirectoryMode::SystemDefault {
            if let Some(user_dirs) = directories::UserDirs::new() {
                if let Some(path) = user_dirs.video_dir() {
                    settings.system_paths.video = resolve_canonical_path(path.to_path_buf());
                }
                if let Some(path) = user_dirs.audio_dir() {
                    settings.system_paths.audio = resolve_canonical_path(path.to_path_buf());
                }
                if let Some(path) = user_dirs.picture_dir() {
                    settings.system_paths.image = resolve_canonical_path(path.to_path_buf());
                }
                if let Some(path) = user_dirs.document_dir() {
                    settings.system_paths.document = resolve_canonical_path(path.to_path_buf());
                }
                if let Some(path) = user_dirs.download_dir() {
                    let dl = resolve_canonical_path(path.to_path_buf());
                    settings.system_paths.archive = dl.clone();
                    settings.system_paths.program = dl.clone();
                    settings.system_paths.other = dl;
                }
            }
        }
    }
    if let Some(paths) = patch.custom_paths {
        if let Some(value) = paths.image {
            settings.custom_paths.image = value;
        }
        if let Some(value) = paths.video {
            settings.custom_paths.video = value;
        }
        if let Some(value) = paths.audio {
            settings.custom_paths.audio = value;
        }
        if let Some(value) = paths.document {
            settings.custom_paths.document = value;
        }
        if let Some(value) = paths.archive {
            settings.custom_paths.archive = value;
        }
        if let Some(value) = paths.program {
            settings.custom_paths.program = value;
        }
        if let Some(value) = paths.other {
            settings.custom_paths.other = value;
        }
    }
}

fn normalize_settings(mut settings: AppSettingsDto) -> AppSettingsDto {
    let defaults = AppSettingsDto::default();
    if settings.download_folder.trim().is_empty() {
        settings.download_folder = defaults.download_folder;
    }
    settings.max_task_num = settings.max_task_num.max(1);
    settings.max_chunk_connections = settings
        .max_chunk_connections
        .clamp(MIN_CHUNK_CONNECTIONS, MAX_CHUNK_CONNECTIONS);
    settings.speed_limitation = settings.speed_limitation.max(1);

    if settings.categorized_root.trim().is_empty() {
        settings.categorized_root = defaults.categorized_root;
    }

    normalize_path(
        &mut settings.custom_paths.video,
        &defaults.custom_paths.video,
    );
    normalize_path(
        &mut settings.custom_paths.audio,
        &defaults.custom_paths.audio,
    );
    normalize_path(
        &mut settings.custom_paths.image,
        &defaults.custom_paths.image,
    );
    normalize_path(
        &mut settings.custom_paths.document,
        &defaults.custom_paths.document,
    );
    normalize_path(
        &mut settings.custom_paths.archive,
        &defaults.custom_paths.archive,
    );
    normalize_path(
        &mut settings.custom_paths.program,
        &defaults.custom_paths.program,
    );
    normalize_path(
        &mut settings.custom_paths.other,
        &defaults.custom_paths.other,
    );

    settings
}

fn normalize_path(value: &mut String, fallback: &str) {
    if value.trim().is_empty() {
        *value = fallback.to_string();
    }
}

fn resolve_canonical_path(p: std::path::PathBuf) -> String {
    if let Ok(canon) = std::fs::canonicalize(&p) {
        let s = canon.to_string_lossy().to_string();
        if s.starts_with(r"\\?\") {
            s[4..].to_string()
        } else {
            s
        }
    } else {
        p.to_string_lossy().to_string()
    }
}

async fn persist_settings(path: &Path, settings: &AppSettingsDto) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let text = serde_json::to_string_pretty(settings).context("serialize settings")?;
    tokio::fs::write(path, text)
        .await
        .context("write settings")?;
    Ok(())
}

async fn validate_settings_paths(settings: &AppSettingsDto) -> Result<()> {
    validate_directory("download_folder", &settings.download_folder).await?;
    validate_directory("categorized_root", &settings.categorized_root).await?;
    validate_directory("custom_paths.video", &settings.custom_paths.video).await?;
    validate_directory("custom_paths.audio", &settings.custom_paths.audio).await?;
    validate_directory("custom_paths.image", &settings.custom_paths.image).await?;
    validate_directory("custom_paths.document", &settings.custom_paths.document).await?;
    validate_directory("custom_paths.archive", &settings.custom_paths.archive).await?;
    validate_directory("custom_paths.program", &settings.custom_paths.program).await?;
    validate_directory("custom_paths.other", &settings.custom_paths.other).await?;
    Ok(())
}

async fn validate_directory(label: &str, value: &str) -> Result<()> {
    let path = Path::new(value);
    tokio::fs::create_dir_all(path)
        .await
        .with_context(|| format!("create settings directory {label}"))?;
    let metadata = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("read settings directory metadata {label}"))?;
    if !metadata.is_dir() {
        anyhow::bail!("settings path {label} is not a directory");
    }

    let probe_path = path.join(".gd3-write-probe");
    tokio::fs::write(&probe_path, b"ok")
        .await
        .with_context(|| format!("write settings directory probe {label}"))?;
    let _ = tokio::fs::remove_file(&probe_path).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn patch_clamps_chunk_connections() {
        let root = std::env::temp_dir().join(format!("gd3-settings-test-{}", uuidish()));
        let service = SettingsService::load(root.join("settings.json")).await;
        let settings = service
            .patch(PatchAppSettingsDto {
                max_chunk_connections: Some(999),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(settings.max_chunk_connections, MAX_CHUNK_CONNECTIONS);
    }

    #[tokio::test]
    async fn default_chunk_connection_ceiling_is_maximum_for_adaptive_tuning() {
        let root = std::env::temp_dir().join(format!("gd3-settings-test-{}", uuidish()));
        let service = SettingsService::load(root.join("settings.json")).await;
        let settings = service.get().await;

        assert_eq!(settings.max_chunk_connections, MAX_CHUNK_CONNECTIONS);
    }

    #[tokio::test]
    async fn patch_raises_chunk_connections_to_minimum() {
        let root = std::env::temp_dir().join(format!("gd3-settings-test-{}", uuidish()));
        let service = SettingsService::load(root.join("settings.json")).await;
        let settings = service
            .patch(PatchAppSettingsDto {
                max_chunk_connections: Some(1),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(settings.max_chunk_connections, MIN_CHUNK_CONNECTIONS);
    }

    #[tokio::test]
    async fn patch_rejects_file_path_before_mutating_settings() {
        let root = std::env::temp_dir().join(format!("gd3-settings-test-{}", uuidish()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let file_path = root.join("not-a-directory");
        tokio::fs::write(&file_path, b"file").await.unwrap();
        let service = SettingsService::load(root.join("settings.json")).await;
        let before = service.get().await;

        let result = service
            .patch(PatchAppSettingsDto {
                custom_paths: Some(crate::protocol::settings::PatchCategoryDownloadPaths {
                    video: Some(file_path.to_string_lossy().to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .await;

        assert!(result.is_err());
        assert_eq!(service.get().await, before);
    }

    fn uuidish() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
