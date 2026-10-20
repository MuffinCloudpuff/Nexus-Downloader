use crate::core::models::Task;
use crate::parsers::policy::output_category::OutputCategory;

fn douyin_host(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    ["douyin.com", "douyinvod.com"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

/// A presentation hint, not an engine/pack identifier. Older merged Douyin
/// tasks reuse BilibiliDash, so classify their persisted media URLs as well.
pub(super) fn for_task(task: &Task) -> Option<&'static str> {
    if task
        .output_category
        .as_ref()
        .is_some_and(|category| *category != OutputCategory::Video)
    {
        return None;
    }
    let is_douyin = douyin_host(&task.url)
        || task.stages.iter().any(|stage| {
            douyin_host(&stage.url)
                || stage
                    .headers
                    .iter()
                    .any(|(key, value)| key.eq_ignore_ascii_case("referer") && douyin_host(value))
        });
    is_douyin.then_some("douyin")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::{PackId, Stage, StageType};
    use crate::protocol::task::TaskSummary;
    use std::collections::HashMap;

    fn video(url: &str) -> Task {
        let mut task = Task::new(
            "test".into(),
            "video.mp4".into(),
            url.into(),
            PackId::BilibiliDash,
            0,
            "D:/video.mp4".into(),
        );
        task.output_category = Some(OutputCategory::Video);
        task
    }

    #[test]
    fn douyin_identity_overrides_legacy_merge_pack_without_exposing_urls() {
        let mut task = video("https://example.com/media");
        task.stages.push(Stage {
            stage_index: 0,
            stage_type: StageType::Video,
            url: "https://v11-weba.douyinvod.com/video?secret=not-for-ui".into(),
            file_size: 0,
            headers: HashMap::new(),
            block_num: 0,
        });
        let summary = serde_json::to_value(TaskSummary::from(&task)).unwrap();
        assert_eq!(summary["media_platform"], "douyin");
        assert_eq!(summary["pack_name"], "bilibili_dash");
        assert!(!summary.to_string().contains("not-for-ui"));
        task.output_category = None; // Persisted records predating category metadata.
        assert_eq!(for_task(&task), Some("douyin"));
    }

    #[test]
    fn domain_boundaries_and_non_video_files_are_not_misclassified() {
        for url in [
            "https://www.douyin.com/video/123",
            "https://douyinvod.com/video.mp4",
        ] {
            assert_eq!(for_task(&video(url)), Some("douyin"));
        }
        for url in [
            "https://fakedouyin.com/video",
            "https://douyin.com.evil.test/video",
            "https://example.com/douyin.com/video",
            "file:///D:/douyin.com/video.mp4",
        ] {
            assert_eq!(for_task(&video(url)), None, "{url}");
        }
        let mut image = video("https://www.douyin.com/image.jpg");
        image.output_category = Some(OutputCategory::Image);
        assert_eq!(for_task(&image), None);
    }
}
