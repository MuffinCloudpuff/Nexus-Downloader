//! Desktop and daemon deliberately share the same Windows association resolver.
use base64::Engine;

#[path = "../../../src/platform/system_icon.rs"]
mod system_icon;

#[tauri::command]
pub async fn get_type_icon(ext: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        system_icon::file_type_icon_png(&ext).map(data_url)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_file_icon(path: Option<String>, ext: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        system_icon::file_icon_png(path.as_deref(), &ext).map(data_url)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn data_url(png: Vec<u8>) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}
