use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkManifest {
    pub index: u32,
    pub start: u64,
    pub end: u64,
    pub path: String,
    pub status: ChunkStatus,
    pub bytes_written: u64,
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ChunkStatus {
    Pending,
    Downloading,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageManifest {
    pub stage_id: String,
    pub url: String,
    #[serde(default)]
    pub source_fingerprint: String,
    #[serde(default)]
    pub headers_hash: String,
    pub total_size: u64,
    pub chunks: Vec<ChunkManifest>,
    pub status: ChunkStatus,
}

impl StageManifest {
    pub fn matches_source(&self, url: &str, total_size: u64, headers_hash: &str) -> bool {
        self.url == url
            && self.total_size == total_size
            && self.headers_hash == headers_hash
            && self.source_fingerprint == source_fingerprint(url, total_size, headers_hash)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeManifest {
    pub task_id: String,
    pub output_path: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub stages: HashMap<String, StageManifest>,
}

impl ResumeManifest {
    pub fn new(task_id: String, output_path: String) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            task_id,
            output_path,
            created_at: now,
            updated_at: now,
            stages: HashMap::new(),
        }
    }
}

pub fn headers_fingerprint(headers: &HashMap<String, String>) -> String {
    let mut pairs: Vec<(String, String)> = headers
        .iter()
        .filter_map(|(key, value)| {
            let lower = key.to_ascii_lowercase();
            if lower == "cookie" || lower == "authorization" || lower.ends_with("token") {
                None
            } else {
                Some((lower, value.clone()))
            }
        })
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    stable_fingerprint(&pairs)
}

pub fn source_fingerprint(url: &str, total_size: u64, headers_hash: &str) -> String {
    stable_fingerprint(&(url, total_size, headers_hash))
}

fn stable_fingerprint<T: Serialize>(value: &T) -> String {
    use std::hash::{Hash, Hasher};

    let serialized = serde_json::to_string(value).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serialized.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub struct ResumeStore {
    base_dir: std::path::PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_fingerprint_ignores_sensitive_headers() {
        let mut headers = HashMap::new();
        headers.insert("Referer".to_string(), "https://example.com".to_string());
        headers.insert("Cookie".to_string(), "secret=1".to_string());

        let with_cookie = headers_fingerprint(&headers);
        headers.insert("Cookie".to_string(), "secret=2".to_string());
        assert_eq!(with_cookie, headers_fingerprint(&headers));
    }

    #[test]
    fn stage_manifest_rejects_changed_source() {
        let headers_hash = "abc";
        let stage = StageManifest {
            stage_id: "stage".to_string(),
            url: "https://example.com/file.mp4".to_string(),
            source_fingerprint: source_fingerprint(
                "https://example.com/file.mp4",
                100,
                headers_hash,
            ),
            headers_hash: headers_hash.to_string(),
            total_size: 100,
            chunks: vec![],
            status: ChunkStatus::Pending,
        };

        assert!(stage.matches_source("https://example.com/file.mp4", 100, headers_hash));
        assert!(!stage.matches_source("https://example.com/file.mp4", 101, headers_hash));
        assert!(!stage.matches_source("https://example.com/file.mp4", 100, "changed"));
    }
}

impl ResumeStore {
    pub fn new(base_dir: &str) -> Self {
        Self {
            base_dir: std::path::PathBuf::from(base_dir),
        }
    }

    pub async fn init(&self) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(&self.base_dir).await?;
        Ok(())
    }

    pub async fn load_manifest(&self, task_id: &str) -> anyhow::Result<Option<ResumeManifest>> {
        let path = self.base_dir.join(format!("{}.manifest.json", task_id));
        if !path.exists() {
            return Ok(None);
        }
        let content = tokio::fs::read_to_string(&path).await?;
        let manifest: ResumeManifest = serde_json::from_str(&content)?;
        Ok(Some(manifest))
    }

    pub async fn save_manifest(&self, manifest: &ResumeManifest) -> anyhow::Result<()> {
        let path = self
            .base_dir
            .join(format!("{}.manifest.json", manifest.task_id));
        let content = serde_json::to_string_pretty(manifest)?;

        // Write to temp file then rename for atomicity
        let temp_path = self
            .base_dir
            .join(format!("{}.manifest.json.tmp", manifest.task_id));
        tokio::fs::write(&temp_path, content).await?;
        tokio::fs::rename(temp_path, path).await?;
        Ok(())
    }

    pub async fn delete_manifest(&self, task_id: &str) -> anyhow::Result<()> {
        for suffix in ["manifest.json", "manifest.json.tmp"] {
            let path = self.base_dir.join(format!("{task_id}.{suffix}"));
            match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}
