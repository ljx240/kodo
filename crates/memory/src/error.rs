//! Memory error type. All callers must treat these as recoverable — the
//! memory engine is fail-open: a `StorageUnavailable` or `IndexUnavailable`
//! must never abort the agent loop.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("memory storage is not available at {0}")]
    StorageUnavailable(PathBuf),

    #[error("memory index is unavailable: {0}")]
    IndexUnavailable(String),

    #[error("memory scope denied: {0}")]
    ScopeDenied(String),

    #[error("memory evidence is stale: {0}")]
    StaleEvidence(String),

    #[error("memory budget exceeded: requested {requested}, available {available}")]
    BudgetExceeded { requested: usize, available: usize },

    #[error("memory event is invalid: {0}")]
    InvalidEvent(String),

    #[error("memory entry is quarantined: {0}")]
    Quarantined(String),

    #[error("memory migration required (current={current}, required={required})")]
    MigrationRequired { current: u32, required: u32 },

    #[error("memory sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("memory io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("memory json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("memory not found: {0}")]
    NotFound(String),
}

pub type MemoryResult<T> = Result<T, MemoryError>;

/// Whether the error is a "memory is fine, we just couldn't get a hit" — the
/// kind callers should swallow and proceed. Anything else is unexpected and
/// should be logged.
pub fn is_recoverable(err: &MemoryError) -> bool {
    matches!(
        err,
        MemoryError::StorageUnavailable(_)
            | MemoryError::IndexUnavailable(_)
            | MemoryError::ScopeDenied(_)
            | MemoryError::StaleEvidence(_)
            | MemoryError::BudgetExceeded { .. }
            | MemoryError::InvalidEvent(_)
            | MemoryError::Quarantined(_)
            | MemoryError::NotFound(_)
    )
}
