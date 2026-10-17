use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ClientEventBatch {
    #[serde(default)]
    pub events: Vec<ClientEvent>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ClientEvent {
    #[serde(default)]
    pub occurred_at: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    pub event_type: String,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub tab_id: Option<i64>,
    #[serde(default)]
    pub page_url: Option<String>,
    #[serde(default)]
    pub url_host: Option<String>,
    #[serde(default)]
    pub url_path_kind: Option<String>,
    #[serde(default)]
    pub details: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientEventAccepted {
    pub accepted: bool,
    pub event_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredClientEvent {
    pub occurred_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_path_kind: Option<String>,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub details: Map<String, Value>,
}

impl From<ClientEvent> for StoredClientEvent {
    fn from(event: ClientEvent) -> Self {
        Self {
            occurred_at: event
                .occurred_at
                .map(|value| truncate(value, 80))
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
            trace_id: clean_optional(event.trace_id, 96),
            request_id: clean_optional(event.request_id, 128),
            task_id: clean_optional(event.task_id, 128),
            event_type: truncate(event.event_type, 96),
            surface: clean_optional(event.surface, 64),
            action: clean_optional(event.action, 96),
            source: clean_optional(event.source, 64),
            status: clean_optional(event.status, 64),
            message: clean_optional(event.message, 240),
            tab_id: event.tab_id,
            page_origin: event.page_url.as_deref().and_then(page_origin),
            url_host: clean_optional(event.url_host, 128),
            url_path_kind: clean_optional(event.url_path_kind, 32),
            details: sanitize_details(event.details),
        }
    }
}

fn clean_optional(value: Option<String>, max_len: usize) -> Option<String> {
    value
        .map(|text| truncate(text, max_len))
        .filter(|text| !text.trim().is_empty())
}

fn truncate(value: String, max_len: usize) -> String {
    value.chars().take(max_len).collect()
}

fn page_origin(value: &str) -> Option<String> {
    reqwest::Url::parse(value).ok().map(|mut parsed| {
        parsed.set_path("");
        parsed.set_query(None);
        parsed.set_fragment(None);
        parsed.to_string().trim_end_matches('/').to_string()
    })
}

fn sanitize_details(details: Map<String, Value>) -> Map<String, Value> {
    details
        .into_iter()
        .filter_map(|(key, value)| {
            if is_sensitive_key(&key) {
                None
            } else {
                Some((truncate(key, 64), sanitize_value(value, 0)))
            }
        })
        .take(32)
        .collect()
}

fn sanitize_value(value: Value, depth: usize) -> Value {
    if depth > 2 {
        return Value::String("[truncated]".to_string());
    }

    match value {
        Value::String(text) => Value::String(truncate(text, 240)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .take(16)
                .map(|item| sanitize_value(item, depth + 1))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter_map(|(key, value)| {
                    if is_sensitive_key(&key) {
                        None
                    } else {
                        Some((truncate(key, 64), sanitize_value(value, depth + 1)))
                    }
                })
                .take(32)
                .collect(),
        ),
        primitive => primitive,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| {
            if c == '-' || c.is_whitespace() {
                '_'
            } else {
                c
            }
        })
        .collect::<String>();
    if matches!(
        normalized.as_str(),
        "authorization"
            | "cookie"
            | "cookie_iv"
            | "encrypted"
            | "encrypted_cookie"
            | "encrypted_cookies"
            | "headers"
            | "iv"
            | "password"
            | "secret"
            | "token"
    ) {
        return true;
    }
    normalized.split('_').any(|part| {
        matches!(
            part,
            "authorization" | "cookie" | "token" | "secret" | "password"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn client_event_details_keep_active_count_but_redact_cookie_iv() {
        let mut details = Map::new();
        details.insert("active".to_string(), json!(2));
        details.insert("cookie_iv".to_string(), json!("secret-iv"));
        details.insert("task_title".to_string(), json!("example"));

        let sanitized = sanitize_details(details);

        assert_eq!(sanitized.get("active"), Some(&json!(2)));
        assert_eq!(sanitized.get("task_title"), Some(&json!("example")));
        assert!(!sanitized.contains_key("cookie_iv"));
    }
}
