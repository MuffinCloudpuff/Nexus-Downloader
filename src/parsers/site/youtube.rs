use crate::parsers::interfaces::{SiteParser, SiteResolution};
use crate::protocol::task::CreateTaskCommand;
use anyhow::Result;
use async_trait::async_trait;

pub struct YoutubeSiteParser;

#[async_trait]
impl SiteParser for YoutubeSiteParser {
    fn id(&self) -> &'static str {
        "youtube"
    }
    fn priority(&self) -> u32 {
        100
    }

    fn matches(&self, cmd: &CreateTaskCommand) -> bool {
        cmd.url.contains("youtube.com") || cmd.url.contains("youtu.be")
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
