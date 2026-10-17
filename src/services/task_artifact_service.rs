use crate::core::models::Task;
use crate::core::resume::ResumeStore;
use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskArtifactCleanup {
    pub removed_paths: Vec<String>,
    pub skipped_paths: Vec<String>,
}

pub async fn cleanup_task_artifacts(
    task: &Task,
    resume_store: &ResumeStore,
) -> Result<TaskArtifactCleanup> {
    let mut cleanup = TaskArtifactCleanup::default();

    remove_output_file(&task.path, &mut cleanup).await?;
    remove_known_work_dir(PathBuf::from("temp").join(&task.id), &mut cleanup).await?;
    remove_youtube_parts_dir(task, &mut cleanup).await?;
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
    let Ok(metadata) = tokio::fs::metadata(&path).await else {
        return Ok(());
    };
    if metadata.is_dir() {
        tokio::fs::remove_dir_all(&path).await?;
        cleanup.removed_paths.push(path.display().to_string());
    } else {
        cleanup.skipped_paths.push(path.display().to_string());
    }
    Ok(())
}

async fn remove_youtube_parts_dir(task: &Task, cleanup: &mut TaskArtifactCleanup) -> Result<()> {
    if task.path.trim().is_empty() {
        return Ok(());
    }
    let output_parent = Path::new(&task.path)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let parts_dir = output_parent.join(format!(".{}.parts", task.id));
    remove_known_work_dir(parts_dir, cleanup).await
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
        assert!(cleanup.removed_paths.len() >= 3);
    }
}
