use crate::parsers::interfaces::{SiteParser, SiteResolution};
use crate::parsers::policy::quality;
use crate::protocol::task::{CapturedResource, CreateTaskCommand};
use crate::services::bilibili_cookie_store;
use crate::utils::to_safe_filename;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use tracing::info;

pub struct BilibiliSiteParser {
    client: Client,
}

impl Default for BilibiliSiteParser {
    fn default() -> Self {
        Self::new()
    }
}

impl BilibiliSiteParser {
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

    fn stream_size(&self, stream: &Value) -> Option<u64> {
        stream["size"].as_u64()
    }

    fn context_page_url<'a>(&self, cmd: &'a CreateTaskCommand) -> Option<&'a str> {
        cmd.client_context
            .as_ref()
            .and_then(|context| context.page_url.as_deref())
            .filter(|url| url.contains("bilibili.com/video/"))
    }

    fn get_cookie(&self, cmd: &CreateTaskCommand) -> Option<String> {
        if let Some(cookie) = cmd.decrypted_cookies.as_deref() {
            let cookie = cookie.trim();
            if !cookie.is_empty() {
                info!("🔑 Using Bilibili cookie from encrypted browser payload");
                return Some(cookie.to_string());
            }
        }
        for (k, v) in &cmd.headers {
            if k.to_lowercase() == "cookie" {
                info!("🔑 Using Bilibili cookie from request headers");
                return Some(v.clone());
            }
        }
        if let Some(saved_cookie) = bilibili_cookie_store::load_bilibili_cookie() {
            return Some(saved_cookie);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::task::{ClientContext, CreateTaskCommand};

    fn command_with_cookies(
        header_cookie: Option<&str>,
        decrypted_cookies: Option<&str>,
    ) -> CreateTaskCommand {
        let mut headers = HashMap::new();
        if let Some(cookie) = header_cookie {
            headers.insert("Cookie".to_string(), cookie.to_string());
        }
        CreateTaskCommand {
            request_id: "req".to_string(),
            source: Some("page_media".to_string()),
            title: "B-test".to_string(),
            url: "https://www.bilibili.com/video/BV1iH6ZBREFn/".to_string(),
            headers,
            resources: vec![],
            client_context: None,
            decrypted_cookies: decrypted_cookies.map(str::to_string),
            filename: None,
            save_path: None,
            mime: None,
            size: None,
            supports_range: None,
            payload_fingerprint: "fp".to_string(),
        }
    }

    #[test]
    fn encrypted_browser_cookie_is_used_before_saved_cookie_fallback() {
        let parser = BilibiliSiteParser::new();
        assert_eq!(
            parser.get_cookie(&command_with_cookies(None, Some("SESSDATA=browser"))),
            Some("SESSDATA=browser".to_string())
        );
    }

    #[test]
    fn encrypted_browser_cookie_has_priority_over_header_cookie() {
        let parser = BilibiliSiteParser::new();
        assert_eq!(
            parser.get_cookie(&command_with_cookies(
                Some("SESSDATA=header"),
                Some("SESSDATA=browser")
            )),
            Some("SESSDATA=browser".to_string())
        );
    }

    #[test]
    fn matches_bilibili_from_client_context_page_url() {
        let parser = BilibiliSiteParser::new();
        let mut cmd = command_with_cookies(None, None);
        cmd.url = "https://cn-bj-fx-01-02.bilivideo.com/upgcxcode/video.m4s".to_string();
        cmd.client_context = Some(ClientContext {
            page_url: Some("https://www.bilibili.com/video/BV17BGC6JE3t/".to_string()),
            ..Default::default()
        });

        assert!(parser.matches(&cmd));
        assert_eq!(
            parser.context_page_url(&cmd),
            Some("https://www.bilibili.com/video/BV17BGC6JE3t/")
        );
    }
}

#[async_trait]
impl SiteParser for BilibiliSiteParser {
    fn id(&self) -> &'static str {
        "bilibili"
    }
    fn priority(&self) -> u32 {
        100
    }

    fn matches(&self, cmd: &CreateTaskCommand) -> bool {
        if cmd.url.contains("bilibili.com") {
            return true;
        }
        if self.context_page_url(cmd).is_some() {
            return true;
        }
        if cmd.source.as_deref() == Some("resource_merge") {
            let title_lower = cmd.title.to_lowercase();
            if title_lower.contains("bilibili") || title_lower.contains("哔哩哔哩") {
                return true;
            }
        }
        false
    }

    async fn parse(&self, cmd: &CreateTaskCommand) -> Result<SiteResolution> {
        let mut target_url = self
            .context_page_url(cmd)
            .map(str::to_string)
            .unwrap_or_else(|| cmd.url.clone());

        // Handle resource_merge interception
        if cmd.source.as_deref() == Some("resource_merge") {
            for res in &cmd.resources {
                for (k, v) in &res.headers {
                    if k.to_lowercase() == "referer"
                        && (v.contains("/video/BV") || v.contains("/video/av"))
                    {
                        target_url = v.to_string();
                    }
                }
            }
        }

        let cookie = self.get_cookie(cmd);

        info!("BilibiliSiteParser started for URL: {}", target_url);

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
        let raw_title = view_resp["data"]["title"]
            .as_str()
            .unwrap_or("bilibili_video")
            .to_string();
        let title = format!("B-{}", to_safe_filename(&raw_title, "bilibili_video"));

        info!("Parsed video info: cid={}, title={}", cid, title);

        // 2. 获取音视频流 URL
        let play_url = format!(
            "https://api.bilibili.com/x/player/wbi/playurl?bvid={}&cid={}&qn={}&fnval=16&fourk=0",
            bvid,
            cid,
            quality::DEFAULT_BILIBILI_MAX_VIDEO_QN
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

        let dash = &play_resp["data"]["dash"];
        if dash.is_null() {
            anyhow::bail!(
                "No DASH stream found in response. You may need a higher privilege cookie."
            );
        }

        let video_streams = dash["video"].as_array().context("No video stream found")?;
        let video_stream = quality::select_bilibili_video_stream(
            video_streams,
            quality::DEFAULT_BILIBILI_MAX_VIDEO_QN,
        )
        .context("No video stream found")?;

        let audio_stream = dash["audio"]
            .as_array()
            .and_then(|a| a.first())
            .context("No audio stream found")?;

        info!(
            selected_qn = ?quality::bilibili_stream_video_qn(video_stream),
            selected_bandwidth = quality::bilibili_stream_bandwidth(video_stream),
            stream_count = video_streams.len(),
            "selected Bilibili video stream"
        );

        let video_url = self
            .extract_stream_url(video_stream)
            .context("Failed to extract video URL")?;
        let audio_url = self
            .extract_stream_url(audio_stream)
            .context("Failed to extract audio URL")?;

        let mut out_headers = HashMap::new();
        out_headers.insert(
            "Referer".to_string(),
            "https://www.bilibili.com".to_string(),
        );
        out_headers.insert(
            "User-Agent".to_string(),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36".to_string(),
        );
        if let Some(ref c) = cookie {
            out_headers.insert("Cookie".to_string(), c.clone());
        }

        let resources = vec![
            CapturedResource {
                kind: Some("video".to_string()),
                url: video_url,
                size: self.stream_size(video_stream),
                ..Default::default()
            },
            CapturedResource {
                kind: Some("audio".to_string()),
                url: audio_url,
                size: self.stream_size(audio_stream),
                ..Default::default()
            },
        ];

        Ok(SiteResolution {
            url: cmd.url.clone(),
            title,
            headers: out_headers,
            resources,
        })
    }
}
