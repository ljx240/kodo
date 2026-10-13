//! Maintenance: integrity check, FTS rebuild, outbox retry, statistics.

use serde::Serialize;
use std::collections::BTreeMap;

use crate::error::MemoryResult;
use crate::model::{MemoryPrivacy, MemoryScope};
use crate::outbox;
use crate::store::Store;

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub integrity: Vec<String>,
    pub row_counts: BTreeMap<String, u64>,
    pub outbox_pending: u64,
    pub outbox_replayed: usize,
    pub tombstones: u64,
    pub notes: Vec<String>,
    pub db_size_bytes: u64,
}

pub fn diagnose(
    store: &Store,
    path: &std::path::Path,
    reindex: bool,
    privacy: MemoryPrivacy,
) -> MemoryResult<DoctorReport> {
    let integrity = store.integrity_check()?;
    let ok = integrity.len() == 1 && integrity[0] == "ok";
    let row_counts = store.row_counts()?;
    let outbox_pending = store.outbox_count_pending()?;

    let mut notes = Vec::new();
    let mut outbox_replayed = 0usize;
    if reindex {
        store.fts_rebuild()?;
        notes.push("fts5 rebuild applied".to_owned());
        outbox_replayed = outbox::replay(store, privacy, 1024)?;
    }

    let tombstones = row_counts.get("memory_tombstones").copied().unwrap_or(0);

    let db_size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    Ok(DoctorReport {
        ok,
        integrity,
        row_counts,
        outbox_pending,
        outbox_replayed,
        tombstones,
        notes,
        db_size_bytes,
    })
}

/// Convenience: a snapshot of the entry list scoped to a project, used
/// by the `kodo memory list` command.
pub fn list_in_scope(
    store: &Store,
    scope: &MemoryScope,
    limit: Option<usize>,
) -> MemoryResult<Vec<crate::model::MemoryEntry>> {
    let filter = crate::model::MemoryFilter {
        scope: Some(scope.clone()),
        limit,
        include_history: true,
        ..Default::default()
    };
    store.list(&filter)
}
