use std::env;
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

fn main() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let exe = std::env::current_exe().unwrap();
    let mut path_str = exe.to_string_lossy().to_string();
    if path_str.starts_with("\\\\?\\") {
        path_str = path_str["\\\\?\\".len()..].to_string();
    }
    let command = format!("\"{}\" --tray", path_str);

    println!("Trying to set Run key to: {}", command);

    let key_result = hkcu.open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        winreg::enums::KEY_SET_VALUE,
    );
    match key_result {
        Ok(key) => match key.set_value("Nexus Downloader Test", &command) {
            Ok(_) => println!("Successfully set Run key."),
            Err(e) => println!("Failed to set Run key: {}", e),
        },
        Err(e) => println!("Failed to open subkey: {}", e),
    }
}
