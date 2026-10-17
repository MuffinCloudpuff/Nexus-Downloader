use crate::core::execution_plan::ExecutionPlan;
use crate::protocol::task::{CapturedResource, CreateTaskCommand};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;

/// Intermediate result after Site parser extracts info
#[derive(Debug, Clone)]
pub struct SiteResolution {
    pub url: String,
    pub title: String,
    pub headers: HashMap<String, String>,
    pub resources: Vec<CapturedResource>,
}

#[async_trait]
pub trait SiteParser: Send + Sync {
    fn id(&self) -> &'static str;
    fn priority(&self) -> u32;
    fn matches(&self, cmd: &CreateTaskCommand) -> bool;
    async fn parse(&self, cmd: &CreateTaskCommand) -> Result<SiteResolution>;
}

#[async_trait]
pub trait ProtocolParser: Send + Sync {
    fn id(&self) -> &'static str;
    fn priority(&self) -> u32;
    fn matches(&self, resolution: &SiteResolution) -> bool;
    async fn parse(&self, task_id: &str, resolution: &SiteResolution) -> Result<ExecutionPlan>;
}
