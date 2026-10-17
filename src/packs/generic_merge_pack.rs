use crate::core::interfaces::{CreateTaskRequest, FeaturePack};
use crate::core::models::{ExecutionKind, PackId, Stage, StageType, Task};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;

pub struct GenericMergePack;

impl Default for GenericMergePack {
    fn default() -> Self {
        Self::new()
    }
}

impl GenericMergePack {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl FeaturePack for GenericMergePack {
    fn pack_id(&self) -> PackId {
        PackId::GenericMerge
    }

    fn priority(&self) -> u32 {
        20
    }

    fn matches(&self, req: &CreateTaskRequest) -> bool {
        req.source.as_deref() == Some("resource_merge")
    }

    async fn parse(&self, req: &CreateTaskRequest) -> Result<Task> {
        let mut task = Task::new(
            if req.request_id.is_empty() {
                "unknown_id".to_string()
            } else {
                req.request_id.clone()
            },
            if req.title.is_empty() {
                "resource_merge_task".to_string()
            } else {
                req.title.clone()
            },
            "gd3+ffmpeg://merge".to_string(),
            self.pack_id(),
            0,
            "./downloads".to_string(),
        );
        task.execution_kind = Some(ExecutionKind::DashMediaMerge);
        task.cookie_str = req.decrypted_cookies.clone();

        for (idx, item) in req.resources.iter().enumerate() {
            if let Some(url) = item.get("url").and_then(|u| u.as_str()) {
                let mut headers_map = HashMap::new();
                if let Some(hdrs) = item.get("headers").and_then(|h| h.as_object()) {
                    for (k, v) in hdrs {
                        if let Some(s) = v.as_str() {
                            headers_map.insert(k.clone(), s.to_string());
                        }
                    }
                }

                task.add_stage(Stage {
                    stage_index: idx as u32,
                    stage_type: if idx == 0 {
                        StageType::Video
                    } else {
                        StageType::Audio
                    },
                    url: url.to_string(),
                    file_size: item.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
                    headers: headers_map,
                    block_num: 1,
                });
            }
        }

        Ok(task)
    }
}
