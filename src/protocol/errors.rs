use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub stage: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    IdempotencyConflict,
    UnsupportedSite,
    UnsupportedMedia,
    UnsupportedProtocol,
    ParseFailed,
    AuthRequired,
    TokenInvalid,
    CookieDecryptionFailed,
    Http403,
    Http404,
    RangeNotSupported,
    ContentLengthMissing,
    NetworkTimeout,
    RetryExhausted,
    DiskFull,
    WriteFailed,
    MergeFailed,
    ExternalToolMissing,
    ExternalToolFailed,
    TorrentMetadataFailed,
    Canceled,
    Paused,
    TaskNotFound,
    InvalidTaskState,
    UnsupportedOperation,
    InternalError,
}
