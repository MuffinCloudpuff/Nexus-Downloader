pub mod client_events;
pub mod errors;
pub mod events;
pub mod metrics;
pub mod settings;
pub mod task;

#[cfg(test)]
mod tests {
    use super::errors::{ErrorCode, TaskError};
    use super::events::{ErrorPayload, TaskEvent, TaskEventPayload};
    use super::task::{
        CapturedResource, ClientContext, CreateTaskCommand, CreateTaskHttpReq, CreateTaskPayload,
        TaskStatus,
    };
    use std::collections::HashMap;

    #[test]
    fn test_create_task_command_roundtrip() {
        let cmd = CreateTaskCommand {
            request_id: "req_123".to_string(),
            source: Some("extension".to_string()),
            title: "Test Video".to_string(),
            url: "https://example.com/video".to_string(),
            headers: HashMap::from([("User-Agent".to_string(), "Test".to_string())]),
            resources: vec![CapturedResource {
                kind: Some("video".to_string()),
                url: "https://example.com/video.mp4".to_string(),
                mime: Some("video/mp4".to_string()),
                ..Default::default()
            }],
            filename: Some("video.mp4".to_string()),
            mime: Some("video/mp4".to_string()),
            size: None,
            supports_range: None,
            client_context: Some(ClientContext {
                extension_id: Some("ext_1".to_string()),
                tab_id: Some(42),
                page_url: Some("https://example.com/page".to_string()),
                protocol_version: Some(1),
                trace_id: Some("trace_1".to_string()),
            }),
            decrypted_cookies: None,
            payload_fingerprint: "fp".to_string(),
        };

        let json_str = serde_json::to_string(&cmd).unwrap();
        let decoded: CreateTaskCommand = serde_json::from_str(&json_str).unwrap();

        assert_eq!(cmd.request_id, decoded.request_id);
        assert_eq!(cmd.title, decoded.title);
        assert_eq!(
            cmd.headers.get("User-Agent"),
            decoded.headers.get("User-Agent")
        );
        assert_eq!(decoded.resources[0].url, "https://example.com/video.mp4");
    }

    #[test]
    fn test_create_task_http_request_to_command() {
        let req = CreateTaskHttpReq {
            request_id: "req_456".to_string(),
            source: Some("page_media".to_string()),
            title: "Captured".to_string(),
            payload: CreateTaskPayload {
                url: "https://example.com/master.m3u8".to_string(),
                headers: HashMap::from([(
                    "referer".to_string(),
                    "https://example.com".to_string(),
                )]),
                resources: vec![],
                encrypted_cookies: None,
                cookie_iv: None,
                filename: None,
                mime: None,
                size: None,
                supports_range: None,
            },
            client_context: None,
        };

        let cmd = CreateTaskCommand::from_http(req, Some("SESSDATA=secret".to_string()));
        assert_eq!(cmd.url, "https://example.com/master.m3u8");
        assert!(!cmd.headers.contains_key("cookie"));
        assert_eq!(
            cmd.effective_download_headers().get("Cookie"),
            Some(&"SESSDATA=secret".to_string())
        );
        assert_eq!(cmd.decrypted_cookies, Some("SESSDATA=secret".to_string()));
        assert!(!cmd.payload_fingerprint.is_empty());
    }

    #[test]
    fn test_task_status_roundtrip() {
        let status = TaskStatus::Downloading;
        let json_str = serde_json::to_string(&status).unwrap();
        assert_eq!(json_str, "\"downloading\"");

        let decoded: TaskStatus = serde_json::from_str(&json_str).unwrap();
        assert_eq!(status, decoded);

        let resuming_json = serde_json::to_string(&TaskStatus::Resuming).unwrap();
        assert_eq!(resuming_json, "\"resuming\"");
        let decoded_resuming: TaskStatus = serde_json::from_str(&resuming_json).unwrap();
        assert_eq!(decoded_resuming, TaskStatus::Resuming);
    }

    #[test]
    fn test_task_event_roundtrip() {
        let event = TaskEvent {
            event_id: 1,
            task_id: "task_123".to_string(),
            request_id: Some("req_123".to_string()),
            trace_id: Some("trace_123".to_string()),
            occurred_at: "2026-06-05T20:35:10+08:00".to_string(),
            payload: TaskEventPayload::TaskFailed {
                failure: ErrorPayload {
                    code: "http_404".to_string(),
                    message: "Not Found".to_string(),
                    retryable: false,
                    stage: Some("downloading".to_string()),
                },
            },
        };

        let json_str = serde_json::to_string(&event).unwrap();
        let decoded: TaskEvent = serde_json::from_str(&json_str).unwrap();

        assert_eq!(event.event_id, decoded.event_id);
        if let TaskEventPayload::TaskFailed { failure } = decoded.payload {
            assert_eq!(failure.code, "http_404");
        } else {
            panic!("Wrong payload type");
        }
    }

    #[test]
    fn test_task_error_roundtrip() {
        let err = TaskError {
            code: ErrorCode::UnsupportedSite,
            message: "Site not supported".to_string(),
            retryable: false,
            stage: None,
        };

        let json_str = serde_json::to_string(&err).unwrap();
        let decoded: TaskError = serde_json::from_str(&json_str).unwrap();

        assert_eq!(err.code, decoded.code);
        assert_eq!(err.message, decoded.message);
    }
}
