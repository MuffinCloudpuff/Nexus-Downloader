use crate::core::models::{ArtifactState, Task, TaskStatus};
use crate::protocol::settings::{AppSettingsDto, CategoryDownloadPaths};
use crate::services::settings_service::SettingsService;
use crate::services::task_service::TaskService;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

const REMOVE_CONFIRM_DELAY: Duration = Duration::from_millis(800);

#[derive(Debug)]
enum ArtifactFsEvent {
    Removed(PathBuf),
    Renamed {
        old_path: PathBuf,
        new_path: PathBuf,
    },
    Created(PathBuf),
}

pub fn spawn_artifact_monitor(
    task_service: Arc<TaskService>,
    settings_service: Arc<SettingsService>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        run_artifact_monitor(task_service, settings_service).await;
    })
}

async fn run_artifact_monitor(
    task_service: Arc<TaskService>,
    settings_service: Arc<SettingsService>,
) {
    reconcile_known_artifacts(task_service.clone()).await;
    let mut settings = settings_service.get().await;
    let mut settings_rx = settings_service.subscribe_changes();

    loop {
        if let Err(error) = run_monitor_generation(
            task_service.clone(),
            settings.clone(),
            &mut settings_rx,
            &mut settings,
        )
        .await
        {
            tracing::warn!(%error, "artifact monitor generation stopped");
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}

async fn run_monitor_generation(
    task_service: Arc<TaskService>,
    initial_settings: AppSettingsDto,
    settings_rx: &mut tokio::sync::broadcast::Receiver<AppSettingsDto>,
    current_settings: &mut AppSettingsDto,
) -> anyhow::Result<()> {
    let roots = watched_roots(&initial_settings, &task_service).await;
    let (event_tx, mut event_rx) = mpsc::channel::<ArtifactFsEvent>(128);
    let mut generation = platform::WatchGeneration::start(&roots, event_tx)?;
    let mut task_change_rx = task_service.subscribe_task_changes();

    loop {
        tokio::select! {
            changed = settings_rx.recv() => {
                generation.stop();
                match changed {
                    Ok(next_settings) => {
                        *current_settings = next_settings;
                        return Ok(());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "artifact monitor settings channel lagged");
                        *current_settings = initial_settings.clone();
                        return Ok(());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Ok(());
                    }
                }
            }
            changed = task_change_rx.recv() => {
                match changed {
                    Ok(event) => {
                        if let Some(task) = task_service.get_task(&event.task_id).await
                            && task.status != TaskStatus::Completed {
                                continue;
                            }
                        let next_roots = watched_roots(&current_settings, &task_service).await;
                        if next_roots != roots {
                            generation.stop();
                            tracing::info!(
                                task_id = %event.task_id,
                                reason = ?event.reason,
                                "artifact monitor restarting after watched roots changed"
                            );
                            return Ok(());
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        generation.stop();
                        tracing::warn!(skipped, "artifact monitor task change channel lagged");
                        return Ok(());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        generation.stop();
                        return Ok(());
                    }
                }
            }
            event = event_rx.recv() => {
                let Some(event) = event else {
                    generation.stop();
                    return Ok(());
                };
                handle_artifact_event(task_service.clone(), roots.clone(), event).await;
            }
        }
    }
}

async fn handle_artifact_event(
    task_service: Arc<TaskService>,
    roots: Vec<PathBuf>,
    event: ArtifactFsEvent,
) {
    match event {
        ArtifactFsEvent::Renamed { old_path, new_path } => {
            if !is_under_watched_root(&new_path, &roots) {
                schedule_removed_path(task_service, old_path).await;
                return;
            }
            update_task_path_for_exact_move(task_service, old_path, new_path).await;
        }
        ArtifactFsEvent::Removed(path) => {
            schedule_removed_path(task_service, path).await;
        }
        ArtifactFsEvent::Created(path) => {
            update_missing_task_for_created_path(task_service, path).await;
        }
    }
}

async fn update_task_path_for_exact_move(
    task_service: Arc<TaskService>,
    old_path: PathBuf,
    new_path: PathBuf,
) {
    let Some(task) = completed_task_by_path(&task_service, &old_path).await else {
        return;
    };
    let new_path_text = new_path.display().to_string();
    if task_service
        .update_task_output_path(&task.id, new_path_text.clone())
        .await
        .is_some()
    {
        tracing::info!(
            task_id = %task.id,
            old_path = %old_path.display(),
            new_path = %new_path_text,
            "completed artifact path updated after filesystem move"
        );
    }
}

async fn schedule_removed_path(task_service: Arc<TaskService>, removed: PathBuf) {
    let Some(task) = completed_task_by_path(&task_service, &removed).await else {
        tracing::debug!(
            artifact_path = %removed.display(),
            artifact_path_key = %path_key(&removed),
            "artifact remove event ignored because no completed task matches path"
        );
        return;
    };

    tokio::spawn(async move {
        tokio::time::sleep(REMOVE_CONFIRM_DELAY).await;
        if tokio::fs::try_exists(&removed).await.unwrap_or(false) {
            return;
        }

        if task_service
            .mark_task_artifact_missing(&task.id, removed.display().to_string())
            .await
            .is_some()
        {
            tracing::info!(
                task_id = %task.id,
                artifact_path = %removed.display(),
                "completed artifact missing after filesystem event"
            );
        }
    });
}

async fn completed_task_by_path(task_service: &TaskService, path: &Path) -> Option<Task> {
    let normalized = path_key(path);
    task_service
        .get_tasks()
        .await
        .into_iter()
        .find(|task| task.status == TaskStatus::Completed && path_key(&task.path) == normalized)
}

async fn update_missing_task_for_created_path(task_service: Arc<TaskService>, created: PathBuf) {
    let Some(created_name) = created.file_name().map(|value| value.to_os_string()) else {
        return;
    };
    let Ok(metadata) = tokio::fs::metadata(&created).await else {
        return;
    };
    if !metadata.is_file() {
        return;
    }

    let created_size = metadata.len();
    let candidates: Vec<Task> = task_service
        .get_tasks()
        .await
        .into_iter()
        .filter(|task| {
            task.status == TaskStatus::Completed
                && Path::new(&task.path).file_name() == Some(created_name.as_os_str())
                && (task.file_size == 0 || task.file_size == created_size)
        })
        .collect();

    for task in candidates {
        let old_path = normalize_path(&task.path);
        if old_path == normalize_path(&created) {
            continue;
        }
        if tokio::fs::try_exists(&old_path).await.unwrap_or(false) {
            continue;
        }
        let created_text = created.display().to_string();
        if task_service
            .update_task_output_path(&task.id, created_text.clone())
            .await
            .is_some()
        {
            tracing::info!(
                task_id = %task.id,
                old_path = %old_path.display(),
                new_path = %created_text,
                "completed artifact path updated after filesystem create event"
            );
        }
        break;
    }
}

async fn reconcile_known_artifacts(task_service: Arc<TaskService>) {
    for task in task_service.get_tasks().await {
        if task.status != TaskStatus::Completed || task.path.trim().is_empty() {
            continue;
        }
        let path = normalize_path(&task.path);
        let path_text = path.display().to_string();
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            if matches!(
                task.artifact_state,
                ArtifactState::Unknown | ArtifactState::Missing
            ) {
                let _ = task_service
                    .mark_task_artifact_available(&task.id, path_text)
                    .await;
            }
        } else {
            let _ = task_service
                .mark_task_artifact_missing(&task.id, path_text)
                .await;
        }
    }
}

async fn watched_roots(settings: &AppSettingsDto, task_service: &TaskService) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    match settings.directory_mode {
        crate::protocol::settings::DownloadDirectoryMode::Flat => {
            push_unique_root(&mut roots, &mut seen, &settings.download_folder);
        }
        crate::protocol::settings::DownloadDirectoryMode::Categorized => {
            let base = std::path::Path::new(&settings.categorized_root);
            push_unique_root(&mut roots, &mut seen, base.join("video"));
            push_unique_root(&mut roots, &mut seen, base.join("audio"));
            push_unique_root(&mut roots, &mut seen, base.join("image"));
            push_unique_root(&mut roots, &mut seen, base.join("document"));
            push_unique_root(&mut roots, &mut seen, base.join("archive"));
            push_unique_root(&mut roots, &mut seen, base.join("program"));
            push_unique_root(&mut roots, &mut seen, base.join("other"));
        }
        crate::protocol::settings::DownloadDirectoryMode::Custom => {
            for value in category_root_values(&settings.custom_paths) {
                push_unique_root(&mut roots, &mut seen, value);
            }
        }
        crate::protocol::settings::DownloadDirectoryMode::SystemDefault => {
            for value in category_root_values(&settings.system_paths) {
                push_unique_root(&mut roots, &mut seen, value);
            }
        }
    }
    for task in task_service.get_tasks().await {
        if task.status != TaskStatus::Completed || task.path.trim().is_empty() {
            continue;
        }
        let normalized = normalize_path(&task.path);
        if let Some(parent) = normalized.parent() {
            push_unique_root(&mut roots, &mut seen, parent);
        }
    }
    roots.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    roots
}

