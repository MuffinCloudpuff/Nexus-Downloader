use crate::core::models::Task;
use crate::utils::to_safe_filename;
use anyhow::{Context, Result};
use std::path::PathBuf;

pub fn captured_merge_output_path(title: &str, source_url: &str, task_id: &str) -> String {
    let safe_title = to_safe_filename(title, "captured_media");
    let suffix = bilibili_video_suffix(source_url)
        .unwrap_or_else(|| safe_suffix(&task_id.chars().take(8).collect::<String>()));
    format!("{}-{}.mp4", safe_title, suffix)
}

pub fn hls_output_path(title: &str, playlist_url: &str, task_id: &str) -> PathBuf {
    let safe_title = to_safe_filename(title, "hls_media");
    let suffix = twitter_media_suffix(playlist_url)
        .unwrap_or_else(|| safe_suffix(&task_id.chars().take(8).collect::<String>()));
    PathBuf::from(format!("{}-{}.mp4", safe_title, suffix))
}

pub fn youtube_output_path(title: &str) -> PathBuf {
    PathBuf::from(format!(
        "{}.mp4",
        to_safe_filename(title, "Y-youtube_video")
    ))
}

pub fn http_output_path(task: &Task) -> Result<PathBuf> {
    let first_stage = task
        .stages
        .first()
        .context("HTTP task has no downloadable stage")?;

    let safe_title = to_safe_filename(&task.title, "file_download");
    let mut ext = "file".to_string();

    if let Some(url_path) = first_stage.url.split('?').next()
        && let Some(idx) = url_path.rfind('.')
        && url_path.len() - idx <= 5
    {
        ext = url_path[idx + 1..].to_string();
    }

    if ext == "file"
        && task.title.contains('.')
        && let Some(idx) = task.title.rfind('.')
    {
        ext = task.title[idx + 1..].to_string();
    }

    let final_filename = if safe_title.ends_with(&format!(".{}", ext)) {
        safe_title
    } else {
        format!("{}.{}", safe_title, ext)
    };

    Ok(PathBuf::from(final_filename))
}

fn bilibili_video_suffix(raw_url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(raw_url).ok()?;
    let mut previous_was_video = false;
    for part in parsed
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default()
    {
        if previous_was_video && (part.starts_with("BV") || part.starts_with("av")) {
            return Some(safe_suffix(part));
        }
        previous_was_video = part == "video";
    }
    None
}

fn twitter_media_suffix(raw_url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(raw_url).ok()?;
    let parts: Vec<&str> = parsed
        .path_segments()
        .map(|segments| segments.collect())
        .unwrap_or_default();

    if let Some(pl_index) = parts.iter().position(|part| *part == "pl")
        && pl_index > 0
    {
        let id_index = if parts[pl_index - 1] == "pu" && pl_index > 1 {
            pl_index - 2
        } else {
            pl_index - 1
        };
        return Some(safe_suffix(parts[id_index]));
    }

    for part in parts {
        if part.len() >= 10
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Some(safe_suffix(part));
        }
    }
    None
}

fn safe_suffix(value: &str) -> String {
    let suffix: String = value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect();

    if suffix.is_empty() {
        "media".to_string()
    } else {
        suffix
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::{PackId, Stage, StageType};
    use std::collections::HashMap;

    #[test]
    fn captured_merge_output_uses_b_prefix_title_and_bv_suffix() {
        assert_eq!(
            captured_merge_output_path(
                "B-世界杯期间不抓移民？那这世界杯不是白办了吗",
                "https://www.bilibili.com/video/BV15d7D6UE4i/?vd_source=abc",
                "req_page_media_344ade53",
            ),
            "B-世界杯期间不抓移民？那这世界杯不是白办了吗-BV15d7D6UE4i.mp4"
        );
    }

    #[test]
    fn hls_output_uses_x_prefix_title_and_twitter_media_id() {
        let path = hls_output_path(
            "X-linky (@linky)",
            "https://video.twimg.com/ext_tw_video/2063456657753944064/pu/pl/4KaiIKxMVoDTCAs6.m3u8?tag=12",
            "task-c",
        );
        assert_eq!(
            path,
            PathBuf::from("X-linky (@linky)-2063456657753944064.mp4")
        );
    }

    #[test]
    fn youtube_output_uses_y_prefixed_title() {
        assert_eq!(
            youtube_output_path("Y-example video"),
            PathBuf::from("Y-example video.mp4")
        );
    }

    #[test]
    fn http_output_preserves_existing_extension() {
        let mut task = Task::new(
            "task-1".to_string(),
            "video".to_string(),
            "https://example.com/video.mp4".to_string(),
            PackId::Http,
            0,
            "video.mp4".to_string(),
        );
        task.add_stage(Stage {
            stage_index: 0,
            stage_type: StageType::Video,
            url: "https://example.com/video.mp4".to_string(),
            file_size: 0,
            headers: HashMap::new(),
            block_num: 1,
        });

        assert_eq!(http_output_path(&task).unwrap(), PathBuf::from("video.mp4"));
    }
}
