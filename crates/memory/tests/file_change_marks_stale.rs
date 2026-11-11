// P1: `mark_changed_paths` must (a) flip matching entries to `stale` once,
// (b) keep them stale on a second call (idempotency), and (c) be a no-op
// when the path does not appear in any evidence row.

mod _helpers;

use kodo_memory::store::Store;
use kodo_memory::{MemoryScope, MemoryStatus};

use _helpers::sample_entry;

fn store_with_entry(id: &str, evidence_path: &str) -> (tempfile::TempDir, Store, MemoryScope) {
    let (dir, store) = _helpers::fresh_store("file-change-marks-stale");
    let entry = sample_entry(id, "k2k routing", "fallback to default", Some("/tmp/proj"));
    store.upsert_entry(&entry).unwrap();
    store
        .insert_evidence(id, evidence_path, "digest-initial", None)
        .unwrap();
    (dir, store, entry.scope)
}

#[test]
fn mark_changed_paths_marks_stale() {
    let (_dir, store, scope) = store_with_entry("fc-1", "src/main.rs");
    let n = store
        .mark_changed_paths(&scope, &["src/main.rs".to_owned()])
        .unwrap();
    assert_eq!(n, 1);
    let got = store.get_entry("fc-1").unwrap().unwrap();
    assert_eq!(got.status, MemoryStatus::Stale);
}

#[test]
fn mark_changed_paths_twice_keeps_status_stale() {
    // Idempotency: a second call must not flip the entry to a different
    // state, must not corrupt the row, and must not move it back to
    // `active`. The exact count of "rows touched" is a SQL UPDATE
    // matching every reference — what matters here is the
    // post-condition: the entry stays `stale` across repeated calls.
    let (_dir, store, scope) = store_with_entry("fc-2", "src/lib.rs");
    store
        .mark_changed_paths(&scope, &["src/lib.rs".to_owned()])
        .unwrap();
    assert_eq!(
        store.get_entry("fc-2").unwrap().unwrap().status,
        MemoryStatus::Stale
    );
    store
        .mark_changed_paths(&scope, &["src/lib.rs".to_owned()])
        .unwrap();
    let got = store.get_entry("fc-2").unwrap().unwrap();
    assert_eq!(got.status, MemoryStatus::Stale);
}

#[test]
fn mark_changed_paths_with_no_evidence_matches_no_entries() {
    // A path that no evidence row references must be a no-op (count=0),
    // not a "mark everything stale" sweep.
    let (_dir, store, scope) = store_with_entry("fc-3", "src/exists.rs");
    let n = store
        .mark_changed_paths(&scope, &["src/never-referenced.rs".to_owned()])
        .unwrap();
    assert_eq!(n, 0);
    let got = store.get_entry("fc-3").unwrap().unwrap();
    assert_eq!(got.status, MemoryStatus::Active);
}

#[test]
fn invalidation_respects_project_and_worktree_and_keeps_rejected_entries() {
    let (_dir, store, scope) = store_with_entry("affected", "src/lib.rs");
    let mut foreign = sample_entry("foreign", "same path", "fixture", Some("/tmp/other"));
    store.upsert_entry(&foreign).unwrap();
    foreign.id = "worktree".into();
    foreign.scope = scope.clone();
    foreign.scope.worktree_id = Some("other worktree".into());
    store.upsert_entry(&foreign).unwrap();
    foreign.id = "rejected".into();
    foreign.scope = scope.clone();
    foreign.status = MemoryStatus::Rejected;
    store.upsert_entry(&foreign).unwrap();
    for id in ["foreign", "worktree", "rejected"] {
        store
            .insert_evidence(id, "src/lib.rs", "digest", None)
            .unwrap();
    }
    assert_eq!(
        store
            .mark_changed_paths(&scope, &["src/lib.rs".into()])
            .unwrap(),
        1
    );
    for (id, status) in [
        ("affected", MemoryStatus::Stale),
        ("foreign", MemoryStatus::Active),
        ("worktree", MemoryStatus::Active),
        ("rejected", MemoryStatus::Rejected),
    ] {
        assert_eq!(store.get_entry(id).unwrap().unwrap().status, status);
    }
}