fn category_root_values(paths: &CategoryDownloadPaths) -> [&str; 7] {
    [
        paths.video.as_str(),
        paths.audio.as_str(),
        paths.image.as_str(),
        paths.document.as_str(),
        paths.archive.as_str(),
        paths.program.as_str(),
        paths.other.as_str(),
    ]
}

fn push_unique_root(roots: &mut Vec<PathBuf>, seen: &mut HashSet<String>, path: impl AsRef<Path>) {
    let normalized = normalize_path(path);
    let key = path_key(&normalized);
    if seen.insert(key) {
        roots.push(normalized);
    }
}

fn is_under_watched_root(path: &Path, roots: &[PathBuf]) -> bool {
    let normalized = normalize_path(path);
    let normalized_key = path_key(&normalized);
    roots
        .iter()
        .any(|root| normalized_key.starts_with(&path_key(root)))
}

fn normalize_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    strip_windows_verbatim_prefix(path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    }))
}

fn path_key(path: impl AsRef<Path>) -> String {
    normalize_path(path)
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

#[cfg(windows)]
fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

#[cfg(not(windows))]
fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
}

#[cfg(windows)]
mod platform {
    use super::ArtifactFsEvent;
    use std::ffi::OsStr;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::thread;
    use tokio::sync::mpsc;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_OPERATION_ABORTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ACTION_ADDED, FILE_ACTION_REMOVED, FILE_ACTION_RENAMED_NEW_NAME,
        FILE_ACTION_RENAMED_OLD_NAME, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY,
        FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
        FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING, ReadDirectoryChangesW,
    };

    pub struct WatchGeneration {
        watchers: Vec<WatchThread>,
    }

    struct WatchThread {
        handle: isize,
        stop: Arc<AtomicBool>,
        externally_closed: Arc<AtomicBool>,
    }

    impl WatchGeneration {
        pub fn start(
            roots: &[PathBuf],
            event_tx: mpsc::Sender<ArtifactFsEvent>,
        ) -> anyhow::Result<Self> {
            let mut watchers = Vec::new();
            for root in roots {
                match WatchThread::start(root.clone(), event_tx.clone()) {
                    Ok(watcher) => watchers.push(watcher),
                    Err(error) => {
                        tracing::warn!(path = %root.display(), %error, "failed to start artifact directory watcher");
                    }
                }
            }
            Ok(Self { watchers })
        }

        pub fn stop(&mut self) {
            for watcher in &self.watchers {
                watcher.stop();
            }
            self.watchers.clear();
        }
    }

    impl Drop for WatchGeneration {
        fn drop(&mut self) {
            self.stop();
        }
    }

    impl WatchThread {
        fn start(root: PathBuf, event_tx: mpsc::Sender<ArtifactFsEvent>) -> anyhow::Result<Self> {
            std::fs::create_dir_all(&root)?;
            let handle = open_directory_handle(&root)?;
            let stop = Arc::new(AtomicBool::new(false));
            let externally_closed = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone();
            let thread_closed = externally_closed.clone();
            let handle_value = handle as isize;

            let thread_root = root.clone();
            thread::Builder::new()
                .name(format!("gd3-artifact-watch-{}", root.display()))
                .spawn(move || {
                    watch_directory_loop(
                        thread_root,
                        handle_value,
                        event_tx,
                        thread_stop,
                        thread_closed,
                    );
                })?;

            tracing::info!(path = %root.display(), "artifact directory watched");
            Ok(Self {
                handle: handle_value,
                stop,
                externally_closed,
            })
        }

        fn stop(&self) {
            self.stop.store(true, Ordering::SeqCst);
            self.externally_closed.store(true, Ordering::SeqCst);
            unsafe {
                CloseHandle(self.handle as HANDLE);
            }
        }
    }

    fn open_directory_handle(path: &Path) -> anyhow::Result<HANDLE> {
        let mut wide: Vec<u16> = OsStr::new(path.as_os_str())
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let handle = unsafe {
            CreateFileW(
                wide.as_mut_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            anyhow::bail!("CreateFileW failed for {}", path.display());
        }
        Ok(handle)
    }

    fn watch_directory_loop(
        root: PathBuf,
        handle_value: isize,
        event_tx: mpsc::Sender<ArtifactFsEvent>,
        stop: Arc<AtomicBool>,
        externally_closed: Arc<AtomicBool>,
    ) {
        let handle = handle_value as HANDLE;
        let mut buffer = vec![0u8; 64 * 1024];
        while !stop.load(Ordering::SeqCst) {
            let mut bytes_returned = 0u32;
            let ok = unsafe {
                ReadDirectoryChangesW(
                    handle,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    1,
                    FILE_NOTIFY_CHANGE_FILE_NAME
                        | FILE_NOTIFY_CHANGE_DIR_NAME
                        | FILE_NOTIFY_CHANGE_SIZE
                        | FILE_NOTIFY_CHANGE_LAST_WRITE,
                    &mut bytes_returned,
                    null_mut(),
                    None,
                )
            };
            if ok == 0 {
                let error = unsafe { GetLastError() };
                if !stop.load(Ordering::SeqCst) && error != ERROR_OPERATION_ABORTED {
                    tracing::warn!(path = %root.display(), error, "ReadDirectoryChangesW failed");
                }
                break;
            }
            if bytes_returned == 0 {
                continue;
            }
            for event in parse_notify_buffer(&root, &buffer[..bytes_returned as usize]) {
                let _ = event_tx.blocking_send(event);
            }
        }

        if !externally_closed.load(Ordering::SeqCst) {
            unsafe {
                CloseHandle(handle);
            }
        }
    }

    fn parse_notify_buffer(root: &Path, buffer: &[u8]) -> Vec<ArtifactFsEvent> {
        let mut events = Vec::new();
        let mut offset = 0usize;
        let mut pending_old_name: Option<PathBuf> = None;

        while offset + size_of::<FILE_NOTIFY_INFORMATION>() <= buffer.len() {
            let info = unsafe {
                &*(buffer
                    .as_ptr()
                    .add(offset)
                    .cast::<FILE_NOTIFY_INFORMATION>())
            };
            let name_len = info.FileNameLength as usize / size_of::<u16>();
            let name_ptr = info.FileName.as_ptr();
            let name = unsafe { std::slice::from_raw_parts(name_ptr, name_len) };
            let relative = PathBuf::from(String::from_utf16_lossy(name));
            let full_path = root.join(relative);

            match info.Action {
                FILE_ACTION_REMOVED => events.push(ArtifactFsEvent::Removed(full_path)),
                FILE_ACTION_RENAMED_OLD_NAME => pending_old_name = Some(full_path),
                FILE_ACTION_RENAMED_NEW_NAME => {
                    if let Some(old_path) = pending_old_name.take() {
                        events.push(ArtifactFsEvent::Renamed {
                            old_path,
                            new_path: full_path,
                        });
                    } else {
                        events.push(ArtifactFsEvent::Created(full_path));
                    }
                }
                FILE_ACTION_ADDED => events.push(ArtifactFsEvent::Created(full_path)),
                _ => {}
            }

            if info.NextEntryOffset == 0 {
                break;
            }
            offset += info.NextEntryOffset as usize;
        }

        if let Some(old_path) = pending_old_name {
            events.push(ArtifactFsEvent::Removed(old_path));
        }

        events
    }
}

