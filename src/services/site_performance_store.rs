use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SitePerformanceRecord {
    pub site_key: String,
    pub last_effective_target: usize,
    pub last_effective_speed_bytes_per_sec: u64,
    pub sample_count: u64,
    pub last_sample_at_epoch_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SitePerformanceFile {
    records: HashMap<String, SitePerformanceRecord>,
}

pub struct SitePerformanceStore {
    path: PathBuf,
    records: RwLock<HashMap<String, SitePerformanceRecord>>,
}

static SITE_PERFORMANCE_STORE: OnceLock<Arc<SitePerformanceStore>> = OnceLock::new();

pub async fn init_site_performance_store(path: impl Into<PathBuf>) -> Arc<SitePerformanceStore> {
    let store = Arc::new(SitePerformanceStore::load(path).await);
    let _ = SITE_PERFORMANCE_STORE.set(store.clone());
    store
}

pub fn site_performance_store() -> Option<Arc<SitePerformanceStore>> {
    SITE_PERFORMANCE_STORE.get().cloned()
}

impl SitePerformanceStore {
    pub async fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let records = match tokio::fs::read_to_string(&path).await {
            Ok(text) => serde_json::from_str::<SitePerformanceFile>(&text)
                .map(|file| file.records)
                .unwrap_or_default(),
            Err(_) => HashMap::new(),
        };

        Self {
            path,
            records: RwLock::new(records),
        }
    }

    pub async fn get(&self, site_key: &str) -> Option<SitePerformanceRecord> {
        self.records.read().await.get(site_key).cloned()
    }

    pub async fn update_effective_target(
        &self,
        site_key: impl Into<String>,
        target: usize,
        speed_bytes_per_sec: u64,
    ) -> Result<SitePerformanceRecord> {
        let site_key = site_key.into();
        let mut guard = self.records.write().await;
        let sample_count = guard
            .get(&site_key)
            .map(|record| record.sample_count.saturating_add(1))
            .unwrap_or(1);
        let record = SitePerformanceRecord {
            site_key: site_key.clone(),
            last_effective_target: target.max(1),
            last_effective_speed_bytes_per_sec: speed_bytes_per_sec,
            sample_count,
            last_sample_at_epoch_ms: now_epoch_ms(),
        };
        guard.insert(site_key, record.clone());
        let snapshot = SitePerformanceFile {
            records: guard.clone(),
        };
        drop(guard);
        self.persist(&snapshot).await?;
        Ok(record)
    }

    async fn persist(&self, snapshot: &SitePerformanceFile) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let text = serde_json::to_string_pretty(snapshot).context("serialize site performance")?;
        tokio::fs::write(&self.path, text)
            .await
            .context("write site performance")?;
        Ok(())
    }
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn site_key_from_url(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_ascii_lowercase()))
        .unwrap_or_else(|| "invalid".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn store_updates_last_effective_target_without_preserving_old_maximum() {
        let root = std::env::temp_dir().join(format!("gd3-site-perf-test-{}", uuidish()));
        let path = root.join("site_performance.json");
        let store = SitePerformanceStore::load(&path).await;

        store
            .update_effective_target("cdn.example", 128, 100)
            .await
            .unwrap();
        let second = store
            .update_effective_target("cdn.example", 32, 50)
            .await
            .unwrap();

        assert_eq!(second.last_effective_target, 32);
        assert_eq!(second.sample_count, 2);

        let reloaded = SitePerformanceStore::load(&path).await;
        assert_eq!(
            reloaded
                .get("cdn.example")
                .await
                .unwrap()
                .last_effective_target,
            32
        );
    }

    #[test]
    fn site_key_uses_url_host() {
        assert_eq!(
            site_key_from_url("https://CDN1.TrashBytes.To/file.bin"),
            "cdn1.trashbytes.to"
        );
        assert_eq!(site_key_from_url("not a url"), "invalid");
    }

    fn uuidish() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
