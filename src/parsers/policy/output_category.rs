use crate::core::execution_plan::{ExecutionKind, ExecutionPlan, StageKind};
use crate::protocol::task::CreateTaskCommand;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputCategory {
    Video,
    Audio,
    Image,
    Document,
    Archive,
    Program,
    Other,
}

impl OutputCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            OutputCategory::Video => "video",
            OutputCategory::Audio => "audio",
            OutputCategory::Image => "image",
            OutputCategory::Document => "document",
            OutputCategory::Archive => "archive",
            OutputCategory::Program => "program",
            OutputCategory::Other => "other",
        }
    }
}

pub fn classify_output(cmd: &CreateTaskCommand, plan: &ExecutionPlan) -> OutputCategory {
    if matches!(
        plan.kind,
        ExecutionKind::Hls
            | ExecutionKind::DashMediaMerge
            | ExecutionKind::CapturedMediaMerge
            | ExecutionKind::YouTubeExternal
    ) {
        return OutputCategory::Video;
    }

    if plan
        .stages
        .iter()
        .any(|stage| matches!(stage.kind, StageKind::Video))
    {
        return OutputCategory::Video;
    }
    if plan
        .stages
        .iter()
        .any(|stage| matches!(stage.kind, StageKind::Audio))
    {
        return OutputCategory::Audio;
    }

    for mime in mime_candidates(cmd) {
        if let Some(category) = classify_mime(mime) {
            return category;
        }
    }

    for ext in extension_candidates(cmd, plan) {
        if let Some(category) = classify_extension(&ext) {
            return category;
        }
    }

    OutputCategory::Other
}

fn mime_candidates(cmd: &CreateTaskCommand) -> Vec<&str> {
    let mut values = Vec::new();
    if let Some(mime) = cmd.mime.as_deref() {
        values.push(mime);
    }
    for resource in &cmd.resources {
        if let Some(mime) = resource.mime.as_deref() {
            values.push(mime);
        }
        if let Some(kind) = resource.kind.as_deref() {
            values.push(kind);
        }
    }
    values
}

fn extension_candidates(cmd: &CreateTaskCommand, plan: &ExecutionPlan) -> Vec<String> {
    let mut values = Vec::new();
    push_extension(&mut values, cmd.filename.as_deref());
    push_extension(&mut values, Some(&cmd.url));
    push_extension(&mut values, Some(&cmd.title));
    for resource in &cmd.resources {
        push_extension(&mut values, resource.filename.as_deref());
        push_extension(&mut values, Some(&resource.url));
    }
    for stage in &plan.stages {
        push_extension(&mut values, Some(&stage.url));
    }
    values
}

fn push_extension(values: &mut Vec<String>, candidate: Option<&str>) {
    let Some(candidate) = candidate else {
        return;
    };
    let without_query = candidate.split(['?', '#']).next().unwrap_or(candidate);
    let name = without_query
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(without_query);
    let Some((_, ext)) = name.rsplit_once('.') else {
        return;
    };
    let ext = ext.trim().to_ascii_lowercase();
    if !ext.is_empty() && ext.len() <= 12 {
        values.push(ext);
    }
}

fn classify_mime(mime: &str) -> Option<OutputCategory> {
    let lower = mime.to_ascii_lowercase();
    if lower.starts_with("video/") || lower.contains("mpegurl") || lower.contains("m3u8") {
        Some(OutputCategory::Video)
    } else if lower.starts_with("audio/") {
        Some(OutputCategory::Audio)
    } else if lower.starts_with("image/") {
        Some(OutputCategory::Image)
    } else if lower.contains("javascript")
        || lower.contains("typescript")
        || lower.contains("python")
        || lower.contains("x-csrc")
        || lower.contains("x-c++src")
        || lower.contains("x-csharp")
        || lower.contains("x-java")
        || lower.contains("x-rust")
        || lower.contains("x-go")
        || lower.contains("x-shellscript")
        || lower.contains("x-php")
        || lower.contains("x-ruby")
        || lower.contains("wasm")
    {
        Some(OutputCategory::Program)
    } else if lower.starts_with("text/")
        || lower.contains("pdf")
        || lower.contains("document")
        || lower.contains("spreadsheet")
        || lower.contains("presentation")
    {
        Some(OutputCategory::Document)
    } else if lower.contains("zip")
        || lower.contains("rar")
        || lower.contains("7z")
        || lower.contains("tar")
        || lower.contains("gzip")
    {
        Some(OutputCategory::Archive)
    } else if lower.contains("executable")
        || lower.contains("installer")
        || lower.contains("android.package")
    {
        Some(OutputCategory::Program)
    } else {
        None
    }
}

