use crate::core::execution_plan::{ExecutionKind, ExecutionPlan, OutputTarget};
use crate::parsers::policy::output_category::OutputCategory;
use crate::protocol::settings::AppSettingsDto;
use crate::protocol::task::CreateTaskCommand;
use crate::utils::to_safe_filename;
use std::path::{Path, PathBuf};

pub fn apply_output_destination(
    cmd: &CreateTaskCommand,
    mut plan: ExecutionPlan,
    category: &OutputCategory,
    settings: &AppSettingsDto,
) -> ExecutionPlan {
    // Preserve the user's exact filename and directory; category routing is only
    // a default for tasks without a browser-confirmed destination.
    if let Some(path) = cmd.save_path.as_ref() {
        plan.output = OutputTarget::File { path: path.clone() };
        plan.output_category = Some(category.clone());
        return plan;
    }
    let filename = final_filename(cmd, &plan);
    let directory = category_directory(category, settings);
    let final_path = Path::new(&directory).join(filename);
    plan.output = OutputTarget::File {
        path: final_path.to_string_lossy().to_string(),
    };
    plan.output_category = Some(category.clone());
    plan
}

pub fn category_directory(category: &OutputCategory, settings: &AppSettingsDto) -> String {
    use crate::protocol::settings::DownloadDirectoryMode;

    match settings.directory_mode {
        DownloadDirectoryMode::Flat => settings.download_folder.clone(),
        DownloadDirectoryMode::Categorized => {
            let root = Path::new(&settings.categorized_root);
            match category {
                OutputCategory::Video => root.join("video").to_string_lossy().to_string(),
                OutputCategory::Audio => root.join("audio").to_string_lossy().to_string(),
                OutputCategory::Image => root.join("image").to_string_lossy().to_string(),
                OutputCategory::Document => root.join("document").to_string_lossy().to_string(),
                OutputCategory::Archive => root.join("archive").to_string_lossy().to_string(),
                OutputCategory::Program => root.join("program").to_string_lossy().to_string(),
                OutputCategory::Other => root.join("other").to_string_lossy().to_string(),
            }
        }
        DownloadDirectoryMode::Custom => match category {
            OutputCategory::Video => settings.custom_paths.video.clone(),
            OutputCategory::Audio => settings.custom_paths.audio.clone(),
            OutputCategory::Image => settings.custom_paths.image.clone(),
            OutputCategory::Document => settings.custom_paths.document.clone(),
            OutputCategory::Archive => settings.custom_paths.archive.clone(),
            OutputCategory::Program => settings.custom_paths.program.clone(),
            OutputCategory::Other => settings.custom_paths.other.clone(),
        },
        DownloadDirectoryMode::SystemDefault => match category {
            OutputCategory::Video => settings.system_paths.video.clone(),
            OutputCategory::Audio => settings.system_paths.audio.clone(),
            OutputCategory::Image => settings.system_paths.image.clone(),
            OutputCategory::Document => settings.system_paths.document.clone(),
            OutputCategory::Archive => settings.system_paths.archive.clone(),
            OutputCategory::Program => settings.system_paths.program.clone(),
            OutputCategory::Other => settings.system_paths.other.clone(),
        },
    }
}

fn final_filename(cmd: &CreateTaskCommand, plan: &ExecutionPlan) -> String {
    match plan.kind {
        ExecutionKind::Hls => media_filename(cmd, plan, "mp4"),
        ExecutionKind::DashMediaMerge | ExecutionKind::CapturedMediaMerge => {
            media_filename(cmd, plan, "mp4")
        }
        ExecutionKind::YouTubeExternal => media_filename(cmd, plan, "mp4"),
        ExecutionKind::HttpFile | ExecutionKind::BitTorrent => generic_filename(cmd, plan),
    }
}

