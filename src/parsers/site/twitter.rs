use crate::parsers::interfaces::{SiteParser, SiteResolution};
use crate::protocol::task::CreateTaskCommand;
use anyhow::Result;
use async_trait::async_trait;

pub struct TwitterSiteParser;

#[async_trait]
impl SiteParser for TwitterSiteParser {
    fn id(&self) -> &'static str {
        "twitter"
    }
    fn priority(&self) -> u32 {
        100
    }

    fn matches(&self, cmd: &CreateTaskCommand) -> bool {
        cmd.url.contains("twitter.com")
            || cmd.url.contains("x.com")
            || cmd.url.contains("twimg.com")
    }

    async fn parse(&self, cmd: &CreateTaskCommand) -> Result<SiteResolution> {
        Ok(SiteResolution {
            url: cmd.url.clone(),
            title: cmd.title.clone(),
            headers: cmd.effective_download_headers(),
            resources: cmd.resources.clone(),
        })
    }
}
