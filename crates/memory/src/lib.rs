//! Kodo Memory Engine (KME).
//!
//! A fail-open, locally-stored, version-aware memory layer for the Kodo
//! agent. Spec: `docs/memory/KODO_MEMORY_IMPLEMENTATION.md`.
//!
//! The crate exposes a single trait [`MemoryAccess`] and a single
//! newtype [`MemoryHandle`]. The default implementation is [`SqliteMemory`].
//!
//! ```ignore
//! use kodo_memory::{MemoryHandle, SqliteMemory, MemoryQuery, MemoryScope};
//! use std::sync::Arc;
//! let handle = SqliteMemory::open("memory.sqlite")?;
//! let query = MemoryQuery::inert(MemoryScope::default());
//! let pack = handle.recall(&query)?;
//! # Ok::<_, kodo_memory::MemoryError>(())
//! ```

pub mod compiler;
pub mod doctor;
pub mod embedding;
pub mod error;
pub mod evidence;
pub mod fts;
pub mod ingest;
pub mod metrics;
pub mod model;
pub mod model_store;
pub mod outbox;
pub mod policy;
pub mod retrieval;
pub mod sanitizer;
pub mod scope;
pub mod store;
pub mod validity;

#[cfg(any(feature = "download", feature = "onnx-inference"))]
pub mod embedding_local;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use error::{MemoryError, MemoryResult};
pub use metrics::{Metrics, MetricsSnapshot};
pub use model::{
    Authority, MemoryConfig, MemoryDiagnostic, MemoryEntry, MemoryEventKind, MemoryFilter,
    MemoryKind, MemoryMode, MemoryOperation, MemoryPack, MemoryPrivacy, MemoryQuery, MemoryScope,
    MemorySnippet, MemoryStatus, MemoryUsage, SafeMemoryEvent, SourceTrust, ValidityLabel,
};
pub use scope::GitContext;
pub use store::Store;

/// The trait every consumer uses. `MemoryHandle` is `Arc<dyn MemoryAccess>`
/// under the hood, so the trait is the public surface — concrete impls
/// stay an implementation detail.
pub trait MemoryAccess: Send + Sync {
    /// Look up memories matching the query. Empty pack when nothing
    /// matches; error only on storage / index failure.
    fn recall(&self, query: &MemoryQuery) -> Result<MemoryPack, MemoryError>;

    /// Hand the engine a sanitized turn event. Idempotent on `event_id`.
    /// Never blocks the caller: a storage error increments `fail_open`
    /// and returns `Ok(false)`.
    fn observe(&self, event: &SafeMemoryEvent) -> Result<bool, MemoryError>;

    /// Mark files changed during a turn. The engine marks every entry
    /// that references one of these paths as `stale` and records the new
    /// digest. Non-fatal on failure.
    fn mark_changed_paths(&self, scope: &MemoryScope, paths: &[String]) -> Result<(), MemoryError>;

    /// Record per-recall usage feedback. Optional; the pack itself does
    /// not require a follow-up.
    fn feedback(&self, usage: &MemoryUsage) -> Result<(), MemoryError>;

    /// List entries. Used by `kodo memory list` and the GUI inspector.
    fn list(&self, filter: &MemoryFilter) -> Result<Vec<MemoryEntry>, MemoryError>;

    /// Fetch one entry by id.
    fn inspect(&self, id: &str) -> Result<Option<MemoryEntry>, MemoryError>;

    /// Soft-disable an entry. Rejected, kept for history.
    fn disable(&self, id: &str) -> Result<(), MemoryError>;

    /// Permanent delete + tombstone. The outbox worker will refuse to
    /// revive a memory whose id is in the tombstone table.
    fn tombstone(&self, id: &str, reason: &str, by: &str) -> Result<(), MemoryError>;

    /// True when the id is in the tombstone table. Cheap; the FFI
    /// surface uses it before showing a recalled row.
    fn is_tombstoned(&self, id: &str) -> Result<bool, MemoryError>;

    /// Read-only metrics snapshot. The GUI surfaces these in a status bar.
    fn metrics(&self) -> MetricsSnapshot;
}

/// Cheap to clone (Arc under the hood), so it can travel through
/// `RunRequest::memory` by value without dragging the trait object
/// signature everywhere.
#[derive(Clone)]
pub struct MemoryHandle {
    inner: Arc<dyn MemoryAccess>,
}

impl MemoryHandle {
    pub fn new(inner: Arc<dyn MemoryAccess>) -> Self {
        Self { inner }
    }

