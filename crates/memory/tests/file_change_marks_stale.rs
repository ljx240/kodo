// P1: `mark_changed_paths` must (a) flip matching entries to `stale` once,
// (b) keep them stale on a second call (idempotency), and (c) be a no-op
// when the path does not appear in any evidence row.

mod _helpers;

use kodo_memory::store::Store;
use kodo_memory::MemoryStatus;

use _helpers::sample_entry;

fn store_with_entry(id: &str, evidence_path: &str) -> (tempfile::TempDir, Store) {
    let (dir, store) = _helpers::fresh_store("file-change-marks-stale");
    let entry = sample_entry(id, "k2k routing", "fallback to default", Some("/tmp/proj"));
    store.upsert_entry(&entry).unwrap();
    store
        .insert_evidence(id, evidence_path, "digest-initial", None)
        .unwrap();
    (dir, store)
}

#[test]
fn mark_changed_paths_marks_stale() {
    let (_dir, store) = store_with_entry("fc-1", "src/main.rs");
    let n = store
        .mark_changed_paths(&["src/main.rs".to_owned()])
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
    let (_dir, store) = store_with_entry("fc-2", "src/lib.rs");
    store
        .mark_changed_paths(&["src/lib.rs".to_owned()])
        .unwrap();
    assert_eq!(
        store.get_entry("fc-2").unwrap().unwrap().status,
        MemoryStatus::Stale
    );
    store
        .mark_changed_paths(&["src/lib.rs".to_owned()])
        .unwrap();
    let got = store.get_entry("fc-2").unwrap().unwrap();
    assert_eq!(got.status, MemoryStatus::Stale);
}

#[test]
fn mark_changed_paths_with_no_evidence_matches_no_entries() {
    // A path that no evidence row references must be a no-op (count=0),
    // not a "mark everything stale" sweep.
    let (_dir, store) = store_with_entry("fc-3", "src/exists.rs");
    let n = store
        .mark_changed_paths(&["src/never-referenced.rs".to_owned()])
        .unwrap();
    assert_eq!(n, 0);
    let got = store.get_entry("fc-3").unwrap().unwrap();
    assert_eq!(got.status, MemoryStatus::Active);
}
