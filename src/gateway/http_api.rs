use crate::gateway::auth::PairingStore;
use crate::protocol::client_events::{ClientEventAccepted, ClientEventBatch, StoredClientEvent};
use crate::protocol::errors::{ErrorCode, TaskError};
use crate::protocol::settings::{PatchAppSettingsDto, SettingsUpdateResp};
use crate::protocol::task::{
    CreateTaskCommand, CreateTaskHttpReq, CreateTaskHttpResp, GetTaskEventsResp, GetTasksResp,
    TaskCommandResp, TaskSummary,
};
use crate::services::bilibili_cookie_store;
use crate::services::core_service::CoreService;
use crate::services::settings_service::SettingsService;
use crate::services::storage_service;
use crate::services::task_application_service::TaskApplicationService;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderValue, Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use axum::{extract::FromRequestParts, http::request::Parts};
use base64::Engine;
use qrcode::QrCode;
use qrcode::render::svg;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tower_http::cors::{AllowOrigin, CorsLayer};

#[derive(Clone)]
pub struct AuthToken;

impl FromRequestParts<AppState> for AuthToken {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(auth_header) = parts.headers.get(axum::http::header::AUTHORIZATION)
            && let Ok(auth_str) = auth_header.to_str()
            && auth_str.starts_with("Bearer ")
        {
            let token = auth_str.trim_start_matches("Bearer ").trim();
            if state.auth_store.validate(token).await {
                return Ok(AuthToken);
            }
        }
        Err((StatusCode::UNAUTHORIZED, "Invalid or missing Bearer token"))
    }
}

#[derive(Clone)]
pub struct AppState {
    pub core_service: Arc<CoreService>,
    pub app_service: Arc<TaskApplicationService>,
    pub settings_service: Arc<SettingsService>,
    pub auth_store: Arc<PairingStore>,
}

pub async fn create_task(
    _auth: AuthToken,
    State(state): State<AppState>,
    Json(request): Json<CreateTaskHttpReq>,
) -> impl IntoResponse {
    let request_trace_id = request
        .client_context
        .as_ref()
        .and_then(|context| context.trace_id.clone());
    let decrypted_cookies = match (
        request.payload.encrypted_cookies.as_deref(),
        request.payload.cookie_iv.as_deref(),
    ) {
        (Some(encrypted), Some(iv)) => match crate::gateway::crypto::decrypt_cookies(encrypted, iv)
        {
            Some(cookies) => Some(cookies),
            None => {
                let resp = CreateTaskHttpResp {
                    request_id: request.request_id,
                    trace_id: request_trace_id,
                    task_id: None,
                    accepted: false,
                    initial_status: None,
                    event_cursor: None,
                    error: Some(TaskError {
                        code: ErrorCode::CookieDecryptionFailed,
                        message: "failed to decrypt browser cookies".to_string(),
                        retryable: false,
                        stage: Some("gateway".to_string()),
                    }),
                };
                return (StatusCode::BAD_REQUEST, Json(resp));
            }
        },
        _ => None,
    };

    let request_url_host = url_host(&request.payload.url);
    let request_url_path_kind = url_path_kind(&request.payload.url);
    let deferred = request.payload.defer_start;
    let cmd = CreateTaskCommand::from_http(request, decrypted_cookies);
    tracing::info!(
        request_id = %cmd.request_id,
        trace_id = ?cmd.client_context.as_ref().and_then(|ctx| ctx.trace_id.as_deref()),
        source = ?cmd.source,
        title = %cmd.title,
        url_host = %request_url_host,
        url_path_kind = %request_url_path_kind,
        header_count = cmd.headers.len(),
        resource_count = cmd.resources.len(),
        tab_id = ?cmd.client_context.as_ref().and_then(|ctx| ctx.tab_id),
        page_origin = ?cmd.client_context.as_ref().and_then(|ctx| ctx.page_url.as_deref()),
        has_cookies = cmd.decrypted_cookies.is_some(),
        "create task request received"
    );

    let request_id = cmd.request_id.clone();
    let trace_id = cmd
        .client_context
        .as_ref()
        .and_then(|context| context.trace_id.clone());
    let result = if deferred {
        state.app_service.create_deferred_browser_task(cmd).await
    } else {
        state.app_service.create_task(cmd).await
    };
    match result {
        Ok(accepted) => {
            tracing::info!(
                request_id = %accepted.request_id,
                trace_id = ?accepted.trace_id,
                task_id = %accepted.task_id,
                initial_status = ?accepted.status,
                event_cursor = accepted.event_cursor,
                "create task request accepted"
            );
            let resp = CreateTaskHttpResp {
                request_id: accepted.request_id,
                trace_id: accepted.trace_id,
                task_id: Some(accepted.task_id),
                accepted: true,
                initial_status: Some(accepted.status),
                event_cursor: Some(accepted.event_cursor),
                error: None,
            };
            (StatusCode::OK, Json(resp))
        }
        Err(e) => {
            tracing::error!(
                request_id = %request_id,
                trace_id = ?trace_id,
                failure_code = ?e.code,
                stage = ?e.stage,
                retryable = e.retryable,
                "failed to create task"
            );
            let resp = CreateTaskHttpResp {
                request_id,
                trace_id,
                task_id: None,
                accepted: false,
                initial_status: None,
                event_cursor: None,
                error: Some(e),
            };
            (StatusCode::BAD_REQUEST, Json(resp))
        }
    }
}

