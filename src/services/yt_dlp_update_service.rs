//! Project-managed, versioned official yt-dlp executables. Never resolves via PATH.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::{Mutex, OnceCell};
use tokio::time::Instant;

const RELEASE_PAGE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest";
const RELEASE_API: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(15 * 60);
const MAX_ASSET_BYTES: u64 = 128 * 1024 * 1024;
const EXECUTABLE: &str = "yt-dlp.exe";
static UPDATER: OnceCell<YtDlpUpdater> = OnceCell::const_new();

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct UpdateState {
    version: String,
    checked_at: u64,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

struct YtDlpUpdater {
    root: PathBuf,
    client: reqwest::Client,
    release_page: String,
    release_api: Option<String>,
    // Serialize bootstrap/update checks, while versioned binaries remain immutable
    // so an existing extraction can keep using the old version without a lock.
    last_attempt: Mutex<Option<Instant>>,
}

impl YtDlpUpdater {
    fn new(root: PathBuf, release_page: String) -> Result<Self> {
        Ok(Self {
            root,
            client: reqwest::Client::builder()
                .user_agent(concat!("NexusDownloader/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(120))
                .build()?,
            release_api: (release_page == RELEASE_PAGE).then(|| RELEASE_API.to_owned()),
            release_page,
            last_attempt: Mutex::new(None),
        })
    }

    fn executable_path(&self, version: &str) -> Result<PathBuf> {
        ensure!(valid_version(version), "invalid yt-dlp release version");
        Ok(self.root.join(version).join(EXECUTABLE))
    }

    async fn read_state(&self) -> Option<UpdateState> {
        let data = tokio::fs::read(self.root.join("state.json")).await.ok()?;
        let state: UpdateState = serde_json::from_slice(&data).ok()?;
        let path = self.executable_path(&state.version).ok()?;
        tokio::fs::metadata(path)
            .await
            .ok()
            .filter(|metadata| metadata.is_file() && metadata.len() > 0)?;
        Some(state)
    }

    async fn write_state(&self, state: &UpdateState) -> Result<()> {
        // The active pointer is committed only after the executable is validated.
        let pending = self.root.join("state.json.pending");
        tokio::fs::write(&pending, serde_json::to_vec_pretty(state)?).await?;
        tokio::fs::rename(pending, self.root.join("state.json")).await?;
        Ok(())
    }

    async fn ensure_current(&self) -> Result<PathBuf> {
        let mut last_attempt = self.last_attempt.lock().await;
        let installed = self.read_state().await;
        let now = unix_now();
        let fresh = installed.as_ref().is_some_and(|state| {
            now >= state.checked_at && now - state.checked_at < CHECK_INTERVAL.as_secs()
        });
        if fresh {
            return self.executable_path(&installed.as_ref().unwrap().version);
        }
        if last_attempt.is_some_and(|attempt| attempt.elapsed() < RETRY_INTERVAL) {
            if let Some(state) = installed {
                return self.executable_path(&state.version);
            }
            bail!("yt-dlp installation failed recently; automatic retry is pending");
        }
        *last_attempt = Some(Instant::now());

        match self.check_for_update(installed.as_ref()).await {
            Ok(path) => Ok(path),
            Err(error) => {
                if let Some(state) = installed {
                    tracing::warn!(%error, version = %state.version,
                        "yt-dlp update failed; keeping the installed version");
                    self.executable_path(&state.version)
                } else {
                    Err(error.context("unable to install official yt-dlp.exe"))
                }
            }
        }
    }

    async fn check_for_update(&self, installed: Option<&UpdateState>) -> Result<PathBuf> {
        tokio::fs::create_dir_all(&self.root).await?;
        // API/redirect requests only discover metadata. Executable bytes always
        // go through our UniversalDownloader, regardless of metadata source.
        let release = self.latest_release().await?;
        ensure!(
            !release.draft && !release.prerelease && valid_version(&release.tag_name),
            "expected a valid official stable yt-dlp release"
        );

        if let Some(state) = installed {
            // Date-formatted stable tags sort chronologically. Never auto-downgrade.
            if release.tag_name <= state.version {
                let mut state = state.clone();
                state.checked_at = unix_now();
                self.write_state(&state).await?;
                return self.executable_path(&state.version);
            }
        }

        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == EXECUTABLE)
            .context("official release does not contain yt-dlp.exe")?;
        let checksums = release
            .assets
            .iter()
            .find(|asset| asset.name == "SHA2-256SUMS")
            .context("official release does not contain SHA2-256SUMS")?;
        self.validate_asset_url(&asset.browser_download_url, &release.tag_name)?;
        self.validate_asset_url(&checksums.browser_download_url, &release.tag_name)?;
        let sums = self
            .client
            .get(&checksums.browser_download_url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let expected_hash = checksum_for_executable(&sums)?;

        let executable = self.executable_path(&release.tag_name)?;
        let version_dir = executable.parent().context("missing version directory")?;
        tokio::fs::create_dir_all(version_dir).await?;
        let pending = version_dir.join("yt-dlp.pending.exe");
        tracing::info!(version = %release.tag_name, path = %executable.display(),
            "downloading official yt-dlp.exe update");
        let result = self
            .download_and_verify(asset, &expected_hash, &release.tag_name, &pending)
            .await;
        if let Err(error) = result {
            let _ = tokio::fs::remove_file(&pending).await;
            return Err(error);
        }
        // No working executable or active pointer was touched during download.
        tokio::fs::rename(&pending, &executable).await?;
        self.write_state(&UpdateState {
            version: release.tag_name.clone(),
            checked_at: unix_now(),
        })
        .await?;
        tracing::info!(version = %release.tag_name, path = %executable.display(),
            "official yt-dlp update activated");
        Ok(executable)
    }

    async fn latest_release(&self) -> Result<Release> {
        if let Some(api) = &self.release_api {
            let result: Result<Release> = async {
                Ok(self
                    .client
                    .get(api)
                    .header(reqwest::header::ACCEPT, "application/vnd.github+json")
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?)
            }
            .await;
            match result {
                Ok(release) => return Ok(release),
                Err(error) => tracing::warn!(%error,
                    "version metadata API unavailable; checking public release page"),
            }
        }
        self.release_from_redirect().await
    }

    async fn release_from_redirect(&self) -> Result<Release> {
        let response = self
            .client
            .head(&self.release_page)
            .send()
            .await?
            .error_for_status()?;
        let url = response.url();
        if self.release_page == RELEASE_PAGE {
            ensure!(
                url.scheme() == "https" && url.host_str() == Some("github.com"),
                "unexpected official release redirect origin"
            );
        }
        let version = url
            .path()
            .strip_prefix("/yt-dlp/yt-dlp/releases/tag/")
            .filter(|version| valid_version(version))
            .context("unexpected official release redirect")?;
        let assets = [EXECUTABLE, "SHA2-256SUMS"]
            .into_iter()
            .map(|name| {
                Ok(Asset {
                    name: name.to_owned(),
                    browser_download_url: url
                        .join(&format!(
                            "/yt-dlp/yt-dlp/releases/download/{version}/{name}"
                        ))?
                        .to_string(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Release {
            tag_name: version.to_owned(),
            draft: false,
            prerelease: false,
            assets,
        })
    }

    fn validate_asset_url(&self, url: &str, version: &str) -> Result<()> {
        let expected = format!("https://github.com/yt-dlp/yt-dlp/releases/download/{version}/");
        if self.release_page == RELEASE_PAGE {
            ensure!(url.starts_with(&expected), "unexpected yt-dlp asset origin");
        }
        Ok(())
    }

    async fn download_and_verify(
        &self,
        asset: &Asset,
        hash: &str,
        version: &str,
        pending: &Path,
    ) -> Result<()> {
        let engine = crate::core::engine::downloader::UniversalDownloader::new();
        let token = tokio_util::sync::CancellationToken::new();
        let _cancel_on_drop = token.clone().drop_guard();
        let headers = std::collections::HashMap::from([(
            "User-Agent".to_owned(),
            concat!("NexusDownloader/", env!("CARGO_PKG_VERSION")).to_owned(),
        )]);
        tokio::time::timeout(
            Duration::from_secs(120),
            engine.download_internal_file(
                &asset.browser_download_url,
                pending,
                headers,
                MAX_ASSET_BYTES,
                token,
            ),
        )
        .await
        .context("yt-dlp core download timed out")??;

        let mut file = tokio::fs::File::open(pending).await?;
        let mut digest = Sha256::new();
        let size = file.metadata().await?.len();
        ensure!(
            size > 0 && size <= MAX_ASSET_BYTES,
            "incomplete yt-dlp download"
        );
        let mut magic = [0u8; 2];
        file.read_exact(&mut magic).await?;
        ensure!(
            &magic == b"MZ",
            "downloaded yt-dlp is not a Windows executable"
        );
        digest.update(magic);
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        drop(file);
        ensure!(
            format!("{:x}", digest.finalize()) == hash,
            "yt-dlp SHA-256 mismatch"
        );
        let output = tokio::time::timeout(
            Duration::from_secs(20),
            hidden_command(pending)
                .arg("--ignore-config")
                .arg("--version")
                .output(),
        )
        .await??;
        ensure!(output.status.success(), "downloaded yt-dlp cannot run");
        ensure!(
            String::from_utf8_lossy(&output.stdout).trim() == version,
            "downloaded yt-dlp version does not match release"
        );
        Ok(())
    }
}

fn checksum_for_executable(checksums: &str) -> Result<String> {
    for line in checksums.lines() {
        let mut columns = line.split_whitespace();
        if let (Some(hash), Some(name)) = (columns.next(), columns.next())
            && name.trim_start_matches('*') == EXECUTABLE
            && hash.len() == 64
            && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Ok(hash.to_ascii_lowercase());
        }
    }
    bail!("official checksums do not contain a valid yt-dlp.exe SHA-256")
}

fn valid_version(version: &str) -> bool {
    version.len() == 10
        && version.bytes().enumerate().all(|(index, byte)| {
            if index == 4 || index == 7 {
                byte == b'.'
            } else {
                byte.is_ascii_digit()
            }
        })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn hidden_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}

async fn updater() -> Result<&'static YtDlpUpdater> {
    UPDATER
        .get_or_try_init(|| async {
            let exe = std::env::current_exe()?;
            let directory = exe.parent().context("cannot resolve daemon directory")?;
            YtDlpUpdater::new(directory.join("data/tools/yt-dlp"), RELEASE_PAGE.to_owned())
        })
        .await
}

/// Used at the actual YouTube execution entry, including recovered/retried tasks.
pub async fn ensure_yt_dlp() -> Result<PathBuf> {
    let updater = updater().await?;
    // A background update must not stall new extractions when a usable version
    // already exists; atomic state replacement keeps these reads consistent.
    if let Some(installed) = updater.read_state().await {
        return updater.executable_path(&installed.version);
    }
    updater.ensure_current().await
}

/// Start only in the long-running daemon, never in Native Messaging subprocesses.
pub fn spawn_update_checker() {
    tokio::spawn(async {
        let mut timer = tokio::time::interval(RETRY_INTERVAL);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            let result = match updater().await {
                Ok(updater) => updater.ensure_current().await,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                tracing::warn!(%error, "yt-dlp automatic installation/check failed; will retry");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::get};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Fixture {
        root: PathBuf,
        updater: YtDlpUpdater,
        requests: Arc<AtomicUsize>,
        range_requests: Arc<AtomicUsize>,
        api_requests: Arc<AtomicUsize>,
        server: tokio::task::JoinHandle<()>,
    }

    fn test_directory() -> PathBuf {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).unwrap();
        let root =
            std::env::temp_dir().join(format!("gd3-ytdlp-test-{:x}", u64::from_le_bytes(random)));
        std::fs::create_dir(&root).unwrap();
        root
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.server.abort();
            // Only the unique temporary directory created by this fixture.
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    impl Fixture {
        async fn new(version: &str, body: &[u8], hash: &str, status: StatusCode) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(AtomicUsize::new(0));
            let counter = requests.clone();
            let range_requests = Arc::new(AtomicUsize::new(0));
            let range_counter = range_requests.clone();
            let api_requests = Arc::new(AtomicUsize::new(0));
            let api_counter = api_requests.clone();
            let metadata = serde_json::json!({
                "tag_name": version, "draft": false, "prerelease": false,
                "assets": [
                    { "name": EXECUTABLE, "browser_download_url": format!("{base}/yt-dlp/yt-dlp/releases/download/{version}/yt-dlp.exe") },
                    { "name": "SHA2-256SUMS", "browser_download_url": format!("{base}/yt-dlp/yt-dlp/releases/download/{version}/SHA2-256SUMS") }
                ]
            });
            let checksums = format!("{}  yt-dlp.exe\n", hash);
            let binary = body.to_vec();
            let redirect = format!("/yt-dlp/yt-dlp/releases/tag/{version}");
            let router = Router::new()
                .route(
                    "/release",
                    get(move || {
                        let metadata = metadata.clone();
                        let counter = api_counter.clone();
                        async move {
                            counter.fetch_add(1, Ordering::SeqCst);
                            Json(metadata)
                        }
                    }),
                )
                .route(
                    "/latest",
                    get(move || {
                        let redirect = redirect.clone();
                        let counter = counter.clone();
                        async move {
                            counter.fetch_add(1, Ordering::SeqCst);
                            if status.is_success() {
                                axum::response::Redirect::temporary(&redirect).into_response()
                            } else {
                                status.into_response()
                            }
                        }
                    }),
                )
                .route(
                    "/yt-dlp/yt-dlp/releases/tag/{version}",
                    get(|| async { StatusCode::OK }),
                )
                .route(
                    "/yt-dlp/yt-dlp/releases/download/{version}/SHA2-256SUMS",
                    get(move || {
                        let body = checksums.clone();
                        async move { body }
                    }),
                )
                .route(
                    "/yt-dlp/yt-dlp/releases/download/{version}/yt-dlp.exe",
                    get(move |headers: axum::http::HeaderMap| {
                        let body = binary.clone();
                        let counter = range_counter.clone();
                        async move {
                            let range = headers
                                .get("range")
                                .unwrap()
                                .to_str()
                                .unwrap()
                                .strip_prefix("bytes=")
                                .unwrap();
                            let (start, end) = range.split_once('-').unwrap();
                            let start: usize = start.parse().unwrap();
                            let end: usize = end.parse::<usize>().unwrap().min(body.len() - 1);
                            counter.fetch_add(1, Ordering::SeqCst);
                            (
                                StatusCode::PARTIAL_CONTENT,
                                [(
                                    "content-range",
                                    format!("bytes {start}-{end}/{}", body.len()),
                                )],
                                body[start..=end].to_vec(),
                            )
                                .into_response()
                        }
                    }),
                );
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let root = test_directory();
            let updater = YtDlpUpdater::new(root.clone(), format!("{base}/latest")).unwrap();
            Self {
                root,
                updater,
                requests,
                range_requests,
                api_requests,
                server,
            }
        }

        async fn seed_installed(&self, version: &str, checked_at: u64) -> PathBuf {
            let executable = self.updater.executable_path(version).unwrap();
            tokio::fs::create_dir_all(executable.parent().unwrap())
                .await
                .unwrap();
            tokio::fs::write(&executable, b"working old version")
                .await
                .unwrap();
            self.updater
                .write_state(&UpdateState {
                    version: version.to_owned(),
                    checked_at,
                })
                .await
                .unwrap();
            executable
        }
    }

    #[test]
    fn checksums_select_the_exact_windows_asset() {
        let hash = "a".repeat(64);
        assert_eq!(
            checksum_for_executable(&format!(
                "{}  yt-dlp.exe\n{}  yt-dlp_linux",
                hash,
                "b".repeat(64)
            ))
            .unwrap(),
            hash
        );
        assert_eq!(
            checksum_for_executable(&format!("{} *yt-dlp.exe", hash.to_uppercase())).unwrap(),
            hash
        );
        assert!(checksum_for_executable(&format!("{}  yt-dlp_arm64.exe", hash)).is_err());
        assert!(checksum_for_executable("invalid  yt-dlp.exe").is_err());
    }

    #[test]
    fn rejects_unsafe_versions_and_unofficial_asset_urls() {
        for version in ["../yt-dlp", "2026.08.19/../../evil", "nightly", "2026.8.19"] {
            assert!(!valid_version(version));
        }
        let updater = YtDlpUpdater::new(PathBuf::from("unused"), RELEASE_PAGE.to_owned()).unwrap();
        assert!(
            updater
                .validate_asset_url("https://evil.test/yt-dlp.exe", "2026.08.19")
                .is_err()
        );
        assert!(
            updater
                .validate_asset_url(
                    "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp.exe",
                    "2026.08.19"
                )
                .is_ok()
        );
    }

    #[tokio::test]
    async fn fresh_installation_does_not_query_github() {
        let fixture = Fixture::new("2026.08.19", b"unused", "unused", StatusCode::OK).await;
        let old = fixture.seed_installed("2026.03.17", unix_now()).await;
        assert_eq!(fixture.updater.ensure_current().await.unwrap(), old);
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unchanged_release_checks_once_for_concurrent_downloads() {
        let fixture = Fixture::new("2026.03.17", b"unused", "unused", StatusCode::OK).await;
        let old = fixture.seed_installed("2026.03.17", 0).await;
        let (a, b, c) = tokio::join!(
            fixture.updater.ensure_current(),
            fixture.updater.ensure_current(),
            fixture.updater.ensure_current()
        );
        for result in [a, b, c] {
            assert_eq!(result.unwrap(), old);
        }
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 1);
        let saved = fixture.updater.read_state().await.unwrap();
        assert!(saved.checked_at > 0);
    }

    #[tokio::test]
    async fn invalid_release_redirect_keeps_old_version() {
        let fixture = Fixture::new("nightly", b"unused", "unused", StatusCode::OK).await;
        let old = fixture.seed_installed("2026.03.17", 0).await;
        assert_eq!(fixture.updater.ensure_current().await.unwrap(), old);
        assert_eq!(fixture.updater.read_state().await.unwrap().checked_at, 0);
        assert_eq!(fixture.range_requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn github_failure_keeps_old_binary_and_backs_off() {
        let fixture =
            Fixture::new("unused", b"unused", "unused", StatusCode::TOO_MANY_REQUESTS).await;
        let old = fixture.seed_installed("2026.03.17", 0).await;
        assert_eq!(fixture.updater.ensure_current().await.unwrap(), old);
        assert_eq!(fixture.updater.ensure_current().await.unwrap(), old);
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.updater.read_state().await.unwrap().checked_at, 0);
    }

    #[tokio::test]
    async fn corrupt_new_release_never_replaces_active_version() {
        let fixture = Fixture::new(
            "2026.08.19",
            b"MZcorrupted download",
            &"0".repeat(64),
            StatusCode::OK,
        )
        .await;
        let old = fixture.seed_installed("2026.03.17", 0).await;
        assert_eq!(fixture.updater.ensure_current().await.unwrap(), old);
        assert_eq!(tokio::fs::read(&old).await.unwrap(), b"working old version");
        assert_eq!(
            fixture.updater.read_state().await.unwrap().version,
            "2026.03.17"
        );
        assert!(!fixture.root.join("2026.08.19/yt-dlp.pending.exe").exists());
        assert!(!fixture.root.join("2026.08.19/yt-dlp.exe").exists());
        assert!(fixture.range_requests.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn unavailable_first_install_is_an_error_not_a_path_fallback() {
        let fixture = Fixture::new(
            "unused",
            b"unused",
            "unused",
            StatusCode::SERVICE_UNAVAILABLE,
        )
        .await;
        assert!(fixture.updater.ensure_current().await.is_err());
        assert!(fixture.updater.ensure_current().await.is_err());
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 1);
        assert!(!fixture.root.join("state.json").exists());
    }

    #[tokio::test]
    async fn older_github_release_never_downgrades() {
        let fixture = Fixture::new("2026.03.17", b"unused", "unused", StatusCode::OK).await;
        let installed = fixture.seed_installed("2026.08.19", 0).await;
        assert_eq!(fixture.updater.ensure_current().await.unwrap(), installed);
        assert_eq!(
            fixture.updater.read_state().await.unwrap().version,
            "2026.08.19"
        );
    }

    #[tokio::test]
    async fn matching_checksum_but_unrunnable_binary_never_activates() {
        let body = b"this is not an executable";
        let hash = format!("{:x}", Sha256::digest(body));
        let fixture = Fixture::new("2026.08.19", body, &hash, StatusCode::OK).await;
        let installed = fixture.seed_installed("2026.03.17", 0).await;
        assert_eq!(fixture.updater.ensure_current().await.unwrap(), installed);
        assert_eq!(
            fixture.updater.read_state().await.unwrap().version,
            "2026.03.17"
        );
        assert!(!fixture.root.join("2026.08.19/yt-dlp.pending.exe").exists());
    }

    #[tokio::test]
    async fn api_is_metadata_only_and_binary_uses_our_range_engine() {
        let mut fixture = Fixture::new(
            "2026.08.19",
            b"MZcorrupted download",
            &"0".repeat(64),
            StatusCode::OK,
        )
        .await;
        fixture.updater.release_api =
            Some(fixture.updater.release_page.replace("/latest", "/release"));
        let installed = fixture.seed_installed("2026.03.17", 0).await;
        assert_eq!(fixture.updater.ensure_current().await.unwrap(), installed);
        assert_eq!(fixture.api_requests.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 0);
        assert!(fixture.range_requests.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    #[ignore = "Downloads and executes the official GitHub release; requires network access"]
    async fn official_release_download_verifies_and_runs() {
        let root = test_directory();
        let updater = YtDlpUpdater::new(root.clone(), RELEASE_PAGE.to_owned()).unwrap();
        let result = updater.ensure_current().await;
        if let Ok(path) = &result {
            let state = updater.read_state().await.unwrap();
            println!(
                "Verified official yt-dlp {} at {}",
                state.version,
                path.display()
            );
            assert_eq!(updater.ensure_current().await.unwrap(), *path);
        }
        let _ = tokio::fs::remove_dir_all(&root).await;
        result.unwrap();
    }
}
