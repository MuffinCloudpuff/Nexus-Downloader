use crate::core::models::Task;
use crate::core::resume::ResumeStore;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskArtifactCleanup {
    pub removed_paths: Vec<String>,
    pub skipped_paths: Vec<String>,
}

/// A task's media workspace is always a sibling of its final output, never a
/// system-temp or installation-directory fallback. Keep the historical YouTube
/// name so existing adjacent partial downloads remain discoverable.
pub fn task_work_dir(task: &Task) -> Result<PathBuf> {
    validate_task_id(&task.id)?;
    let output = Path::new(&task.path);
    anyhow::ensure!(
        !task.path.trim().is_empty() && output.file_name().is_some(),
        "Task has no output file path"
    );
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Ok(parent.join(format!(".{}.parts", task.id)))
}

fn validate_task_id(id: &str) -> Result<()> {
    anyhow::ensure!(
        !id.is_empty()
            && id != "."
            && id != ".."
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')),
        "Invalid task ID for temporary-file cleanup"
    );
    Ok(())
}

/// Successful downloads remove only temporary data, not the finished artifact.
/// Also recognize old application-relative folders for retry/delete compatibility.
pub async fn cleanup_task_temporary_files(task: &Task) -> Result<TaskArtifactCleanup> {
    let work_dir = task_work_dir(task)?;
    let mut cleanup = TaskArtifactCleanup::default();
    remove_known_work_dir(work_dir, &mut cleanup).await?;
    let mut legacy_roots = vec![std::env::current_dir()?.join("temp")];
    if let Some(parent) = std::env::current_exe()?.parent() {
        let root = parent.join("temp");
        if !legacy_roots.contains(&root) {
            legacy_roots.push(root);
        }
    }
    for root in legacy_roots {
        // The ID was validated as one safe filename component before joining.
        remove_known_work_dir(root.join(&task.id), &mut cleanup).await?;
    }
    Ok(cleanup)
}

pub async fn cleanup_task_artifacts(
    task: &Task,
    resume_store: &ResumeStore,
) -> Result<TaskArtifactCleanup> {
    // Validate before deleting anything, including the final output.
    task_work_dir(task)?;
    let mut cleanup = cleanup_task_temporary_files(task).await?;
    remove_output_file(&task.path, &mut cleanup).await?;
    resume_store.delete_manifest(&task.id).await?;

    Ok(cleanup)
}

async fn remove_output_file(path: &str, cleanup: &mut TaskArtifactCleanup) -> Result<()> {
    if path.trim().is_empty() {
        return Ok(());
    }
    let path = PathBuf::from(path);
    let Ok(metadata) = tokio::fs::metadata(&path).await else {
        return Ok(());
    };
    if metadata.is_file() {
        tokio::fs::remove_file(&path).await?;
        cleanup.removed_paths.push(path.display().to_string());
    } else {
        cleanup.skipped_paths.push(path.display().to_string());
    }
    Ok(())
}

