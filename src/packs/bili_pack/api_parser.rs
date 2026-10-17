use crate::core::interfaces::{CreateTaskRequest, FeaturePack};
use crate::core::models::{ExecutionKind, PackId, Stage, StageType, Task};
use crate::services::bilibili_cookie_store;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use tracing::info;

pub struct BiliApiParser {
    client: Client,
}

impl Default for BiliApiParser {
    fn default() -> Self {
        Self::new()
    }
}

impl BiliApiParser {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    fn extract_bvid(&self, url: &str) -> Option<String> {
        let parts: Vec<&str> = url.split("/video/").collect();
        if parts.len() > 1 {
            let bvid_part = parts[1].split('?').next()?;
            let bvid_clean = bvid_part.trim_matches('/');
            return Some(bvid_clean.to_string());
        }
        None
    }

    fn extract_stream_url(&self, stream: &Value) -> Option<String> {
        if let Some(base_url) = stream["baseUrl"].as_str() {
            return Some(base_url.to_string());
        }
        if let Some(base_url) = stream["base_url"].as_str() {
            return Some(base_url.to_string());
        }
        if let Some(backup_urls) = stream["backupUrl"].as_array()
            && let Some(first) = backup_urls.first()
        {
            return Some(first.as_str()?.to_string());
        }
        None
    }

    fn get_cookie(&self, req: &CreateTaskRequest) -> Option<String> {
        if let Some(ref c) = req.decrypted_cookies {
            return Some(c.clone());
        }
        for (k, v) in &req.headers {
            if k.to_lowercase() == "cookie" {
                return Some(v.clone());
            }
        }
        if let Some(saved_cookie) = bilibili_cookie_store::load_bilibili_cookie() {
            return Some(saved_cookie);
        }
        None
    }
}

#[async_trait]
impl FeaturePack for BiliApiParser {
    fn pack_id(&self) -> PackId {
        PackId::BilibiliDash
    }

    fn priority(&self) -> u32 {
        90
    }

    fn matches(&self, req: &CreateTaskRequest) -> bool {
        if req.url.contains("bilibili.com") {
            return true;
        }
        if req.source.as_deref() == Some("resource_merge") {
            let title_lower = req.title.to_lowercase();
            if title_lower.contains("bilibili") || title_lower.contains("哔哩哔哩") {
                return true;
            }
        }
        false
    }

    async fn parse(&self, req: &CreateTaskRequest) -> Result<Task> {
        let mut target_url = req.url.clone();

        // Handle resource_merge interception
        if req.source.as_deref() == Some("resource_merge") {
            for res in &req.resources {
                if let Some(headers) = res.get("headers").and_then(|h| h.as_object()) {
                    for (k, v) in headers {
                        if k.to_lowercase() == "referer"
                            && let Some(ref_str) = v.as_str()
                            && (ref_str.contains("/video/BV") || ref_str.contains("/video/av"))
                        {
                            target_url = ref_str.to_string();
                        }
                    }
                }
            }
        }

        let cookie = self.get_cookie(req);

        info!("BiliApiParser started for URL: {}", target_url);

        let bvid = self
            .extract_bvid(&target_url)
            .context("Failed to extract BVID from URL")?;
        info!("Extracted BVID: {}", bvid);

        // 1. 获取视频信息 (cid, title)
        let view_url = format!(
            "https://api.bilibili.com/x/web-interface/view?bvid={}",
            bvid
        );
        let mut view_req = self.client.get(&view_url).header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        );

        if let Some(ref c) = cookie {
            view_req = view_req.header("Cookie", c);
        }

        let view_resp = view_req.send().await?.json::<Value>().await?;

        if view_resp["code"].as_i64() != Some(0) {
            anyhow::bail!(
                "Failed to get video info: {}",
                view_resp["message"].as_str().unwrap_or("unknown error")
            );
        }

        let cid = view_resp["data"]["cid"]
            .as_i64()
            .context("Missing cid in response")?;
        let mut title = view_resp["data"]["title"]
            .as_str()
            .unwrap_or("bilibili_video")
            .to_string();

        // 清理非法字符
        title = title.replace(
            |c: char| !c.is_alphanumeric() && !c.is_whitespace() && c != '-',
            "_",
        );

        info!("Parsed video info: cid={}, title={}", cid, title);

        // 2. 获取音视频流 URL
        let play_url = format!(
            "https://api.bilibili.com/x/player/wbi/playurl?bvid={}&cid={}&qn=116&fnval=16&fourk=1",
            bvid, cid
        );
        let mut play_req = self
            .client
            .get(&play_url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            )
            .header("Referer", "https://www.bilibili.com");

        if let Some(ref c) = cookie {
            play_req = play_req.header("Cookie", c);
        }

        let play_resp = play_req.send().await?.json::<Value>().await?;

        if play_resp["code"].as_i64() != Some(0) {
            anyhow::bail!(
                "Failed to get playurl: {}",
                play_resp["message"].as_str().unwrap_or("unknown error")
            );
        }

        let dash = &play_resp["data"]["dash"];
        if dash.is_null() {
            anyhow::bail!("No DASH streams found in response");
        }

        let video_url = self
            .extract_stream_url(&dash["video"][0])
            .context("Failed to extract video url")?;
        let audio_url = self
            .extract_stream_url(&dash["audio"][0])
            .context("Failed to extract audio url")?;

        let mut headers = HashMap::new();
        headers.insert(
            "Referer".to_string(),
            "https://www.bilibili.com".to_string(),
        );
        headers.insert(
            "User-Agent".to_string(),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36".to_string(),
        );

        let mut task = Task::new(
            if req.request_id.is_empty() {
                format!("bili_{}", bvid)
            } else {
                req.request_id.clone()
            },
            title.clone(),
            target_url.to_string(),
            self.pack_id(),
            0,
            "./downloads".to_string(),
        );

        task.execution_kind = Some(ExecutionKind::DashMediaMerge);
        task.cookie_str = cookie.clone();

        task.add_stage(Stage {
            stage_index: 0,
            stage_type: StageType::Video,
            url: video_url,
            file_size: 0,
            headers: headers.clone(),
            block_num: 1,
        });

        task.add_stage(Stage {
            stage_index: 1,
            stage_type: StageType::Audio,
            url: audio_url,
            file_size: 0,
            headers: headers.clone(),
            block_num: 1,
        });

        info!("Successfully parsed Task: {}", title);
        Ok(task)
    }
}
