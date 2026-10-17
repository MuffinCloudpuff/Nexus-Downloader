use serde_json::json;
use std::env;
use std::fs;
use std::path::PathBuf;
use winreg::RegKey;
use winreg::enums::*;

pub fn auto_register(app_data_dir: &str) {
    let current_exe = match env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            tracing::warn!("Failed to get current executable path: {}", e);
            return;
        }
    };

    let manifest_path = match PathBuf::from(app_data_dir).canonicalize() {
        Ok(p) => p.join("com.gd3.nexusdownloader.json"),
        Err(_) => PathBuf::from(app_data_dir).join("com.gd3.nexusdownloader.json"),
    };

    let manifest_json = json!({
        "name": "com.gd3.nexusdownloader",
        "description": "Nexus Downloader Native Messaging Host",
        "path": current_exe.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": [
            "chrome-extension://kacpjjdipobohmfbpmnmlmfghoidnngh/"
        ]
    });

    // Write manifest
    if let Err(e) = fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest_json).unwrap(),
    ) {
        tracing::warn!("Failed to write Native Messaging manifest: {}", e);
        return;
    }

    // Write registry
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key_path = "Software\\Google\\Chrome\\NativeMessagingHosts\\com.gd3.nexusdownloader";
    match hkcu.create_subkey(key_path) {
        Ok((key, _disp)) => {
            let manifest_abs = manifest_path.canonicalize().unwrap_or(manifest_path);
            let mut path_str = manifest_abs.to_string_lossy().to_string();
            if path_str.starts_with("\\\\?\\") {
                path_str = path_str["\\\\?\\".len()..].to_string();
            }
            if let Err(e) = key.set_value("", &path_str) {
                tracing::warn!("Failed to set registry value for Native Messaging: {}", e);
            } else {
                tracing::info!("Native Messaging Host auto-registered successfully.");
            }
        }
        Err(e) => {
            tracing::warn!("Failed to open/create Native Messaging registry key: {}", e);
        }
    }
}
