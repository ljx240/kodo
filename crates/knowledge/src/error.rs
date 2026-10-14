use std::io;

use thiserror::Error;

/// Recoverable errors returned by the local knowledge catalog.
#[derive(Debug, Error)]
pub enum KnowledgeError {
    #[error("invalid knowledge input")]
    InvalidInput,
    #[error("knowledge resource not found")]
    NotFound,
    #[error("knowledge scope denied")]
    ScopeDenied,
    #[error("knowledge storage unavailable")]
    StorageUnavailable,
    #[error("knowledge document version conflict")]
    RevisionConflict,
    #[error("sensitive content rejected")]
    SensitiveContent,
    #[error("unsupported knowledge format")]
    UnsupportedFormat,
    #[error("knowledge preview expired")]
    PreviewExpired,
    #[error("knowledge source changed")]
    SourceChanged,
    #[error("explicit approval is required before sending knowledge remotely")]
    RemoteApprovalRequired,
    #[error("knowledge is unavailable")]
    KnowledgeUnavailable,
    #[error("knowledge database operation failed")]
    Sqlite(#[from] rusqlite::Error),
    #[error("knowledge database file operation failed")]
    Io(#[from] io::Error),
    #[error("knowledge database data is invalid")]
    Json(#[from] serde_json::Error),
}

impl KnowledgeError {
    /// Stable, UI-facing code. Human wording belongs to the caller.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::NotFound => "not_found",
            Self::ScopeDenied => "scope_denied",
            Self::StorageUnavailable | Self::Sqlite(_) | Self::Io(_) | Self::Json(_) => {
                "storage_unavailable"
            }
            Self::RevisionConflict => "revision_conflict",
            Self::SensitiveContent => "sensitive_content",
            Self::UnsupportedFormat => "unsupported_format",
            Self::PreviewExpired => "preview_expired",
            Self::SourceChanged => "source_changed",
            Self::RemoteApprovalRequired => "knowledge_remote_approval_required",
            Self::KnowledgeUnavailable => "knowledge_unavailable",
        }
    }
}

pub type KnowledgeResult<T> = Result<T, KnowledgeError>;