    pub fn none() -> Option<Self> {
        None
    }

    pub fn as_dyn(&self) -> &Arc<dyn MemoryAccess> {
        &self.inner
    }
}

impl std::ops::Deref for MemoryHandle {
    type Target = dyn MemoryAccess;
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

/// Default implementation. One `MemoryHandle` per app; the underlying
/// `Store` is shared by `Arc`.
pub struct SqliteMemory {
    store: store::Store,
    privacy: MemoryPrivacy,
}

impl SqliteMemory {
    /// Open or create the on-disk database at `path`. The parent
    /// directory is created if missing.
    pub fn open(path: impl Into<PathBuf>) -> MemoryResult<Self> {
        Self::open_with_privacy(path, MemoryPrivacy::High)
    }

    pub fn open_with_privacy(
        path: impl Into<PathBuf>,
        privacy: MemoryPrivacy,
    ) -> MemoryResult<Self> {
        let metrics = Arc::new(Metrics::new());
        let store = Store::open(path, metrics)?;
        Ok(Self { store, privacy })
    }

    pub fn open_at(state_dir: &Path) -> MemoryResult<Self> {
        let path = state_dir.join("memory").join("v1").join("memory.sqlite");
        Self::open(path)
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn path(&self) -> &Path {
        &self.store.path
    }
}

impl MemoryAccess for SqliteMemory {
    fn recall(&self, query: &MemoryQuery) -> Result<MemoryPack, MemoryError> {
        match retrieval::recall(&self.store, query) {
            Ok(report) => Ok(report.pack),
            Err(e) => {
                self.fail_open(&format!("recall: {e}"));
                if error::is_recoverable(&e) {
                    Ok(MemoryPack::empty())
                } else {
                    Err(e)
                }
            }
        }
    }

    fn observe(&self, event: &SafeMemoryEvent) -> Result<bool, MemoryError> {
        match ingest::observe(&self.store, event, self.privacy) {
            Ok(inserted) => Ok(inserted),
            Err(e) => {
                self.fail_open(&format!("observe: {e}"));
                if error::is_recoverable(&e) {
                    Ok(false)
                } else {
                    Err(e)
                }
            }
        }
    }

    fn mark_changed_paths(
        &self,
        _scope: &MemoryScope,
        paths: &[String],
    ) -> Result<(), MemoryError> {
        let n = match self.store.mark_changed_paths(paths) {
            Ok(n) => n,
            Err(e) => {
                self.fail_open(&format!("mark_changed_paths: {e}"));
                if !error::is_recoverable(&e) {
                    return Err(e);
                }
                0
            }
        };
        if n > 0 {
            self.store
                .metrics
                .marks_stale
                .fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    }

    fn feedback(&self, usage: &MemoryUsage) -> Result<(), MemoryError> {
        match self.store.record_usage(usage) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.fail_open(&format!("feedback: {e}"));
                if error::is_recoverable(&e) {
                    Ok(())
                } else {
                    Err(e)
                }
            }
        }
    }

    fn list(&self, filter: &MemoryFilter) -> Result<Vec<MemoryEntry>, MemoryError> {
        self.store.list(filter).map_err(|e| {
            self.fail_open(&format!("list: {e}"));
            e
        })
    }

    fn inspect(&self, id: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        self.store.get_entry(id).map_err(|e| {
            self.fail_open(&format!("inspect: {e}"));
            e
        })
    }

    fn disable(&self, id: &str) -> Result<(), MemoryError> {
        self.store.disable(id)
    }

    fn tombstone(&self, id: &str, reason: &str, by: &str) -> Result<(), MemoryError> {
        self.store.tombstone(id, reason, by)
    }

    fn is_tombstoned(&self, id: &str) -> Result<bool, MemoryError> {
        self.store.is_tombstoned(id)
    }

    fn metrics(&self) -> MetricsSnapshot {
        self.store.metrics.snapshot()
    }
}

impl SqliteMemory {
    fn fail_open(&self, msg: &str) {
        eprintln!("[kodo-memory] fail-open: {msg}");
        self.store
            .metrics
            .fail_open
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl From<SqliteMemory> for MemoryHandle {
    fn from(m: SqliteMemory) -> Self {
        MemoryHandle::new(Arc::new(m))
    }
}

/// Construct a `MemoryHandle` from a path. Convenience for tests and the
/// CLI; production code uses `SqliteMemory::open` directly.
pub fn open_handle(path: impl Into<PathBuf>) -> MemoryResult<MemoryHandle> {
    Ok(SqliteMemory::open(path)?.into())
}
