use crate::gateway::crypto;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const ENCRYPTED_COOKIE_PATH: &str = "config/bili_cookie.enc.json";
const LEGACY_COOKIE_PATH: &str = "config/bili_cookie.txt";

#[derive(Debug, Deserialize, Serialize)]
struct EncryptedCookieFile {
    version: u8,
    encrypted_cookies: String,
    cookie_iv: String,
}

pub fn load_bilibili_cookie() -> Option<String> {
    let encrypted_path = Path::new(ENCRYPTED_COOKIE_PATH);
    let legacy_path = Path::new(LEGACY_COOKIE_PATH);
    match load_bilibili_cookie_from_paths(encrypted_path, legacy_path) {
        Ok(cookie) => cookie,
        Err(error) => {
            warn!(error = %error, "failed to load saved Bilibili cookie");
            None
        }
    }
}

pub fn save_bilibili_cookie(cookie: &str) -> Result<()> {
    save_bilibili_cookie_to_paths(
        cookie,
        Path::new(ENCRYPTED_COOKIE_PATH),
        Path::new(LEGACY_COOKIE_PATH),
    )
}

fn load_bilibili_cookie_from_paths(
    encrypted_path: &Path,
    legacy_path: &Path,
) -> Result<Option<String>> {
    if let Some(cookie) = read_encrypted_cookie(encrypted_path)? {
        info!("Loaded saved Bilibili cookie from encrypted local store");
        return Ok(Some(cookie));
    }

    let legacy_cookie = match fs::read_to_string(legacy_path) {
        Ok(raw) => raw.trim().to_string(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to read legacy Bilibili cookie file"),
    };

    if legacy_cookie.is_empty() {
        return Ok(None);
    }

    save_bilibili_cookie_to_paths(&legacy_cookie, encrypted_path, legacy_path)
        .context("failed to migrate legacy Bilibili cookie to encrypted local store")?;
    info!("Migrated legacy Bilibili cookie to encrypted local store");
    Ok(Some(legacy_cookie))
}

fn read_encrypted_cookie(path: &Path) -> Result<Option<String>> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to read encrypted Bilibili cookie file"),
    };

    let file: EncryptedCookieFile =
        serde_json::from_str(&raw).context("invalid encrypted Bilibili cookie file")?;
    if file.version != 1 {
        bail!(
            "unsupported encrypted Bilibili cookie file version {}",
            file.version
        );
    }

    let cookie = crypto::decrypt_cookies(&file.encrypted_cookies, &file.cookie_iv)
        .context("failed to decrypt encrypted Bilibili cookie file")?;
    let cookie = cookie.trim().to_string();
    if cookie.is_empty() {
        return Ok(None);
    }

    Ok(Some(cookie))
}

fn save_bilibili_cookie_to_paths(
    cookie: &str,
    encrypted_path: &Path,
    legacy_path: &Path,
) -> Result<()> {
    let cookie = cookie.trim();
    if cookie.is_empty() {
        bail!("refusing to save empty Bilibili cookie");
    }

    let (encrypted_cookies, cookie_iv) =
        crypto::encrypt_cookies(cookie).context("failed to encrypt Bilibili cookie")?;
    let file = EncryptedCookieFile {
        version: 1,
        encrypted_cookies,
        cookie_iv,
    };

    if let Some(parent) = encrypted_path.parent() {
        fs::create_dir_all(parent).context("failed to create Bilibili cookie config directory")?;
    }

    let tmp_path = temp_path_for(encrypted_path);
    let serialized =
        serde_json::to_string_pretty(&file).context("failed to serialize Bilibili cookie store")?;
    fs::write(&tmp_path, serialized).context("failed to write encrypted Bilibili cookie store")?;
    fs::rename(&tmp_path, encrypted_path)
        .context("failed to replace encrypted Bilibili cookie store")?;

    if legacy_path.exists()
        && let Err(error) = fs::remove_file(legacy_path)
    {
        warn!(
            path = %legacy_path.display(),
            error = %error,
            "failed to remove legacy plaintext Bilibili cookie file after encryption"
        );
    }

    Ok(())
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut tmp = path.to_path_buf();
    tmp.set_extension("tmp");
    tmp
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("gd3-{name}-{id}"))
    }

    #[test]
    fn encrypted_cookie_roundtrips_without_plaintext_file() {
        let dir = temp_dir("bili-cookie-save");
        let encrypted_path = dir.join("bili_cookie.enc.json");
        let legacy_path = dir.join("bili_cookie.txt");

        save_bilibili_cookie_to_paths("SESSDATA=secret", &encrypted_path, &legacy_path).unwrap();
        let raw = fs::read_to_string(&encrypted_path).unwrap();

        assert!(!raw.contains("SESSDATA=secret"));
        assert_eq!(
            load_bilibili_cookie_from_paths(&encrypted_path, &legacy_path).unwrap(),
            Some("SESSDATA=secret".to_string())
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn legacy_plaintext_cookie_is_migrated_then_removed() {
        let dir = temp_dir("bili-cookie-migrate");
        fs::create_dir_all(&dir).unwrap();
        let encrypted_path = dir.join("bili_cookie.enc.json");
        let legacy_path = dir.join("bili_cookie.txt");
        fs::write(&legacy_path, "SESSDATA=legacy").unwrap();

        assert_eq!(
            load_bilibili_cookie_from_paths(&encrypted_path, &legacy_path).unwrap(),
            Some("SESSDATA=legacy".to_string())
        );
        assert!(encrypted_path.exists());
        assert!(!legacy_path.exists());

        let _ = fs::remove_dir_all(dir);
    }
}
