use crate::protocol::errors::{ErrorCode, TaskError};
use crate::protocol::metrics::{AppVolumeStats, StoragePathStats};
use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use sysinfo::Disks;

pub async fn inspect_path(path: impl Into<String>) -> StoragePathStats {
    let path = path.into();
    let target = Path::new(&path);
    let exists = target.exists();
    let mut error = None;

    if let Err(err) = tokio::fs::create_dir_all(target).await {
        error = Some(TaskError {
            code: ErrorCode::InvalidRequest,
            message: err.to_string(),
            retryable: false,
            stage: Some("storage".to_string()),
        });
    }

    let writable = if error.is_none() {
        let probe = target.join(".gd3_write_probe");
        match tokio::fs::write(&probe, b"ok").await {
            Ok(()) => {
                let _ = tokio::fs::remove_file(probe).await;
                true
            }
            Err(err) => {
                error = Some(TaskError {
                    code: ErrorCode::InvalidRequest,
                    message: err.to_string(),
                    retryable: false,
                    stage: Some("storage".to_string()),
                });
                false
            }
        }
    } else {
        false
    };

    let (available_bytes, total_bytes) = disk_space_for(target);

    StoragePathStats {
        path,
        exists,
        writable,
        available_bytes,
        total_bytes,
        error,
    }
}

pub fn inspect_app_volume() -> AppVolumeStats {
    let source_path = std::env::current_exe()
        .or_else(|_| std::env::current_dir())
        .unwrap_or_else(|_| PathBuf::from("."));
    let (volume_root, available_bytes, total_bytes) = disk_space_details_for(&source_path);
    let error = if available_bytes.is_none() || total_bytes.is_none() {
        Some(TaskError {
            code: ErrorCode::InvalidRequest,
            message: "unable to resolve storage volume for application path".to_string(),
            retryable: false,
            stage: Some("storage".to_string()),
        })
    } else {
        None
    };

    AppVolumeStats {
        source_path: source_path.display().to_string(),
        volume_root: volume_root
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        available_bytes,
        total_bytes,
        error,
    }
}

fn disk_space_for(path: &Path) -> (Option<u64>, Option<u64>) {
    let (_, available_bytes, total_bytes) = disk_space_details_for(path);
    (available_bytes, total_bytes)
}

fn disk_space_details_for(path: &Path) -> (Option<PathBuf>, Option<u64>, Option<u64>) {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let volume_root = volume_root_for_path(&canonical).unwrap_or_else(|| canonical.clone());

    #[cfg(windows)]
    {
        let (available_bytes, total_bytes) = windows_disk_space_for(&volume_root);
        (Some(volume_root), available_bytes, total_bytes)
    }

    #[cfg(not(windows))]
    {
        return disk_space_details_from_sysinfo(&canonical);
    }
}

#[cfg(windows)]
fn volume_root_for_path(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    match components.next()? {
        std::path::Component::Prefix(prefix) => {
            let mut root = PathBuf::from(prefix.as_os_str());
            root.push(std::path::MAIN_SEPARATOR.to_string());
            Some(root)
        }
        _ => path.ancestors().last().map(Path::to_path_buf),
    }
}

#[cfg(not(windows))]
fn volume_root_for_path(path: &Path) -> Option<PathBuf> {
    path.ancestors().last().map(Path::to_path_buf)
}

#[cfg(windows)]
fn windows_disk_space_for(path: &Path) -> (Option<u64>, Option<u64>) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut available_bytes = 0u64;
    let mut total_bytes = 0u64;
    let mut total_free_bytes = 0u64;

    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide_path.as_ptr(),
            &mut available_bytes,
            &mut total_bytes,
            &mut total_free_bytes,
        )
    };

    if ok == 0 {
        (None, None)
    } else {
        (Some(available_bytes), Some(total_bytes))
    }
}

#[cfg(not(windows))]
fn disk_space_details_from_sysinfo(path: &Path) -> (Option<PathBuf>, Option<u64>, Option<u64>) {
    let disks = Disks::new_with_refreshed_list();
    let mut best_match_len = 0usize;
    let mut result = (None, None, None);

    for disk in disks.list() {
        let mount = disk.mount_point();
        if canonical.starts_with(mount) {
            let len = mount.as_os_str().len();
            if len >= best_match_len {
                best_match_len = len;
                result = (
                    Some(mount.to_path_buf()),
                    Some(disk.available_space()),
                    Some(disk.total_space()),
                );
            }
        }
    }

    result
}
