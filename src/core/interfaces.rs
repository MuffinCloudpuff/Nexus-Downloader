use crate::core::models::{PackId, Task};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct CreateTaskRequest {
    pub request_id: String,
    pub title: String,
    pub source: Option<String>,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub resources: Vec<serde_json::Value>,
    pub decrypted_cookies: Option<String>,
}

#[async_trait]
pub trait FeaturePack: Send + Sync {
    /// 获取功能包的全局唯一 ID
    fn pack_id(&self) -> PackId;

    /// 获取优先级。在遍历路由时，优先级越高的包越先进行 matches 测试
    fn priority(&self) -> u32;

    /// 根据 Request 判断当前功能包是否能够接管处理
    fn matches(&self, req: &CreateTaskRequest) -> bool;

    /// 核心解析流程：提取目标 URL 的信息，抓取真实直链，分离出下载阶段 (Stages)，并封装为 Task
    async fn parse(&self, req: &CreateTaskRequest) -> Result<Task>;
}

pub struct PackRegistry {
    packs: Vec<Box<dyn FeaturePack>>,
}

impl Default for PackRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PackRegistry {
    pub fn new() -> Self {
        Self { packs: Vec::new() }
    }

    pub fn register(&mut self, pack: Box<dyn FeaturePack>) {
        self.packs.push(pack);
        self.packs.sort_by_key(|b| std::cmp::Reverse(b.priority()));
    }

    pub async fn parse_task(&self, req: &CreateTaskRequest) -> Result<Task> {
        for pack in &self.packs {
            if pack.matches(req) {
                return pack.parse(req).await;
            }
        }
        anyhow::bail!("No pack matched the request")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct MockPack {
        id: PackId,
        priority: u32,
        matches_ret: bool,
    }

    #[async_trait]
    impl FeaturePack for MockPack {
        fn pack_id(&self) -> PackId {
            self.id.clone()
        }
        fn priority(&self) -> u32 {
            self.priority
        }
        fn matches(&self, _req: &CreateTaskRequest) -> bool {
            self.matches_ret
        }
        async fn parse(&self, req: &CreateTaskRequest) -> Result<Task> {
            Ok(Task::new(
                "test".into(),
                "test".into(),
                req.url.clone(),
                self.pack_id(),
                0,
                "./".into(),
            ))
        }
    }

    #[tokio::test]
    async fn test_pack_registry_priority() {
        let mut registry = PackRegistry::new();

        registry.register(Box::new(MockPack {
            id: PackId::Other("Low".into()),
            priority: 10,
            matches_ret: true,
        }));
        registry.register(Box::new(MockPack {
            id: PackId::Other("High".into()),
            priority: 90,
            matches_ret: true,
        }));

        let req = CreateTaskRequest {
            request_id: "1".into(),
            title: "Test".into(),
            source: None,
            url: "http://test".into(),
            headers: Default::default(),
            resources: vec![],
            decrypted_cookies: None,
        };

        let task = registry.parse_task(&req).await.unwrap();
        assert_eq!(task.pack_id, PackId::Other("High".into()));
    }
}
