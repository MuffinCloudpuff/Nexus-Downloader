use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};

pub use crate::core::models::{ArtifactState, TaskStatus};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CreateTaskHttpReq {
    pub request_id: String,
    pub source: Option<String>,
    pub title: String,
    pub payload: CreateTaskPayload,
    pub client_context: Option<ClientContext>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CreateTaskPayload {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub resources: Vec<CapturedResource>,
    #[serde(default)]
    pub encrypted_cookies: Option<String>,
    #[serde(default, alias = "iv")]
    pub cookie_iv: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    /// Final absolute target confirmed by the browser (not a suggested filename).
    #[serde(default)]
    pub save_path: Option<String>,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default, alias = "supportsRange")]
    pub supports_range: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CapturedResource {
    #[serde(default, alias = "type")]
    pub kind: Option<String>,
    pub url: String,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default, alias = "file_size")]
    pub size: Option<u64>,
    #[serde(default, alias = "supportsRange")]
    pub supports_range: Option<bool>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default, alias = "pageTitle")]
    pub page_title: Option<String>,
    #[serde(default, alias = "pageUrl")]
    pub page_url: Option<String>,
}

impl CapturedResource {
    pub fn stage_kind(&self) -> &str {
        self.kind
            .as_deref()
            .or(self.mime.as_deref())
            .unwrap_or("generic_file")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ClientContext {
    pub extension_id: Option<String>,
    pub tab_id: Option<i64>,
    pub page_url: Option<String>,
    pub protocol_version: Option<u32>,
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CreateTaskCommand {
    pub request_id: String,
    pub source: Option<String>,
    pub title: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub resources: Vec<CapturedResource>,
    pub filename: Option<String>,
    #[serde(default)]
    pub save_path: Option<String>,
    pub mime: Option<String>,
    pub size: Option<u64>,
    pub supports_range: Option<bool>,
    pub client_context: Option<ClientContext>,
    pub decrypted_cookies: Option<String>,
    pub payload_fingerprint: String,
}

impl CreateTaskCommand {
    pub fn from_http(req: CreateTaskHttpReq, decrypted_cookies: Option<String>) -> Self {
        let mut command = Self {
            request_id: req.request_id,
            source: req.source,
            title: req.title,
            url: req.payload.url,
            headers: req.payload.headers,
            resources: req.payload.resources,
            filename: req.payload.filename,
            save_path: req.payload.save_path,
            mime: req.payload.mime,
            size: req.payload.size,
            supports_range: req.payload.supports_range,
            client_context: req.client_context,
            decrypted_cookies,
            payload_fingerprint: String::new(),
        };
        command.payload_fingerprint = command.compute_payload_fingerprint();
        command
    }

    pub fn effective_download_headers(&self) -> HashMap<String, String> {
        let mut headers = self.headers.clone();
        if let Some(cookie) = self.decrypted_cookies.as_deref() {
            let cookie = cookie.trim();
            if !cookie.is_empty() && !has_header_case_insensitive(&headers, "cookie") {
                headers.insert("Cookie".to_string(), cookie.to_string());
            }
        }
        headers
    }

    pub fn compute_payload_fingerprint(&self) -> String {
        let mut canonical_headers = BTreeMap::new();
        for (key, value) in &self.headers {
            canonical_headers.insert(key.to_ascii_lowercase(), value);
        }

        let resource_fingerprints: Vec<String> = self
            .resources
            .iter()
            .map(|resource| {
                serde_json::to_string(&serde_json::json!({
                    "kind": resource.kind,
                    "url": resource.url,
                    "mime": resource.mime,
                    "filename": resource.filename,
                    "size": resource.size,
                    "supports_range": resource.supports_range,
                }))
                .unwrap_or_default()
            })
            .collect();

        let mut canonical = serde_json::json!({
            "source": self.source,
            "title": self.title,
            "url": self.url,
            "filename": self.filename,
            "mime": self.mime,
            "size": self.size,
            "supports_range": self.supports_range,
            "headers": canonical_headers,
            "resources": resource_fingerprints,
            "client_context": self.client_context.as_ref().map(|context| serde_json::json!({
                "extension_id": context.extension_id,
                "tab_id": context.tab_id,
                "page_url": context.page_url,
                "protocol_version": context.protocol_version,
            })),
            "has_decrypted_cookies": self.decrypted_cookies.is_some(),
        });
        // Keep fingerprints of older requests (without save_path) compatible.
        if let Some(path) = &self.save_path {
            canonical["save_path"] = serde_json::json!(path);
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        canonical.to_string().hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

fn has_header_case_insensitive(headers: &HashMap<String, String>, target: &str) -> bool {
    headers.keys().any(|key| key.eq_ignore_ascii_case(target))
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateTaskAccepted {
    pub request_id: String,
    pub trace_id: Option<String>,
    pub task_id: String,
    pub status: TaskStatus,
    pub event_cursor: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateTaskHttpResp {
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub accepted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_status: Option<TaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_cursor: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::protocol::errors::TaskError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskCommandResp {
    pub task_id: String,
    pub command: String,
    pub command_accepted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::protocol::errors::TaskError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskSummary {
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    pub title: String,
    pub created_at: String,
    pub status: TaskStatus,
    pub artifact_state: ArtifactState,
    pub progress: f64,
    pub received_bytes: u64,
    pub file_size: u64,
    pub speed_bytes_per_sec: u64,
    pub event_cursor: u64,
    pub pack_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_category: Option<crate::parsers::policy::output_category::OutputCategory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl From<&crate::core::models::Task> for TaskSummary {
    fn from(task: &crate::core::models::Task) -> Self {
        let created_at = if task.created_at.trim().is_empty() {
            task.events
                .first()
                .map(|event| event.occurred_at.clone())
                .unwrap_or_default()
        } else {
            task.created_at.clone()
        };

        Self {
            task_id: task.id.clone(),
            request_id: task.request_id.clone(),
            trace_id: task.trace_id.clone(),
            title: task.title.clone(),
            created_at,
            status: task.status.clone(),
            artifact_state: task.artifact_state.clone(),
            progress: task.progress,
            received_bytes: task.received_bytes,
            file_size: task.file_size,
            speed_bytes_per_sec: task.speed,
            event_cursor: task.event_cursor,
            pack_name: serde_json::to_string(&task.pack_id)
                .unwrap_or_else(|_| "\"unknown\"".to_string())
                .trim_matches('"')
                .to_string(),
            output_category: task.output_category.clone(),
            output_path: Some(task.path.clone()),
            source: task.source.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GetTasksResp {
    pub tasks: Vec<TaskSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GetTaskEventsResp {
    pub task_id: String,
    pub from_event_id: u64,
    pub events: Vec<crate::core::models::TaskEvent>,
}