fn url_host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_else(|| "invalid".to_string())
}

fn url_path_kind(url: &str) -> &'static str {
    let path = reqwest::Url::parse(url)
        .ok()
        .map(|parsed| parsed.path().to_ascii_lowercase())
        .unwrap_or_default();

    if path.ends_with(".m3u8") {
        "m3u8"
    } else if path.ends_with(".mp4") {
        "mp4"
    } else if path.ends_with(".m4s") {
        "m4s"
    } else {
        "other"
    }
}

pub async fn get_tasks(_auth: AuthToken, State(state): State<AppState>) -> impl IntoResponse {
    let tasks = state
        .core_service
        .get_tasks()
        .await
        .iter()
        .map(TaskSummary::from)
        .collect();
    (StatusCode::OK, Json(GetTasksResp { tasks }))
}

pub async fn get_settings(_auth: AuthToken, State(state): State<AppState>) -> impl IntoResponse {
    (StatusCode::OK, Json(state.settings_service.get().await))
}

pub async fn patch_settings(
    _auth: AuthToken,
    State(state): State<AppState>,
    Json(patch): Json<PatchAppSettingsDto>,
) -> impl IntoResponse {
    match state.settings_service.patch(patch).await {
        Ok(settings) => (
            StatusCode::OK,
            Json(SettingsUpdateResp {
                settings,
                error: None,
            }),
        ),
        Err(error) => {
            tracing::warn!(%error, "rejected settings update");
            (
                StatusCode::BAD_REQUEST,
                Json(SettingsUpdateResp {
                    settings: state.settings_service.get().await,
                    error: Some(TaskError {
                        code: ErrorCode::InvalidRequest,
                        message: error.to_string(),
                        retryable: false,
                        stage: Some("settings".to_string()),
                    }),
                }),
            )
        }
    }
}

pub async fn get_runtime_metrics(
    _auth: AuthToken,
    State(state): State<AppState>,
) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(state.core_service.runtime_metrics().await),
    )
}

#[derive(Debug, Deserialize)]
pub struct StorageStatsQuery {
    pub path: String,
}

pub async fn get_storage_path_stats(
    _auth: AuthToken,
    Query(query): Query<StorageStatsQuery>,
) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(storage_service::inspect_path(query.path).await),
    )
}

pub async fn get_storage_app_volume_stats(_auth: AuthToken) -> impl IntoResponse {
    (StatusCode::OK, Json(storage_service::inspect_app_volume()))
}

pub async fn accept_client_events(
    _auth: AuthToken,
    Json(batch): Json<ClientEventBatch>,
) -> impl IntoResponse {
    let events: Vec<StoredClientEvent> = batch
        .events
        .into_iter()
        .take(100)
        .filter(|event| !event.event_type.trim().is_empty())
        .map(StoredClientEvent::from)
        .collect();

    for event in &events {
        tracing::info!(
            trace_id = ?event.trace_id.as_deref(),
            request_id = ?event.request_id.as_deref(),
            task_id = ?event.task_id.as_deref(),
            event_type = %event.event_type,
            surface = ?event.surface.as_deref(),
            action = ?event.action.as_deref(),
            source = ?event.source.as_deref(),
            status = ?event.status.as_deref(),
            message = ?event.message.as_deref(),
            tab_id = ?event.tab_id,
            page_origin = ?event.page_origin.as_deref(),
            url_host = ?event.url_host.as_deref(),
            url_path_kind = ?event.url_path_kind.as_deref(),
            "client telemetry event"
        );
    }

    if let Err(error) = append_client_events(&events).await {
        tracing::error!(%error, "failed to persist client telemetry events");
    }

    (
        StatusCode::OK,
        Json(ClientEventAccepted {
            accepted: true,
            event_count: events.len(),
        }),
    )
}

async fn append_client_events(events: &[StoredClientEvent]) -> anyhow::Result<()> {
    if events.is_empty() {
        return Ok(());
    }

    tokio::fs::create_dir_all("logs").await?;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("logs/client-events.jsonl")
        .await?;

    for event in events {
        let line = serde_json::to_string(event)?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
    }
    file.flush().await?;
    Ok(())
}