/// Never silently route an invalid explicit destination to a default folder.
pub fn validate_save_path(cmd: &CreateTaskCommand) -> Result<(), String> {
    let Some(value) = cmd.save_path.as_deref() else {
        return Ok(());
    };
    let path = Path::new(value);
    if cmd.source.as_deref() != Some("download")
        || !reqwest::Url::parse(&cmd.url).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
        || !path.is_absolute()
        || path.file_name().is_none()
        || value.contains('\0')
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("save_path must be a browser-confirmed absolute file path".into());
    }
    #[cfg(windows)]
    {
        // Reject device namespaces and alternate data streams. Regular drive
        // paths and UNC shares selected in the browser remain supported.
        if value.starts_with(r"\\?\")
            || value.starts_with(r"\\.\")
            || value.get(2..).is_some_and(|tail| tail.contains(':'))
            || value.ends_with(['/', '\\'])
        {
            return Err("save_path is not a regular file path".into());
        }
    }
    Ok(())
}

fn generic_filename(cmd: &CreateTaskCommand, plan: &ExecutionPlan) -> String {
    let raw_name = cmd
        .filename
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or(Some(plan.title.as_str()))
        .unwrap_or("download");
    let safe = to_safe_filename(raw_name, "download");
    if Path::new(&safe).extension().is_some() {
        return safe;
    }
    let ext = extension_from_facts(cmd).unwrap_or_else(|| "file".to_string());
    format!("{safe}.{ext}")
}

fn media_filename(cmd: &CreateTaskCommand, plan: &ExecutionPlan, ext: &str) -> String {
    let title = to_safe_filename(&plan.title, "media");
    let suffix = media_suffix(cmd, plan).unwrap_or_else(|| {
        plan.task_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(8)
            .collect::<String>()
    });
    let base = if title.ends_with(&format!("-{suffix}")) {
        title
    } else {
        format!("{title}-{suffix}")
    };
    ensure_extension(base, ext)
}

fn ensure_extension(name: String, ext: &str) -> String {
    if name.to_ascii_lowercase().ends_with(&format!(".{ext}")) {
        name
    } else {
        format!("{name}.{ext}")
    }
}

fn media_suffix(cmd: &CreateTaskCommand, plan: &ExecutionPlan) -> Option<String> {
    for candidate in std::iter::once(cmd.url.as_str())
        .chain(cmd.resources.iter().map(|res| res.url.as_str()))
        .chain(plan.stages.iter().map(|stage| stage.url.as_str()))
    {
        if let Some(value) = bilibili_video_suffix(candidate)
            .or_else(|| twitter_media_suffix(candidate))
            .or_else(|| youtube_video_suffix(candidate))
        {
            return Some(value);
        }
    }
    None
}

fn extension_from_facts(cmd: &CreateTaskCommand) -> Option<String> {
    let candidates = std::iter::once(cmd.filename.as_deref())
        .chain(std::iter::once(Some(cmd.url.as_str())))
        .chain(cmd.resources.iter().map(|res| res.filename.as_deref()))
        .chain(cmd.resources.iter().map(|res| Some(res.url.as_str())));

    for candidate in candidates.flatten() {
        let without_query = candidate.split(['?', '#']).next().unwrap_or(candidate);
        let name = without_query
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(without_query);
        if let Some((_, ext)) = name.rsplit_once('.') {
            let ext = ext.trim().to_ascii_lowercase();
            if !ext.is_empty() && ext.len() <= 12 {
                return Some(ext);
            }
        }
    }
    None
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

fn youtube_video_suffix(raw_url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(raw_url).ok()?;
    if let Some(id) = parsed
        .query_pairs()
        .find_map(|(key, value)| (key == "v").then(|| value.to_string()))
    {
        return Some(safe_suffix(&id));
    }
    let parts: Vec<&str> = parsed
        .path_segments()
        .map(|segments| segments.collect())
        .unwrap_or_default();
    if parsed
        .host_str()
        .is_some_and(|host| host.contains("youtu.be"))
    {
        return parts.first().map(|part| safe_suffix(part));
    }
    if let Some(index) = parts
        .iter()
        .position(|part| *part == "shorts" || *part == "embed")
    {
        return parts.get(index + 1).map(|part| safe_suffix(part));
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

pub fn output_path(plan: &ExecutionPlan) -> PathBuf {
    match &plan.output {
        OutputTarget::File { path } | OutputTarget::Directory { path } => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::execution_plan::{
        ExecutionStage, MergePolicy, ResourceBudget, ResumePolicy, StageKind,
    };
    use std::collections::HashMap;

    fn command() -> CreateTaskCommand {
        CreateTaskCommand {
            request_id: "req".into(),
            source: Some("test".into()),
            title: "X-linky (@linky)".into(),
            url: "https://video.twimg.com/ext_tw_video/2063456657753944064/pu/pl/abc.m3u8".into(),
            headers: HashMap::new(),
            resources: vec![],
            client_context: None,
            decrypted_cookies: None,
            filename: None,
            save_path: None,
            mime: Some("video/mp4".into()),
            size: None,
            supports_range: None,
            payload_fingerprint: "fp".into(),
        }
    }

    fn plan() -> ExecutionPlan {
        ExecutionPlan {
            task_id: "task-1".into(),
            request_id: None,
            trace_id: None,
            source: None,
            title: "X-linky (@linky)".into(),
            output_category: None,
            output: OutputTarget::File {
                path: "old.mp4".into(),
            },
            kind: ExecutionKind::Hls,
            stages: vec![ExecutionStage {
                stage_id: "playlist".into(),
                kind: StageKind::Video,
                url: "https://video.twimg.com/ext_tw_video/2063456657753944064/pu/pl/abc.m3u8"
                    .into(),
                headers: HashMap::new(),
                expected_size: None,
                checksum: None,
                output_part: "playlist".into(),
            }],
            resume_policy: ResumePolicy::Disabled,
            merge_policy: MergePolicy::None,
            resource_budget: ResourceBudget {
                max_connections: 1,
                max_bandwidth_bytes_per_sec: None,
            },
        }
    }

    #[test]
    fn applies_category_path_before_download() {
        let settings = AppSettingsDto {
            directory_mode: crate::protocol::settings::DownloadDirectoryMode::Custom,
            custom_paths: crate::protocol::settings::CategoryDownloadPaths {
                video: "D:/videos".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let plan = apply_output_destination(&command(), plan(), &OutputCategory::Video, &settings);
        assert_eq!(
            output_path(&plan),
            PathBuf::from("D:/videos").join("X-linky (@linky)-2063456657753944064.mp4")
        );
    }

    #[test]
    fn explicit_browser_path_wins_in_every_directory_mode() {
        use crate::protocol::settings::DownloadDirectoryMode::*;
        for mode in [Flat, Categorized, Custom, SystemDefault] {
            let mut cmd = command();
            cmd.source = Some("download".into());
            cmd.filename = Some("different-name.jpg".into());
            let chosen = std::env::temp_dir()
                .join("用户选择的目录")
                .join("原图 (2).jpg");
            cmd.save_path = Some(chosen.to_string_lossy().to_string());
            assert!(validate_save_path(&cmd).is_ok());
            let settings = AppSettingsDto {
                directory_mode: mode,
                ..Default::default()
            };
            let result = apply_output_destination(&cmd, plan(), &OutputCategory::Image, &settings);
            assert_eq!(output_path(&result), chosen);
            assert_eq!(result.output_category, Some(OutputCategory::Image));
        }
    }

    #[test]
    fn invalid_explicit_paths_are_rejected_not_rerouted() {
        let mut cmd = command();
        cmd.source = Some("download".into());
        for value in ["", "relative/photo.jpg", "D:photo.jpg", "photo\0.jpg"] {
            cmd.save_path = Some(value.into());
            assert!(validate_save_path(&cmd).is_err(), "{value:?}");
        }
        cmd.save_path = Some(
            std::env::temp_dir()
                .join("../photo.jpg")
                .display()
                .to_string(),
        );
        assert!(validate_save_path(&cmd).is_err());
        cmd.save_path = Some(std::env::temp_dir().join("photo.jpg").display().to_string());
        cmd.source = Some("page_media".into());
        assert!(validate_save_path(&cmd).is_err());
        #[cfg(windows)]
        for value in [
            r"\\.\NUL",
            r"\\?\D:\photo.jpg",
            r"D:\photo.jpg:stream",
            r"D:\photos\",
        ] {
            cmd.source = Some("download".into());
            cmd.save_path = Some(value.into());
            assert!(validate_save_path(&cmd).is_err(), "{value:?}");
        }
    }

    #[test]
    fn images_have_category_directories_without_an_explicit_path() {
        let settings = AppSettingsDto::default();
        assert_eq!(
            PathBuf::from(category_directory(&OutputCategory::Image, &settings)),
            PathBuf::from("./downloads/image")
        );
    }
}
