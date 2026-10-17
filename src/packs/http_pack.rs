use crate::core::interfaces::{CreateTaskRequest, FeaturePack};
use crate::core::models::{ExecutionKind, PackId, Stage, StageType, Task};
use crate::utils::to_safe_filename;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct HttpPack;

impl Default for HttpPack {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpPack {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl FeaturePack for HttpPack {
    fn pack_id(&self) -> PackId {
        PackId::Http
    }

    fn priority(&self) -> u32 {
        10 // Fallback generic priority
    }

    fn matches(&self, req: &CreateTaskRequest) -> bool {
        req.url.starts_with("http://") || req.url.starts_with("https://")
    }

    async fn parse(&self, req: &CreateTaskRequest) -> Result<Task> {
        let url = &req.url;
        let path = "./downloads";

        let headers = req.headers.clone();

        // 发送探测请求 (Probe) 来获取文件大小和断点续传支持情况
        let client = Client::builder()
            .danger_accept_invalid_certs(true)
            .build()?;

        let mut file_size = 0;
        let mut supports_range = false;

        let mut req_builder = client.get(url).header("range", "bytes=1-1");
        for (k, v) in &headers {
            req_builder = req_builder.header(k, v);
        }

        let res = req_builder.send().await?;
        if res.status().is_success() || res.status() == 206 {
            if res.status() == 206 {
                supports_range = true;
            }
            if let Some(cl) = res.headers().get("content-length")
                && let Ok(len) = cl.to_str().unwrap_or("").parse::<u64>()
            {
                file_size = len;
            }
            if let Some(cr) = res.headers().get("content-range") {
                let s = cr.to_str().unwrap_or("");
                if let Some(idx) = s.rfind('/')
                    && let Ok(len) = s[idx + 1..].parse::<u64>()
                {
                    file_size = len;
                }
            }
        }

        // 尝试从 Content-Disposition 提取文件名
        let mut filename = req.title.clone();
        let mut cd_filename = String::new();
        if let Some(cd) = res.headers().get("content-disposition") {
            let cd_str = cd.to_str().unwrap_or("");
            if let Some(idx) = cd_str.find("filename=") {
                cd_filename = cd_str[idx + 9..]
                    .trim_matches(|c| c == '"' || c == '\'' || c == ';')
                    .to_string();
            }
        }

        if !cd_filename.is_empty() {
            filename = cd_filename;
        } else if filename.is_empty()
            && let Some(idx) = url.rfind('/')
        {
            filename = url[idx + 1..]
                .split('?')
                .next()
                .unwrap_or("unknown")
                .to_string();
        }

        let safe_filename = to_safe_filename(&filename, "file_download");
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
            safe_filename.clone(),
            url.to_string(),
            self.pack_id(),
            file_size,
            path.to_string(),
        );
        task.execution_kind = Some(ExecutionKind::HttpFile);

        task.cookie_str = req.decrypted_cookies.clone();

        task.add_stage(Stage {
            stage_index: 0,
            stage_type: StageType::HttpDownload,
            url: url.to_string(),
            file_size,
            headers,
            block_num: if supports_range { 16 } else { 1 },
        });

        Ok(task)
    }
}