#[cfg(not(windows))]
mod platform {
    use super::ArtifactFsEvent;
    use std::path::PathBuf;
    use tokio::sync::mpsc;

    pub struct WatchGeneration;

    impl WatchGeneration {
        pub fn start(
            _roots: &[PathBuf],
            _event_tx: mpsc::Sender<ArtifactFsEvent>,
        ) -> anyhow::Result<Self> {
            tracing::warn!("artifact monitor is only implemented on Windows");
            Ok(Self)
        }

        pub fn stop(&mut self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::{PackId, Task};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_data_dir(name: &str) -> String {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("gd3-artifact-monitor-{name}-{unique}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    #[tokio::test]
    async fn watched_roots_deduplicates_category_paths() {
        let service = TaskService::new(&temp_data_dir("dedupe"));
        let root = std::env::temp_dir().join("gd3-watch-root");
        let path = root.display().to_string();
        let mut initial_settings = AppSettingsDto::default();
        initial_settings.directory_mode = crate::protocol::settings::DownloadDirectoryMode::Custom;
        initial_settings.custom_paths = CategoryDownloadPaths {
            video: path.clone(),
            audio: path.clone(),
            image: path.clone(),
            document: path.clone(),
            archive: path.clone(),
            program: path.clone(),
            other: path,
        };
        let roots = watched_roots(&initial_settings, &service).await;
        assert_eq!(roots.len(), 1);
    }

    #[tokio::test]
    async fn watched_roots_include_completed_task_output_parent() {
        let service = TaskService::new(&temp_data_dir("history-root"));
        let configured = std::env::temp_dir().join("gd3-configured-root");
        let historical = std::env::temp_dir().join("gd3-historical-root");
        std::fs::create_dir_all(&configured).unwrap();
        std::fs::create_dir_all(&historical).unwrap();

        let mut task = Task::new(
            "task-history".to_string(),
            "video".to_string(),
            "https://example.com/video.mp4".to_string(),
            PackId::Http,
            0,
            historical.join("video.mp4").display().to_string(),
        );
        task.status = TaskStatus::Completed;
        service.add_task(task).await.unwrap();
        service
            .complete_task(
                "task-history",
                historical.join("video.mp4").display().to_string(),
            )
            .await;

        let mut initial_settings = AppSettingsDto::default();
        initial_settings.directory_mode = crate::protocol::settings::DownloadDirectoryMode::Custom;
        initial_settings.custom_paths = CategoryDownloadPaths {
            video: configured.display().to_string(),
            audio: configured.display().to_string(),
            image: configured.display().to_string(),
            document: configured.display().to_string(),
            archive: configured.display().to_string(),
            program: configured.display().to_string(),
            other: configured.display().to_string(),
        };
        let roots = watched_roots(&initial_settings, &service).await;

        assert!(roots.contains(&normalize_path(&configured)));
        assert!(roots.contains(&normalize_path(&historical)));
    }

    #[test]
    fn normalize_relative_path_anchors_to_current_dir() {
        let path = normalize_path("./downloads/video");
        assert!(path.is_absolute());
    }

    #[test]
    fn path_key_treats_windows_verbatim_prefix_as_same_path() {
        let normal = PathBuf::from(r"D:\Captures\video.mp4");
        let verbatim = PathBuf::from(r"\\?\D:\Captures\video.mp4");

        assert_eq!(path_key(normal), path_key(verbatim));
    }
}
