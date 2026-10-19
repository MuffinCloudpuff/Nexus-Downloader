use gd3_daemon::gateway;
use gd3_daemon::gateway::auth::PairingStore;
use gd3_daemon::parsers::protocol::{
    bittorrent::BitTorrentProtocolParser, captured_merge::CapturedMergeProtocolParser,
    dash::DashProtocolParser, hls::HlsProtocolParser, http_file::HttpFileProtocolParser,
    youtube::YoutubeProtocolParser,
};
use gd3_daemon::parsers::registry::ParserRegistry;
use gd3_daemon::parsers::site::{
    bilibili::BilibiliSiteParser, twitter::TwitterSiteParser, youtube::YoutubeSiteParser,
};
use gd3_daemon::services::artifact_monitor_service;
use gd3_daemon::services::core_service::CoreService;
use gd3_daemon::services::settings_service::SettingsService;
use gd3_daemon::services::site_performance_store;
use gd3_daemon::services::task_application_service::TaskApplicationService;
use gd3_daemon::services::task_service::TaskService;
use std::fs::OpenOptions;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let is_native_messaging = args.contains(&"--native-messaging".to_string())
        || args.iter().any(|a| a.starts_with("chrome-extension://"));

    let _log_guard = if !is_native_messaging {
        init_logging()
    } else {
        None // Do not log to stdout in native messaging mode (interferes with stdout communication)
    };

    if !is_native_messaging {
        tracing::info!("Nexus Downloader Rust Daemon starting...");
    }

    let exe_dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    // 1. 初始化可持久化配置
    let config_dir = exe_dir.join("config");
    let _ = tokio::fs::create_dir_all(&config_dir).await;
    // Browser native-messaging processes only provide pairing. They must not
    // load/recover the task store or start another scheduler before returning.
    if is_native_messaging {
        let auth_store = Arc::new(PairingStore::new(config_dir.join("pairing_token.txt")));
        gd3_daemon::native_messaging::start_native_messaging_host(auth_store).await;
        return;
    }
    let settings_path = config_dir.join("settings.json");
    let settings_service = Arc::new(SettingsService::load(settings_path.to_str().unwrap()).await);

    // 2. 初始化持久化引擎 (防抖刷盘)
    let app_data_dir_path = exe_dir.join("data");
    let app_data_dir = app_data_dir_path.to_str().unwrap();
    let _ = tokio::fs::create_dir_all(app_data_dir).await;

    // 自动注册 Native Messaging
    if !is_native_messaging {
        gd3_daemon::native_messaging_setup::auto_register(app_data_dir);
    }

    site_performance_store::init_site_performance_store(format!(
        "{app_data_dir}/site_performance.json"
    ))
    .await;
    let task_service = Arc::new(TaskService::new(app_data_dir));

    // 3. 初始化并发核心调度器
    let core_service = Arc::new(CoreService::new(task_service.clone(), app_data_dir));
    core_service.init().await.unwrap();
    artifact_monitor_service::spawn_artifact_monitor(
        task_service.clone(),
        settings_service.clone(),
    );

    // 4. 初始化和注册功能包 (新版 ParserRegistry)
    let mut registry = ParserRegistry::new();

    // 注册 SiteParsers
    registry.register_site(Box::new(BilibiliSiteParser::new()));
    registry.register_site(Box::new(YoutubeSiteParser));
    registry.register_site(Box::new(TwitterSiteParser));

    // 注册 ProtocolParsers
    registry.register_protocol(Box::new(DashProtocolParser));
    registry.register_protocol(Box::new(YoutubeProtocolParser));
    registry.register_protocol(Box::new(CapturedMergeProtocolParser));
    registry.register_protocol(Box::new(HlsProtocolParser));
    registry.register_protocol(Box::new(BitTorrentProtocolParser));
    registry.register_protocol(Box::new(HttpFileProtocolParser));

    let registry = Arc::new(registry);

    let app_service = Arc::new(TaskApplicationService::new(
        core_service.clone(),
        registry.clone(),
        settings_service.clone(),
    ));
    let auth_store = Arc::new(PairingStore::new(config_dir.join("pairing_token.txt")));

    // 6. HTTP API 是浏览器扩展和桌面 UI 的主协议入口，主进程围绕它常驻。
    let app =
        gateway::http_api::create_router(core_service, app_service, settings_service, auth_store);
    let http_addr =
        std::env::var("GD3_HTTP_ADDR").unwrap_or_else(|_| "127.0.0.1:14371".to_string());
    let listener = tokio::net::TcpListener::bind(&http_addr)
        .await
        .unwrap_or_else(|error| panic!("Failed to bind HTTP listener {http_addr}: {error}"));
    tracing::info!("🚀 HTTP REST API listening on: http://{}", http_addr);
    gd3_daemon::services::yt_dlp_update_service::spawn_update_checker();
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|error| panic!("HTTP REST API stopped unexpectedly: {error}"));
}

fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    if let Err(error) = std::fs::create_dir_all("logs") {
        eprintln!("failed to create logs directory: {error}");
    }

    match OpenOptions::new()
        .create(true)
        .append(true)
        .open("logs/daemon.log")
    {
        Ok(file) => {
            let (non_blocking, guard) = tracing_appender::non_blocking(file);
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::INFO)
                .with_target(false)
                .with_ansi(false)
                .with_writer(non_blocking)
                .init();
            Some(guard)
        }
        Err(error) => {
            eprintln!("failed to open logs/daemon.log: {error}");
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::INFO)
                .with_target(false)
                .with_ansi(false)
                .init();
            None
        }
    }
}
