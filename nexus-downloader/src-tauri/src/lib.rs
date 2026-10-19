mod icon;

use std::{
    net::{SocketAddr, TcpStream},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Listener, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

static HIDE_TO_TRAY_ON_CLOSE: AtomicBool = AtomicBool::new(true);
static EXPLICIT_EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);
static MAIN_WINDOW_LIVE: AtomicBool = AtomicBool::new(true);
static SPAWNED_DAEMON: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

const DAEMON_HTTP_ADDR: &str = "127.0.0.1:14371";
const AUTOSTART_VALUE_NAME: &str = "Nexus Downloader";
#[cfg(windows)]
const AUTOSTART_REG_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            gd3_get_autostart_enabled,
            gd3_set_autostart_enabled,
            icon::get_type_icon,
            icon::get_file_icon
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if HIDE_TO_TRAY_ON_CLOSE.load(Ordering::Relaxed) {
                    api.prevent_close();
                    MAIN_WINDOW_LIVE.store(false, Ordering::Relaxed);
                    let _ = window.destroy();
                }
            }
        })
        .setup(|app| {
            #[cfg(windows)]
            {
                use winreg::{enums::HKEY_CURRENT_USER, RegKey};
                let hkcu = RegKey::predef(HKEY_CURRENT_USER);
                if let Ok((key, _)) = hkcu.create_subkey("Software\\NexusDownloader") {
                    if key.get_value::<u32, _>("FirstRunAutostartSet").is_err() {
                        let _ = set_autostart_enabled(true);
                        let _ = key.set_value("FirstRunAutostartSet", &1u32);
                    }
                }
            }

            app.listen("gd3://close-behavior-changed", |event| {
                let should_hide = event.payload().trim() == "hide_to_tray";
                HIDE_TO_TRAY_ON_CLOSE.store(should_hide, Ordering::Relaxed);
            });
            ensure_daemon_running(app.handle());
            setup_tray(app.handle())?;
            if should_start_hidden_to_tray() {
                if let Some(window) = app.get_webview_window("main") {
                    MAIN_WINDOW_LIVE.store(false, Ordering::Relaxed);
                    let _ = window.destroy();
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Nexus Downloader desktop shell");

    app.run(|_app_handle, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if HIDE_TO_TRAY_ON_CLOSE.load(Ordering::Relaxed)
                && !EXPLICIT_EXIT_REQUESTED.load(Ordering::Relaxed)
            {
                api.prevent_exit();
            } else if EXPLICIT_EXIT_REQUESTED.load(Ordering::Relaxed) {
                stop_spawned_daemon();
            }
        }
    });
}

#[tauri::command]
fn gd3_get_autostart_enabled() -> Result<bool, String> {
    autostart_enabled()
}

#[tauri::command]
fn gd3_set_autostart_enabled(enabled: bool) -> Result<bool, String> {
    set_autostart_enabled(enabled)?;
    autostart_enabled()
}

fn should_start_hidden_to_tray() -> bool {
    std::env::args().any(|arg| arg == "--tray")
}

fn daemon_is_reachable() -> bool {
    let Ok(addr) = DAEMON_HTTP_ADDR.parse::<SocketAddr>() else {
        return false;
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

fn ensure_daemon_running(app: &AppHandle) {
    if daemon_is_reachable() {
        return;
    }

    let Ok(current_exe) = std::env::current_exe() else {
        eprintln!("failed to resolve current exe path for daemon launch");
        return;
    };
    let Some(app_dir) = current_exe.parent() else {
        eprintln!("failed to resolve current exe directory for daemon launch");
        return;
    };
    let daemon_path = app_dir.join("gd3-daemon.exe");
    if !daemon_path.exists() {
        eprintln!(
            "gd3-daemon.exe not found next to Nexus Downloader: {}",
            daemon_path.display()
        );
        return;
    }

    let mut command = hidden_command(&daemon_path);
    command.current_dir(app_dir);
    command.stdout(Stdio::null()).stderr(Stdio::null());

    match command.spawn() {
        Ok(child) => {
            let store = SPAWNED_DAEMON.get_or_init(|| Mutex::new(None));
            if let Ok(mut guard) = store.lock() {
                *guard = Some(child);
            }
        }
        Err(error) => {
            eprintln!("failed to launch gd3-daemon.exe: {error}");
            let _ = app.emit("gd3://daemon-launch-failed", error.to_string());
        }
    }
}

fn stop_spawned_daemon() {
    let Some(store) = SPAWNED_DAEMON.get() else {
        return;
    };
    let Ok(mut guard) = store.lock() else {
        return;
    };
    if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(windows)]
fn get_startup_shortcut_path() -> Result<std::path::PathBuf, String> {
    let appdata = std::env::var("APPDATA").map_err(|e| format!("Failed to read APPDATA: {}", e))?;
    let mut path = std::path::PathBuf::from(appdata);
    path.push(r"Microsoft\Windows\Start Menu\Programs\Startup\Nexus Downloader.lnk");
    Ok(path)
}

#[cfg(windows)]
fn autostart_enabled() -> Result<bool, String> {
    let shortcut = get_startup_shortcut_path()?;
    Ok(shortcut.exists())
}

#[cfg(not(windows))]
fn autostart_enabled() -> Result<bool, String> {
    Ok(false)
}

#[cfg(windows)]
fn set_autostart_enabled(enabled: bool) -> Result<(), String> {
    let shortcut = get_startup_shortcut_path()?;
    if enabled {
        let exe = std::env::current_exe()
            .map_err(|error| format!("failed to resolve current exe for autostart: {error}"))?;
        let mut path_str = exe.to_string_lossy().to_string();
        if path_str.starts_with("\\\\?\\") {
            path_str = path_str["\\\\?\\".len()..].to_string();
        }

        let script = format!(
            "$WshShell = New-Object -comObject WScript.Shell; \
             $Shortcut = $WshShell.CreateShortcut('{}'); \
             $Shortcut.TargetPath = '{}'; \
             $Shortcut.Arguments = '--tray'; \
             $Shortcut.Save()",
            shortcut.display(), path_str
        );

        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let output = std::process::Command::new("powershell")
            .args(&["-NoProfile", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Failed to execute powershell: {}", e))?;

        if !output.status.success() {
            return Err(format!("Powershell failed: {}", String::from_utf8_lossy(&output.stderr)));
        }
    } else {
        if shortcut.exists() {
            std::fs::remove_file(shortcut).map_err(|e| format!("Failed to remove shortcut: {}", e))?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn set_autostart_enabled(_enabled: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn hidden_command<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    use std::os::windows::process::CommandExt;

    let mut command = Command::new(program);
    command.creation_flags(0x08000000);
    command
}

#[cfg(not(windows))]
fn hidden_command<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    Command::new(program)
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Nexus Downloader", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "Close UI", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &quit])?;

    let mut tray_builder = TrayIconBuilder::with_id("main")
        .tooltip("Nexus Downloader")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => open_main_window(app),
            "hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    MAIN_WINDOW_LIVE.store(false, Ordering::Relaxed);
                    let _ = window.destroy();
                }
            }
            "quit" => {
                EXPLICIT_EXIT_REQUESTED.store(true, Ordering::Relaxed);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                open_main_window(tray.app_handle());
            }
        });

    let icon_bytes = include_bytes!("../icons/icon.ico");
    if let Ok(icon) = tauri::image::Image::from_bytes(icon_bytes) {
        tray_builder = tray_builder.icon(icon);
    } else if let Some(icon) = app.default_window_icon() {
        tray_builder = tray_builder.icon(icon.clone());
    }

    tray_builder.build(app)?;

    Ok(())
}

fn open_main_window(app: &AppHandle) {
    if MAIN_WINDOW_LIVE.load(Ordering::Relaxed) {
        if let Some(window) = app.get_webview_window("main") {
            if window.show().and_then(|_| window.set_focus()).is_ok() {
                return;
            }
            MAIN_WINDOW_LIVE.store(false, Ordering::Relaxed);
            let _ = window.destroy();
        }
    }

    match create_main_window(app) {
        Ok(window) => {
            MAIN_WINDOW_LIVE.store(true, Ordering::Relaxed);
            let _ = window.set_focus();
        }
        Err(error) => {
            MAIN_WINDOW_LIVE.store(false, Ordering::Relaxed);
            eprintln!("failed to create Nexus Downloader UI window: {error}");
        }
    }
}

fn create_main_window(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Nexus Downloader")
        .inner_size(1000.0, 680.0)
        .min_inner_size(1000.0, 680.0)
        .center()
        .build()
}
