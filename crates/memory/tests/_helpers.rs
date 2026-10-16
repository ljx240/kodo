// Shared helpers for integration tests. Kept in a non-`mod` file so each
// `tests/*.rs` includes it as a module.
#![allow(dead_code)] // Each integration test uses a different subset of helpers.

use std::sync::Arc;

use kodo_memory::store::now_secs;
use kodo_memory::{
    Authority, MemoryEntry, MemoryKind, MemoryScope, MemoryStatus, Metrics, SourceTrust,
    SqliteMemory, Store,
};

pub fn fresh_store(tag: &str) -> (tempfile::TempDir, Store) {
    let dir = tempfile::Builder::new()
        .prefix(&format!("kodo-memory-int-{tag}-"))
        .tempdir()
        .unwrap();
    let path = dir.path().join("memory.sqlite");
    let store = Store::open(&path, Arc::new(Metrics::new())).unwrap();
    (dir, store)
}

pub fn fresh_sqlite(tag: &str) -> (tempfile::TempDir, SqliteMemory) {
    let dir = tempfile::Builder::new()
        .prefix(&format!("kodo-memory-int-{tag}-"))
        .tempdir()
        .unwrap();
    let path = dir.path().join("memory.sqlite");
    let mem = SqliteMemory::open(&path).unwrap();
    (dir, mem)
}

pub fn sample_entry(
    id: &str,
    title: &str,
    abstract_text: &str,
    project_id: Option<&str>,
) -> MemoryEntry {
    let now = now_secs();
    MemoryEntry {
        id: id.to_owned(),
        kind: MemoryKind::Episodic,
        status: MemoryStatus::Active,
        authority: Authority::Historical,
        scope: MemoryScope {
            project_id: project_id.map(str::to_owned),
            ..Default::default()
        },
        title: title.to_owned(),
        abstract_text: abstract_text.to_owned(),
        body: serde_json::json!({}),
        content_hash: format!("h-{id}"),
        source_trust: SourceTrust::ToolVerified,
        confidence: 0.6,
        verified: true,
        conditions: vec![],
        source_uris: vec![],
        valid_from: None,
        valid_until: None,
        last_verified_at: Some(now),
        last_used_at: None,
        use_count: 0,
        created_at: now,
        updated_at: now,
        superseded_by: None,
        version: 1,
        candidate_skill: false,
    }
}
