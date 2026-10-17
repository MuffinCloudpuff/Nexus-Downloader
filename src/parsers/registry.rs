use super::interfaces::{ProtocolParser, SiteParser};
use crate::core::execution_plan::ExecutionPlan;
use crate::protocol::task::CreateTaskCommand;
use anyhow::Result;

pub struct ParserRegistry {
    site_parsers: Vec<Box<dyn SiteParser>>,
    protocol_parsers: Vec<Box<dyn ProtocolParser>>,
}

impl Default for ParserRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ParserRegistry {
    pub fn new() -> Self {
        Self {
            site_parsers: Vec::new(),
            protocol_parsers: Vec::new(),
        }
    }

    pub fn register_site(&mut self, parser: Box<dyn SiteParser>) {
        self.site_parsers.push(parser);
        self.site_parsers
            .sort_by_key(|b| std::cmp::Reverse(b.priority()));
    }

    pub fn register_protocol(&mut self, parser: Box<dyn ProtocolParser>) {
        self.protocol_parsers.push(parser);
        self.protocol_parsers
            .sort_by_key(|b| std::cmp::Reverse(b.priority()));
    }

    pub async fn resolve(&self, task_id: &str, cmd: &CreateTaskCommand) -> Result<ExecutionPlan> {
        let mut resolution = None;
        for site in &self.site_parsers {
            if site.matches(cmd) {
                tracing::info!(
                    request_id = %cmd.request_id,
                    task_id,
                    parser_id = site.id(),
                    "matched site parser"
                );
                resolution = Some(site.parse(cmd).await?);
                break;
            }
        }
        let resolution = resolution.unwrap_or_else(|| {
            tracing::info!(
                request_id = %cmd.request_id,
                task_id,
                "no site parser matched; using generic resolution"
            );
            super::interfaces::SiteResolution {
                url: cmd.url.clone(),
                title: cmd.title.clone(),
                headers: cmd.effective_download_headers(),
                resources: cmd.resources.clone(),
            }
        });

        for proto in &self.protocol_parsers {
            if proto.matches(&resolution) {
                tracing::info!(
                    request_id = %cmd.request_id,
                    task_id,
                    parser_id = proto.id(),
                    resource_count = resolution.resources.len(),
                    "matched protocol parser"
                );
                return proto.parse(task_id, &resolution).await;
            }
        }

        let host = reqwest::Url::parse(&resolution.url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_string))
            .unwrap_or_else(|| "invalid".to_string());
        anyhow::bail!(
            "No ProtocolParser matched the resolution for host: {}",
            host
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::execution_plan::{
        ExecutionKind, ExecutionPlan, MergePolicy, OutputTarget, ResourceBudget, ResumePolicy,
    };
    use crate::parsers::interfaces::{ProtocolParser, SiteParser, SiteResolution};
    use crate::parsers::protocol::{
        http_file::HttpFileProtocolParser, youtube::YoutubeProtocolParser,
    };
    use crate::parsers::site::youtube::YoutubeSiteParser;
    use crate::protocol::task::CreateTaskCommand;
    use async_trait::async_trait;

    struct MockSiteParser {
        id: &'static str,
        priority: u32,
        matched: bool,
    }
    #[async_trait]
    impl SiteParser for MockSiteParser {
        fn id(&self) -> &'static str {
            self.id
        }
        fn priority(&self) -> u32 {
            self.priority
        }
        fn matches(&self, _cmd: &CreateTaskCommand) -> bool {
            self.matched
        }
        async fn parse(&self, cmd: &CreateTaskCommand) -> anyhow::Result<SiteResolution> {
            Ok(SiteResolution {
                url: format!("{}-{}", self.id, cmd.url),
                title: cmd.title.clone(),
                headers: Default::default(),
                resources: vec![],
            })
        }
    }

    struct MockProtoParser {
        id: &'static str,
        priority: u32,
        matched: bool,
    }
    #[async_trait]
    impl ProtocolParser for MockProtoParser {
        fn id(&self) -> &'static str {
            self.id
        }
        fn priority(&self) -> u32 {
            self.priority
        }
        fn matches(&self, _res: &SiteResolution) -> bool {
            self.matched
        }
        async fn parse(
            &self,
            task_id: &str,
            _res: &SiteResolution,
        ) -> anyhow::Result<ExecutionPlan> {
            Ok(ExecutionPlan {
                task_id: task_id.to_string(),
                request_id: None,
                trace_id: None,
                source: None,
                title: self.id.to_string(),
                output_category: None,
                output: OutputTarget::File { path: "".into() },
                kind: ExecutionKind::HttpFile,
                stages: vec![],
                resume_policy: ResumePolicy::Disabled,
                merge_policy: MergePolicy::None,
                resource_budget: ResourceBudget {
                    max_connections: 1,
                    max_bandwidth_bytes_per_sec: None,
                },
            })
        }
    }

    #[tokio::test]
    async fn test_registry_priority() {
        let mut registry = ParserRegistry::new();
        registry.register_site(Box::new(MockSiteParser {
            id: "low",
            priority: 10,
            matched: true,
        }));
        registry.register_site(Box::new(MockSiteParser {
            id: "high",
            priority: 90,
            matched: true,
        }));

        registry.register_protocol(Box::new(MockProtoParser {
            id: "p_low",
            priority: 10,
            matched: true,
        }));
        registry.register_protocol(Box::new(MockProtoParser {
            id: "p_high",
            priority: 90,
            matched: true,
        }));

        let cmd = CreateTaskCommand {
            request_id: "1".into(),
            source: None,
            title: "Test".into(),
            url: "test_url".into(),
            headers: Default::default(),
            resources: vec![],
            filename: None,
            mime: None,
            size: None,
            supports_range: None,
            client_context: None,
            decrypted_cookies: None,
            payload_fingerprint: "fp".into(),
        };

        let plan = registry.resolve("task_1", &cmd).await.unwrap();

        // Protocol parser "p_high" should match
        assert_eq!(plan.title, "p_high");
    }

    #[tokio::test]
    async fn youtube_site_resolution_does_not_fall_back_to_http_file() {
        let mut registry = ParserRegistry::new();
        registry.register_site(Box::new(YoutubeSiteParser));
        registry.register_protocol(Box::new(YoutubeProtocolParser));
        registry.register_protocol(Box::new(HttpFileProtocolParser));

        let cmd = CreateTaskCommand {
            request_id: "req-youtube".into(),
            source: Some("page_media".into()),
            title: "Y-video - YouTube".into(),
            url: "https://www.youtube.com/shorts/p2a-OwUcZEM".into(),
            headers: Default::default(),
            resources: vec![],
            filename: None,
            mime: None,
            size: None,
            supports_range: None,
            client_context: None,
            decrypted_cookies: None,
            payload_fingerprint: "fp".into(),
        };

        let plan = registry.resolve("task-youtube", &cmd).await.unwrap();

        assert_eq!(plan.kind, ExecutionKind::YouTubeExternal);
    }
}
