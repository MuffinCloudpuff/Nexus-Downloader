#![allow(dead_code)]
use crate::core::interfaces::{CreateTaskRequest, FeaturePack};
use crate::core::models::{ExecutionKind, PackId, Stage, StageType, Task};
use crate::utils::to_safe_filename;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct BitTorrentPack;

impl Default for BitTorrentPack {
    fn default() -> Self {
        Self::new()
    }
}

impl BitTorrentPack {
    pub fn new() -> Self {
        Self
    }
}

// Bencode 数据结构映射 (简版，仅为了嗅探元信息)
#[derive(Debug, Deserialize)]
struct TorrentInfo {
    name: Option<String>,
    length: Option<u64>,
    files: Option<Vec<TorrentFile>>,
    #[serde(rename = "piece length")]
    piece_length: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TorrentFile {
    length: u64,
    path: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TorrentMeta {
    info: TorrentInfo,
}

#[async_trait]
impl FeaturePack for BitTorrentPack {
    fn pack_id(&self) -> PackId {
        PackId::Other("bittorrent".to_string())
    }

    fn priority(&self) -> u32 {
        80
    }

    fn matches(&self, req: &CreateTaskRequest) -> bool {
        req.url.starts_with("magnet:?xt=urn:btih:") || req.url.ends_with(".torrent")
    }

    async fn parse(&self, req: &CreateTaskRequest) -> Result<Task> {
        let url = &req.url;
        let path = "./downloads";

        let mut title = "bt_task".to_string();
        let mut total_size = 0;

        if url.ends_with(".torrent") {
            // 下载或读取本地种子文件
            let content = if url.starts_with("http") {
                reqwest::get(url).await?.bytes().await?.to_vec()
            } else {
                std::fs::read(url)?
            };

            // 使用 serde_bencode 静态反序列化种子文件，完全无惧恶意代码执行
            let meta: TorrentMeta = serde_bencode::from_bytes(&content)
                .map_err(|e| anyhow!("Failed to parse torrent file: {}", e))?;

            title = meta.info.name.unwrap_or_else(|| "bt_task".to_string());

            if let Some(files) = meta.info.files {
                for f in files {
                    total_size += f.length;
                }
            } else if let Some(len) = meta.info.length {
                total_size = len;
            }
        } else if url.starts_with("magnet:") {
            // Magnet link: 提取哈希作为临时文件名，真实大小依赖 BT 引擎的 Metadata 交换阶段
            if let Some(idx) = url.find("btih:") {
                let end = url[idx..].find('&').unwrap_or(url.len() - idx);
                title = url[idx + 5..idx + end].to_string();
            }
        }

        let safe_title = to_safe_filename(&title, "bt_task");
        let task_id = if req.request_id.is_empty() {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
                .to_string()
        } else {
            req.request_id.clone()
        };

        let mut task = Task::new(
            task_id,
            safe_title.clone(),
            url.to_string(),
            self.pack_id(),
            total_size,
            path.to_string(),
        );
        task.execution_kind = Some(ExecutionKind::BitTorrent);

        // 对于 BT 来说，整个种子在引擎层面作为一个独立的 Stage
        task.add_stage(Stage {
            stage_index: 1,
            stage_type: StageType::Other("bittorrent_download".to_string()),
            url: url.to_string(),
            file_size: total_size,
            headers: HashMap::new(),
            block_num: 1, // BT 协议内部由 Piece 机制管理切片，不需要 HTTP 的块机制
        });

        Ok(task)
    }
}