pub async fn get_task(
    _auth: AuthToken,
    axum::extract::Path(task_id): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.core_service.get_task(&task_id).await {
        Some(task) => (StatusCode::OK, Json(Some(TaskSummary::from(&task)))),
        None => (StatusCode::NOT_FOUND, Json(None)),
    }
}

pub async fn pause_task_endpoint(
    _auth: AuthToken,
    axum::extract::Path(task_id): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.app_service.pause_task(&task_id).await {
        Ok(_) => (
            StatusCode::OK,
            Json(TaskCommandResp {
                task_id,
                command: "pause".to_string(),
                command_accepted: true,
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(TaskCommandResp {
                task_id,
                command: "pause".to_string(),
                command_accepted: false,
                error: Some(e),
            }),
        ),
    }
}

pub async fn resume_task_endpoint(
    _auth: AuthToken,
    axum::extract::Path(task_id): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.app_service.resume_task(&task_id).await {
        Ok(_) => (
            StatusCode::OK,
            Json(TaskCommandResp {
                task_id,
                command: "resume".to_string(),
                command_accepted: true,
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(TaskCommandResp {
                task_id,
                command: "resume".to_string(),
                command_accepted: false,
                error: Some(e),
            }),
        ),
    }
}

pub async fn cancel_task_endpoint(
    _auth: AuthToken,
    axum::extract::Path(task_id): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.app_service.cancel_task(&task_id).await {
        Ok(_) => (
            StatusCode::OK,
            Json(TaskCommandResp {
                task_id,
                command: "cancel".to_string(),
                command_accepted: true,
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(TaskCommandResp {
                task_id,
                command: "cancel".to_string(),
                command_accepted: false,
                error: Some(e),
            }),
        ),
    }
}

pub async fn delete_task_endpoint(
    _auth: AuthToken,
    axum::extract::Path(task_id): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.app_service.delete_task_artifacts(&task_id).await {
        Ok(_) => (
            StatusCode::OK,
            Json(TaskCommandResp {
                task_id,
                command: "delete".to_string(),
                command_accepted: true,
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(TaskCommandResp {
                task_id,
                command: "delete".to_string(),
                command_accepted: false,
                error: Some(e),
            }),
        ),
    }
}

pub async fn retry_task_endpoint(
    _auth: AuthToken,
    axum::extract::Path(task_id): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.app_service.retry_task(&task_id).await {
        Ok(_) => (
            StatusCode::OK,
            Json(TaskCommandResp {
                task_id,
                command: "retry".to_string(),
                command_accepted: true,
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(TaskCommandResp {
                task_id,
                command: "retry".to_string(),
                command_accepted: false,
                error: Some(e),
            }),
        ),
    }
}

use axum::response::sse::{Event, Sse};
use futures_util::stream::{self, Stream, StreamExt};
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;

#[derive(Deserialize)]
pub struct SubscribeTaskQuery {
    pub from_event_id: Option<u64>,
}

pub async fn get_task_events(
    _auth: AuthToken,
    axum::extract::Path(task_id): axum::extract::Path<String>,
    Query(query): Query<SubscribeTaskQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let from_id = query.from_event_id.unwrap_or(0);
    match state
        .core_service
        .get_task_events_since(&task_id, from_id)
        .await
    {
        Some(events) => (
            StatusCode::OK,
            Json(Some(GetTaskEventsResp {
                task_id,
                from_event_id: from_id,
                events,
            })),
        ),
        None => (StatusCode::NOT_FOUND, Json(None)),
    }
}

pub async fn subscribe_task(
    _auth: AuthToken,
    axum::extract::Path(task_id): axum::extract::Path<String>,
    Query(query): Query<SubscribeTaskQuery>,
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let from_id = query.from_event_id.unwrap_or(0);

    tracing::info!(task_id = %task_id, from_event_id = from_id, "task event stream subscribed");
    let missed_events = state
        .core_service
        .get_task_events_since(&task_id, from_id)
        .await
        .unwrap_or_default();

    // 2. Subscribe to new events
    let rx = state.core_service.subscribe_events();
    let broadcast_stream = BroadcastStream::new(rx);

    let task_id_clone = task_id.clone();

    // Convert broadcast stream into SSE stream, filtering by task_id
    let live_stream = broadcast_stream.filter_map(move |res| {
        let t_id = task_id_clone.clone();
        async move {
            match res {
                Ok(ev) if ev.task_id == t_id => {
                    let data = serde_json::to_string(&ev).unwrap_or_default();
                    Some(Ok(Event::default()
                        .id(ev.event_id.to_string())
                        .event("task_event")
                        .data(data)))
                }
                _ => None,
            }
        }
    });

    // Convert missed events into SSE stream
    let missed_stream = stream::iter(missed_events.into_iter().map(|ev| {
        let data = serde_json::to_string(&ev).unwrap_or_default();
        Ok(Event::default()
            .id(ev.event_id.to_string())
            .event("task_event")
            .data(data))
    }));

    // Chain missed events before live stream
    let combined_stream = missed_stream.chain(live_stream);

    Sse::new(combined_stream)
}

pub async fn subscribe_all_tasks(
    _auth: AuthToken,
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    tracing::info!("daemon task event stream subscribed");
    let live_stream =
        BroadcastStream::new(state.core_service.subscribe_events()).filter_map(|res| async move {
            match res {
                Ok(ev) => {
                    let data = serde_json::to_string(&ev).unwrap_or_default();
                    Some(Ok(Event::default()
                        .id(format!("{}:{}", ev.task_id, ev.event_id))
                        .event("task_event")
                        .data(data)))
                }
                Err(error) => {
                    tracing::warn!(%error, "daemon task event stream lagged");
                    None
                }
            }
        });

    Sse::new(live_stream)
}

#[derive(Serialize)]
pub struct QrGenerateResponse {
    pub qr_image_url: String,
    pub session_key: String,
}

pub async fn qr_generate() -> Json<Value> {
    let client = Client::new();
    let qr_req_url = "https://passport.bilibili.com/x/passport-login/web/qrcode/generate";

    match client
        .get(qr_req_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .send()
        .await
    {
        Ok(resp) => {
            if let Ok(qr_resp) = resp.json::<Value>().await
                && qr_resp["code"].as_i64() == Some(0)
            {
                let qr_url = qr_resp["data"]["url"].as_str().unwrap_or("");
                let qrcode_key = qr_resp["data"]["qrcode_key"].as_str().unwrap_or("");
                let qr_image_data_url = QrCode::new(qr_url.as_bytes()).ok().map(|code| {
                    let image = code
                        .render::<svg::Color<'_>>()
                        .min_dimensions(256, 256)
                        .build();
                    format!(
                        "data:image/svg+xml;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(image)
                    )
                });
                return Json(json!({
                    "qr_image_url": qr_url,
                    "qr_image_data_url": qr_image_data_url,
                    "session_key": qrcode_key
                }));
            }
        }
        Err(e) => {
            tracing::error!("Failed to generate QR code: {}", e);
        }
    }

    Json(json!({
        "error": "Failed to fetch QR code from Bilibili"
    }))
}

#[derive(Deserialize)]
pub struct QrStatusQuery {
    pub session_key: String,
}

pub async fn qr_status(Query(query): Query<QrStatusQuery>) -> Json<Value> {
    let client = Client::new();
    let poll_url = format!(
        "https://passport.bilibili.com/x/passport-login/web/qrcode/poll?qrcode_key={}",
        query.session_key
    );

    match client
        .get(&poll_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .send()
        .await
    {
        Ok(resp) => {
            let mut cookies = Vec::new();
            for cookie in resp.headers().get_all(reqwest::header::SET_COOKIE) {
                if let Ok(c) = cookie.to_str() {
                    cookies.push(c.to_string());
                }
            }

            if let Ok(poll_data) = resp.json::<Value>().await {
                let code = poll_data["data"]["code"].as_i64().unwrap_or(-1);

                let (login_state, message) = match code {
                    0 => {
                        let mut cookie_parts = Vec::new();
                        for c in &cookies {
                            if let Some(key_val) = c.split(';').next() {
                                cookie_parts.push(key_val.to_string());
                            }
                        }
                        let final_cookie = cookie_parts.join("; ");

                        if !final_cookie.is_empty()
                            && let Err(error) =
                                bilibili_cookie_store::save_bilibili_cookie(&final_cookie)
                        {
                            tracing::error!(
                                error = %error,
                                "failed to save encrypted Bilibili cookie"
                            );
                        }

                        ("auth_match_passed", "登录成功！大会员凭据已保存。")
                    }
                    86101 => ("qr_pending", "等待扫码..."),
                    86090 => ("auth_expired", "二维码已过期，请重新获取。"),
                    86038 => (
                        "qr_scanned_awaiting_confirmation",
                        "已扫码，请在手机端确认登录...",
                    ),
                    _ => ("unknown_error", "未知状态。"),
                };

                return Json(json!({
                    "login_state": login_state,
                    "message": message
                }));
            }
        }
        Err(e) => {
            tracing::error!("Failed to poll QR status: {}", e);
        }
    }

    Json(json!({
        "login_state": "unknown_error",
        "message": "网络请求失败。"
    }))
}

pub async fn get_me() -> Json<Value> {
    if let Some(cookie_str) = bilibili_cookie_store::load_bilibili_cookie() {
        let client = Client::new();
        if let Ok(resp) = client
            .get("https://api.bilibili.com/x/web-interface/nav")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .header("Cookie", cookie_str)
            .send()
            .await
            && let Ok(nav_data) = resp.json::<Value>().await
            && nav_data["code"].as_i64() == Some(0)
        {
            let data = &nav_data["data"];
            let is_login = data["isLogin"].as_bool().unwrap_or(false);
            if is_login {
                let uname = data["uname"].as_str().unwrap_or("Unknown");
                let face = data["face"].as_str().unwrap_or("");
                let is_vip = data["vipStatus"].as_i64().unwrap_or(0) == 1;

                return Json(json!({
                    "logged_in": true,
                    "username": uname,
                    "avatar": face,
                    "is_vip": is_vip
                }));
            }
        }
    }

    Json(json!({
        "logged_in": false
    }))
}

#[derive(Debug, Deserialize)]
pub struct PairingRequest {
    pub request_id: Option<String>,
    pub client_context: Option<crate::protocol::task::ClientContext>,
}

#[derive(Debug, Serialize)]
pub struct PairingResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub pairing_issued: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,
}

pub async fn request_pairing(
    State(state): State<AppState>,
    Json(request): Json<PairingRequest>,
) -> impl IntoResponse {
    match state.auth_store.issue_token().await {
        Ok(token) => (
            StatusCode::OK,
            Json(PairingResponse {
                request_id: request.request_id,
                pairing_issued: true,
                token: Some(token),
                error: None,
            }),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PairingResponse {
                request_id: request.request_id,
                pairing_issued: false,
                token: None,
                error: Some(TaskError {
                    code: ErrorCode::InternalError,
                    message: format!("Failed to issue pairing token: {}", e),
                    retryable: true,
                    stage: Some("gateway".to_string()),
                }),
            }),
        ),
    }
}

pub async fn revoke_pairing(_auth: AuthToken, State(state): State<AppState>) -> impl IntoResponse {
    match state.auth_store.revoke().await {
        Ok(()) => (StatusCode::OK, Json(json!({"pairing_revoked": true}))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "pairing_revoked": false,
                "error": {
                    "code": "internal_error",
                    "message": error.to_string(),
                    "retryable": false
                }
            })),
        ),
    }
}

use dashmap::DashMap;
use std::sync::LazyLock;

#[derive(Deserialize)]
pub struct SystemIconQuery {
    pub ext: String,
}

static ICON_CACHE: LazyLock<DashMap<String, (std::time::Instant, Vec<u8>)>> = LazyLock::new(DashMap::new);

pub async fn get_system_icon(Query(query): Query<SystemIconQuery>) -> impl IntoResponse {
    let mut ext = query.ext.trim().to_string();
    if !ext.starts_with('.') && !ext.is_empty() {
        ext = format!(".{}", ext);
    }

    if let Some(cached) = ICON_CACHE.get(&ext).filter(|entry| entry.0.elapsed().as_secs() < 60) {
        return (
            StatusCode::OK,
            [
                (axum::http::header::CONTENT_TYPE, "image/png"),
                (axum::http::header::CACHE_CONTROL, "public, max-age=60"),
            ],
            cached.1.clone(),
        )
            .into_response();
    }

    let ext_clone = ext.clone();
    let png_bytes_result = tokio::task::spawn_blocking(move || {
        crate::platform::system_icon::file_type_icon_png(&ext_clone)
    })
    .await;

    match png_bytes_result {
        Ok(Ok(bytes)) => {
            ICON_CACHE.insert(ext, (std::time::Instant::now(), bytes.clone()));
            (
                StatusCode::OK,
                [
                    (axum::http::header::CONTENT_TYPE, "image/png"),
                    (axum::http::header::CACHE_CONTROL, "public, max-age=60"),
                ],
                bytes,
            )
                .into_response()
        }
        _ => (StatusCode::NOT_FOUND, "Icon not found").into_response(),
    }
}

pub fn create_router(
    core_service: Arc<CoreService>,
    app_service: Arc<TaskApplicationService>,
    settings_service: Arc<SettingsService>,
    auth_store: Arc<PairingStore>,
) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin: &HeaderValue, _| {
            is_allowed_cors_origin(origin)
        }))
        .allow_methods([Method::GET, Method::POST, Method::PATCH])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE]);

    let state = AppState {
        core_service,
        app_service,
        settings_service,
        auth_store,
    };

    Router::new()
        .route("/api/capabilities", get(get_capabilities))
        .route("/api/tasks", post(create_task).get(get_tasks))
        .route(
            "/api/browser-downloads/import",
            post(import_browser_download),
        )
        .route("/api/settings", get(get_settings).patch(patch_settings))
        .route("/api/runtime/metrics", get(get_runtime_metrics))
        .route("/api/storage/path_stats", get(get_storage_path_stats))
        .route(
            "/api/storage/app_volume_stats",
            get(get_storage_app_volume_stats),
        )
        .route("/api/client-events", post(accept_client_events))
        .route("/api/tasks/stream", get(subscribe_all_tasks))
        .route("/api/tasks/{task_id}", get(get_task))
        .route("/api/tasks/{task_id}/events", get(get_task_events))
        .route("/api/tasks/{task_id}/pause", post(pause_task_endpoint))
        .route("/api/tasks/{task_id}/resume", post(resume_task_endpoint))
        .route(
            "/api/tasks/{task_id}/start",
            post(start_browser_task_endpoint),
        )
        .route("/api/tasks/{task_id}/cancel", post(cancel_task_endpoint))
        .route("/api/tasks/{task_id}/delete", post(delete_task_endpoint))
        .route("/api/tasks/{task_id}/retry", post(retry_task_endpoint))
        .route("/api/tasks/{task_id}/stream", get(subscribe_task))
        .route("/api/pairing/request", post(request_pairing))
        .route("/api/pairing/revoke", post(revoke_pairing))
        .route("/api/auth/bilibili/qr_generate", post(qr_generate))
        .route("/api/auth/bilibili/qr_status", get(qr_status))
        .route("/api/auth/bilibili/me", get(get_me))
        .route("/api/system-icon", get(get_system_icon))
        .layer(cors)
        .with_state(state)
}

async fn get_capabilities(_auth: AuthToken) -> Json<Value> {
    Json(
        json!({"browser_download_save_path": true, "browser_download_import": true, "browser_download_deferred_start": true}),
    )
}

async fn start_browser_task_endpoint(
    _auth: AuthToken,
    axum::extract::Path(task_id): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state
        .core_service
        .start_deferred_browser_task(&task_id)
        .await
    {
        Ok(()) => (StatusCode::OK, Json(json!({"command_accepted": true}))),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"command_accepted": false,"message":error.to_string()})),
        ),
    }
}