async fn remove_known_work_dir(path: PathBuf, cleanup: &mut TaskArtifactCleanup) -> Result<()> {
    let metadata = match tokio::fs::symlink_metadata(&path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Cannot inspect workspace {}", path.display()));
        }
    };
    anyhow::ensure!(
        !metadata.file_type().is_symlink(),
        "Refusing to remove linked workspace {}",
        path.display()
    );
    if metadata.is_dir() {
        tokio::fs::remove_dir_all(&path).await?;
        cleanup.removed_paths.push(path.display().to_string());
    } else {
        cleanup.skipped_paths.push(path.display().to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::PackId;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("gd3-artifacts-{name}-{unique}"));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[tokio::test]
    async fn cleanup_removes_output_temp_parts_and_manifest() {
        let root = temp_root("cleanup");
        let output = root.join("video.mp4");
        let temp_dir = PathBuf::from("temp").join("task-cleanup");
        let parts_dir = root.join(".task-cleanup.parts");
        let resume_dir = root.join("resumes");

        tokio::fs::write(&output, b"final").await.unwrap();
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();
        tokio::fs::write(temp_dir.join("chunk.part"), b"part")
            .await
            .unwrap();
        tokio::fs::create_dir_all(&parts_dir).await.unwrap();
        tokio::fs::write(parts_dir.join("stream.mp4"), b"part")
            .await
            .unwrap();

        let resume_store = ResumeStore::new(resume_dir.to_string_lossy().as_ref());
        resume_store.init().await.unwrap();
        let manifest = crate::core::resume::ResumeManifest::new(
            "task-cleanup".to_string(),
            output.display().to_string(),
        );
        resume_store.save_manifest(&manifest).await.unwrap();
        tokio::fs::write(
            resume_dir.join("task-cleanup.manifest.json.tmp"),
            b"interrupted save",
        )
        .await
        .unwrap();

        let task = Task::new(
            "task-cleanup".to_string(),
            "video".to_string(),
            "https://example.test/video.mp4".to_string(),
            PackId::Http,
            0,
            output.display().to_string(),
        );

        let cleanup = cleanup_task_artifacts(&task, &resume_store).await.unwrap();

        assert!(!output.exists());
        assert!(!temp_dir.exists());
        assert!(!parts_dir.exists());
        assert!(!resume_dir.join("task-cleanup.manifest.json").exists());
        assert!(!resume_dir.join("task-cleanup.manifest.json.tmp").exists());
        assert!(cleanup.removed_paths.len() >= 3);
    }

    fn media_task(id: &str, output: &Path) -> Task {
        Task::new(
            id.to_string(),
            "video".to_string(),
            "https://example.test/media".to_string(),
            PackId::Hls,
            0,
            output.display().to_string(),
        )
    }

    #[test]
    fn workspace_is_a_sibling_even_for_bare_output_filenames() {
        let task = media_task("task-one", Path::new("video.mp4"));
        assert_eq!(
            task_work_dir(&task).unwrap(),
            Path::new(".").join(".task-one.parts")
        );
    }

    #[cfg(windows)]
    #[test]
    fn workspace_follows_target_drive_not_installation_drive() {
        for drive in ["C", "D", "E"] {
            let output = PathBuf::from(format!("{drive}:\\Downloads\\video.mp4"));
            let workspace = task_work_dir(&media_task("task-one", &output)).unwrap();
            assert_eq!(workspace.parent(), output.parent());
            assert_eq!(workspace.file_name().unwrap(), ".task-one.parts");
        }
    }

    #[test]
    fn workspace_rejects_unsafe_ids_and_missing_outputs() {
        for id in [
            "",
            ".",
            "..",
            "../other",
            "..\\other",
            "D:\\other",
            "a/b",
            "a:b",
        ] {
            assert!(
                task_work_dir(&media_task(id, Path::new("video.mp4"))).is_err(),
                "{id}"
            );
        }
        assert!(task_work_dir(&media_task("safe", Path::new(""))).is_err());
    }

    #[tokio::test]
    async fn successful_cleanup_preserves_output_and_other_task_workspaces() {
        let root = temp_root("success");
        let task = media_task("success-task", &root.join("video.mp4"));
        let work = task_work_dir(&task).unwrap();
        let other_task = media_task("other-task", &root.join("other.mp4"));
        let other_work = task_work_dir(&other_task).unwrap();
        for directory in [&work, &other_work] {
            tokio::fs::create_dir_all(directory.join("hls/audio"))
                .await
                .unwrap();
            tokio::fs::write(directory.join("hls/audio/00000.ts"), b"audio")
                .await
                .unwrap();
        }
        tokio::fs::write(&task.path, b"complete video")
            .await
            .unwrap();
        cleanup_task_temporary_files(&task).await.unwrap();
        cleanup_task_temporary_files(&task).await.unwrap(); // idempotent
        assert!(!work.exists());
        assert!(other_work.join("hls/audio/00000.ts").exists());
        assert_eq!(
            tokio::fs::read(&task.path).await.unwrap(),
            b"complete video"
        );
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn invalid_id_does_not_delete_finished_output() {
        let root = temp_root("invalid-id");
        let task = media_task("../escape", &root.join("keep.mp4"));
        tokio::fs::write(&task.path, b"keep").await.unwrap();
        let store = ResumeStore::new(root.join("resumes").to_str().unwrap());
        assert!(cleanup_task_artifacts(&task, &store).await.is_err());
        assert!(Path::new(&task.path).exists());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