fn classify_extension(ext: &str) -> Option<OutputCategory> {
    match ext {
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "flv" | "m4v" | "m3u8" | "mpd" | "wmv" | "mpg"
        | "mpeg" | "3gp" | "ogv" | "vob" | "mts" | "m2ts" | "h264" | "h265" | "hevc" => {
            Some(OutputCategory::Video)
        }
        "mp3" | "flac" | "wav" | "m4a" | "aac" | "ogg" | "opus" => Some(OutputCategory::Audio),
        "jpg" | "jpeg" | "jfif" | "png" | "gif" | "webp" | "avif" | "bmp" | "svg" | "tif"
        | "tiff" | "ico" | "heic" | "heif" | "jxl" => Some(OutputCategory::Image),
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "csv"
        | "json" | "xml" => Some(OutputCategory::Document),
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" => Some(OutputCategory::Archive),
        "exe" | "msi" | "deb" | "dmg" | "apk" | "pkg" | "appimage" | "wasm" | "c" | "h" | "cc"
        | "cpp" | "cxx" | "hpp" | "hxx" | "m" | "mm" | "cs" | "java" | "kt" | "kts" | "swift"
        | "go" | "rs" | "py" | "pyw" | "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "php"
        | "rb" | "pl" | "pm" | "lua" | "r" | "jl" | "dart" | "scala" | "sc" | "clj" | "cljs"
        | "ex" | "exs" | "erl" | "hrl" | "hs" | "lhs" | "ml" | "mli" | "fs" | "fsi" | "fsx"
        | "vb" | "zig" | "nim" | "cr" | "d" | "pas" | "pp" | "asm" | "s" | "sh" | "bash"
        | "zsh" | "fish" | "ps1" | "bat" | "cmd" | "vbs" | "sql" | "html" | "css" | "scss"
        | "sass" | "less" | "vue" | "svelte" | "glsl" | "hlsl" | "wgsl" | "ipynb" | "cmake"
        | "gradle" => Some(OutputCategory::Program),
        _ => None,
    }
}

pub fn is_image_path(path: &str) -> bool {
    let mut extensions = Vec::new();
    push_extension(&mut extensions, Some(path));
    extensions
        .iter()
        .any(|ext| classify_extension(ext) == Some(OutputCategory::Image))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::execution_plan::{
        ExecutionStage, MergePolicy, OutputTarget, ResourceBudget, ResumePolicy,
    };
    use std::collections::HashMap;

    fn command(filename: Option<&str>, mime: Option<&str>) -> CreateTaskCommand {
        CreateTaskCommand {
            request_id: "req".into(),
            source: Some("test".into()),
            title: filename.unwrap_or("download").into(),
            url: "https://example.com/download".into(),
            headers: HashMap::new(),
            resources: vec![],
            client_context: None,
            decrypted_cookies: None,
            filename: filename.map(str::to_string),
            save_path: None,
            mime: mime.map(str::to_string),
            size: None,
            supports_range: None,
            payload_fingerprint: "fp".into(),
        }
    }

    fn plan(kind: ExecutionKind) -> ExecutionPlan {
        ExecutionPlan {
            task_id: "task".into(),
            request_id: None,
            trace_id: None,
            source: None,
            title: "download".into(),
            output_category: None,
            output: OutputTarget::File {
                path: "download.file".into(),
            },
            kind,
            stages: vec![ExecutionStage {
                stage_id: "file".into(),
                kind: StageKind::GenericFile,
                url: "https://example.com/download".into(),
                headers: HashMap::new(),
                expected_size: None,
                checksum: None,
                output_part: "file".into(),
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
    fn classifies_by_mime() {
        assert_eq!(
            classify_output(
                &command(Some("unknown.bin"), Some("video/mp4")),
                &plan(ExecutionKind::HttpFile)
            ),
            OutputCategory::Video
        );
    }

    #[test]
    fn classifies_images_by_mime_or_extension() {
        for ext in [
            "jpg", "JPEG", "jfif", "png", "webp", "avif", "gif", "svg", "heic", "tiff", "ico",
            "jxl",
        ] {
            assert_eq!(
                classify_output(
                    &command(Some(&format!("photo.{ext}")), None),
                    &plan(ExecutionKind::HttpFile)
                ),
                OutputCategory::Image
            );
        }
        assert_eq!(
            classify_output(
                &command(Some("download"), Some("image/jpeg; charset=binary")),
                &plan(ExecutionKind::HttpFile)
            ),
            OutputCategory::Image
        );
        assert_eq!(
            classify_output(
                &command(Some("photo.svg"), Some("image/svg+xml")),
                &plan(ExecutionKind::HttpFile)
            ),
            OutputCategory::Image
        );
        assert_eq!(
            classify_output(
                &command(Some("file.zip"), None),
                &plan(ExecutionKind::HttpFile)
            ),
            OutputCategory::Archive
        );
    }

    #[test]
    fn classifies_by_extension() {
        assert_eq!(
            classify_output(
                &command(Some("invoice.pdf"), None),
                &plan(ExecutionKind::HttpFile)
            ),
            OutputCategory::Document
        );
    }

    #[test]
    fn classifies_source_code_as_program() {
        assert_eq!(
            classify_output(
                &command(Some("main.rs"), None),
                &plan(ExecutionKind::HttpFile)
            ),
            OutputCategory::Program
        );
        assert_eq!(
            classify_output(
                &command(Some("script.py"), Some("text/x-python")),
                &plan(ExecutionKind::HttpFile)
            ),
            OutputCategory::Program
        );
    }
}