async fn import_browser_download(
    _auth: AuthToken,
    State(state): State<AppState>,
    Json(request): Json<crate::protocol::task::ImportBrowserDownloadReq>,
) -> impl IntoResponse {
    match state.app_service.import_browser_download(request).await {
        Ok(accepted) => (
            StatusCode::OK,
            Json(json!({"accepted": true, "task_id": accepted.task_id})),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"accepted": false, "error": error})),
        ),
    }
}

fn is_allowed_cors_origin(origin: &HeaderValue) -> bool {
    let origin = origin.as_bytes();
    origin.starts_with(b"chrome-extension://")
        || origin == b"http://tauri.localhost"
        || origin == b"https://tauri.localhost"
        || origin == b"tauri://localhost"
        || origin == b"http://127.0.0.1:14371"
        || origin == b"http://localhost:14371"
        || origin == b"http://127.0.0.1:3000"
        || origin == b"http://localhost:3000"
        || origin == b"http://127.0.0.1:3001"
        || origin == b"http://localhost:3001"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::protocol::http_file::HttpFileProtocolParser;
    use crate::parsers::registry::ParserRegistry;
    use crate::services::settings_service::SettingsService;
    use crate::services::task_service::TaskService;
    use futures_util::StreamExt;
    use reqwest::Client;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::time::{Duration, timeout};

    #[test]
    fn cors_allows_packaged_tauri_origins() {
        assert!(is_allowed_cors_origin(&HeaderValue::from_static(
            "http://tauri.localhost"
        )));
        assert!(is_allowed_cors_origin(&HeaderValue::from_static(
            "https://tauri.localhost"
        )));
        assert!(is_allowed_cors_origin(&HeaderValue::from_static(
            "tauri://localhost"
        )));
    }

    async fn spawn_test_http_api() -> (String, tokio::task::JoinHandle<()>) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("gd3-http-api-test-{unique}"));
        let data_dir = root.join("data");
        let config_dir = root.join("config");
        tokio::fs::create_dir_all(&data_dir).await.unwrap();
        tokio::fs::create_dir_all(&config_dir).await.unwrap();

        let task_service = Arc::new(TaskService::new(data_dir.to_string_lossy().as_ref()));
        let core_service = Arc::new(CoreService::new(
            task_service,
            data_dir.to_string_lossy().as_ref(),
        ));
        core_service.init().await.unwrap();

        let mut registry = ParserRegistry::new();
        registry.register_protocol(Box::new(HttpFileProtocolParser));
        let registry = Arc::new(registry);
        let settings_service = Arc::new(
            SettingsService::load(
                config_dir
                    .join("settings.json")
                    .to_string_lossy()
                    .to_string(),
            )
            .await,
        );
        let app_service = Arc::new(TaskApplicationService::new(
            core_service.clone(),
            registry,
            settings_service.clone(),
        ));
        let auth_store = Arc::new(PairingStore::new(
            config_dir
                .join("pairing_token.txt")
                .to_string_lossy()
                .as_ref(),
        ));

        let router = create_router(core_service, app_service, settings_service, auth_store);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn deferred_browser_download_waits_for_explicit_start() {
        let (base_url, api_handle) = spawn_test_http_api().await;
        let client = Client::new();
        let output = std::env::temp_dir().join(format!(
            "nexus-deferred-{}.txt",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let source_url = format!("http://{}/file", source_listener.local_addr().unwrap());
        let source_handle = tokio::spawn(async move {
            axum::serve(
                source_listener,
                Router::new().route("/file", get(|| async { "one-byte-compatible fixture" })),
            )
            .await
            .unwrap();
        });
        let token = client.post(format!("{base_url}/api/pairing/request")).json(&json!({"request_id":"deferred-pair","client_context":{"extension_id":"test","protocol_version":1}}))
            .send().await.unwrap().json::<Value>().await.unwrap()["token"].as_str().unwrap().to_owned();
        let created: Value = client
            .post(format!("{base_url}/api/tasks"))
            .bearer_auth(&token)
            .json(&json!({
                "request_id":"deferred-file","source":"download","title":"fixture.txt",
                "payload":{"url":source_url,"save_path":output.to_string_lossy(),"defer_start":true}
            }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(created["accepted"], true);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !output.exists(),
            "task acceptance must not write a browser-owned target"
        );
        client
            .post(format!("{base_url}/api/tasks/deferred-file/start"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let mut done = false;
        for _ in 0..100 {
            let task: Value = client
                .get(format!("{base_url}/api/tasks/deferred-file"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if task["status"] == "completed" {
                done = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(done);
        assert_eq!(
            tokio::fs::read(&output).await.unwrap(),
            b"one-byte-compatible fixture"
        );
        assert_eq!(
            client
                .post(format!("{base_url}/api/tasks/deferred-file/start"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        client
            .post(format!("{base_url}/api/tasks/deferred-file/delete"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert!(!output.exists());
        api_handle.abort();
        source_handle.abort();
    }

    #[tokio::test]
    async fn browser_file_import_preserves_bytes_paths_and_task_lifecycle() {
        let (base_url, server_handle) = spawn_test_http_api().await;
        let client = Client::new();
        let root = std::env::temp_dir().join(format!(
            "nexus-browser-import-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let token = client.post(format!("{base_url}/api/pairing/request"))
            .json(&json!({"request_id":"browser-import-pair","client_context":{"extension_id":"test","protocol_version":1}}))
            .send().await.unwrap().json::<Value>().await.unwrap()["token"].as_str().unwrap().to_string();
        let endpoint = format!("{base_url}/api/browser-downloads/import");
        assert_eq!(
            client
                .post(&endpoint)
                .json(&json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        for (index, (extension, category, url)) in [
            ("wav", "audio", "blob:https://site.test/audio"),
            ("m4a", "audio", "data:audio/mp4;base64,PRIVATE_FILE_CONTENT"),
            ("zip", "archive", "https://site.test/export"),
            ("unusual", "other", "file:///D:/original.unusual"),
        ]
        .iter()
        .enumerate()
        {
            let file = root.join(format!("用户指定位置-{index}.{extension}"));
            let bytes = format!("original bytes {index}");
            tokio::fs::write(&file, bytes.as_bytes()).await.unwrap();
            let body = json!({"request_id":format!("browser-import-{index}"),"save_path":file.to_string_lossy(),
                "source_url":url,"mime":null,"size":bytes.len()});
            let (a, b) = tokio::join!(
                client
                    .post(&endpoint)
                    .bearer_auth(&token)
                    .json(&body)
                    .send(),
                client
                    .post(&endpoint)
                    .bearer_auth(&token)
                    .json(&body)
                    .send()
            );
            let accepted: Value = a.unwrap().error_for_status().unwrap().json().await.unwrap();
            let replay: Value = b.unwrap().error_for_status().unwrap().json().await.unwrap();
            assert_eq!(accepted["accepted"], true);
            assert_eq!(accepted["task_id"], replay["task_id"]);
            let id = accepted["task_id"].as_str().unwrap();
            let summary: Value = client
                .get(format!("{base_url}/api/tasks"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let task = summary["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|task| task["task_id"] == id)
                .unwrap();
            assert_eq!(task["status"], "completed");
            assert_eq!(task["output_path"], file.to_string_lossy().as_ref());
            assert_eq!(task["output_category"], *category);
            assert_eq!(task["received_bytes"], bytes.len());
            assert!(!summary.to_string().contains("PRIVATE_FILE_CONTENT"));
            assert_eq!(tokio::fs::read(&file).await.unwrap(), bytes.as_bytes());
            // Retry must reject BEFORE cleanup, otherwise a revoked blob loses its only copy.
            assert!(
                !client
                    .post(format!("{base_url}/api/tasks/{id}/retry"))
                    .bearer_auth(&token)
                    .json(&json!({}))
                    .send()
                    .await
                    .unwrap()
                    .status()
                    .is_success()
            );
            assert!(file.is_file());
            let mut wrong = body.clone();
            wrong["save_path"] = json!(root.join("different.wav").to_string_lossy());
            assert_eq!(
                client
                    .post(&endpoint)
                    .bearer_auth(&token)
                    .json(&wrong)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::BAD_REQUEST
            );
            client
                .post(format!("{base_url}/api/tasks/{id}/delete"))
                .bearer_auth(&token)
                .json(&json!({}))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
            assert!(!file.exists());
        }
        for (path, size) in [
            (root.to_string_lossy().to_string(), 0),
            (root.join("missing.wav").to_string_lossy().to_string(), 10),
            ("relative.wav".into(), 1),
        ] {
            assert_eq!(client.post(&endpoint).bearer_auth(&token).json(&json!({
                "request_id":"invalid-file","save_path":path,"source_url":"blob:https://site.test/x","size":size
            })).send().await.unwrap().status(), StatusCode::BAD_REQUEST);
        }
        let file = root.join("size.wav");
        tokio::fs::write(&file, b"abc").await.unwrap();
        assert_eq!(client.post(&endpoint).bearer_auth(&token).json(&json!({"request_id":"wrong-size",
            "save_path":file.to_string_lossy(),"source_url":"blob:https://site.test/x","size":9
        })).send().await.unwrap().status(), StatusCode::BAD_REQUEST);
        assert_eq!(tokio::fs::read(&file).await.unwrap(), b"abc");
        server_handle.abort();
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn http_api_supports_pairing_task_idempotency_and_event_replay() {
        let (base_url, server_handle) = spawn_test_http_api().await;
        let client = Client::new();

        let unauthorized = client
            .get(format!("{base_url}/api/tasks"))
            .send()
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);

        let pairing = client
            .post(format!("{base_url}/api/pairing/request"))
            .json(&json!({
                "request_id": "pair-test",
                "client_context": {
                    "extension_id": "test-extension",
                    "protocol_version": 1
                }
            }))
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(pairing["pairing_issued"], true);
        let token = pairing["token"].as_str().unwrap();
        assert!(token.starts_with("gd3_"));

        assert_eq!(
            client
                .get(format!("{base_url}/api/capabilities"))
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        let capabilities: Value = client
            .get(format!("{base_url}/api/capabilities"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(capabilities["browser_download_save_path"], true);

        let create_body = json!({
            "request_id": "req-http-api-idempotent",
            "source": "resource",
            "title": "integration.bin",
            "payload": {
                "url": "http://127.0.0.1:9/integration.bin",
                "headers": { "referer": "http://example.test/page" },
                "filename": "integration.bin",
                "size": 12,
                "supports_range": true
            },
            "client_context": {
                "extension_id": "test-extension",
                "tab_id": 7,
                "page_url": "http://example.test/page",
                "protocol_version": 1
            }
        });

        let created = client
            .post(format!("{base_url}/api/tasks"))
            .bearer_auth(token)
            .json(&create_body)
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(created["accepted"], true);
        assert_eq!(created["request_id"], "req-http-api-idempotent");
        let task_id = created["task_id"].as_str().unwrap().to_string();

        let repeated = client
            .post(format!("{base_url}/api/tasks"))
            .bearer_auth(token)
            .json(&create_body)
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(repeated["accepted"], true);
        assert_eq!(repeated["task_id"], task_id);

        let conflict = client
            .post(format!("{base_url}/api/tasks"))
            .bearer_auth(token)
            .json(&json!({
                "request_id": "req-http-api-idempotent",
                "source": "resource",
                "title": "different.bin",
                "payload": { "url": "http://127.0.0.1:9/different.bin" }
            }))
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(conflict["accepted"], false);
        assert_eq!(conflict["error"]["code"], "idempotency_conflict");

        let events = client
            .get(format!(
                "{base_url}/api/tasks/{task_id}/events?from_event_id=0"
            ))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        let event_types: Vec<String> = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|event| event["type"].as_str().map(str::to_string))
            .collect();
        assert!(
            event_types
                .iter()
                .any(|event_type| event_type == "task_accepted")
        );
        assert!(
            event_types
                .iter()
                .any(|event_type| event_type == "task_queued")
        );

        let stream_response = client
            .get(format!(
                "{base_url}/api/tasks/{task_id}/stream?from_event_id=0"
            ))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(stream_response.status(), reqwest::StatusCode::OK);
        let mut stream = stream_response.bytes_stream();
        let first_chunk = timeout(Duration::from_secs(3), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&first_chunk);
        assert!(text.contains("task_event"));
        assert!(text.contains("task_accepted") || text.contains("task_queued"));

        server_handle.abort();
    }
}
