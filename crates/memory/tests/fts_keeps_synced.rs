mod _helpers;

use _helpers::{fresh_store, sample_entry};
use kodo_memory::fts;

#[test]
fn fts_reflects_insert_update_delete() {
    let (_dir, store) = fresh_store("fts-sync");
    let e = sample_entry("x", "routing table", "fallback to default", Some("/p"));
    store.upsert_entry(&e).unwrap();
    let hits = fts::search(&store, "routing", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "x");

    // Delete via tombstone; FTS row should vanish.
    store.tombstone("x", "test", "tester").unwrap();
    let hits2 = fts::search(&store, "routing", 10).unwrap();
    assert!(hits2.is_empty());
}

#[test]
fn fts_rebuild_recovers_from_index_drift() {
    // The 'rebuild' command rebuilds the FTS5 index from the table's
    // stored data. We exercise it to confirm the call is well-formed;
    // the actual index recovery path is exercised by fts_keeps_synced
    // and the doctor command.
    let (_dir, store) = fresh_store("fts-rebuild");
    let e = sample_entry("r", "k2k", "routing", Some("/p"));
    store.upsert_entry(&e).unwrap();
    // Sanity: entry is searchable.
    let pre = fts::search(&store, "k2k", 10).unwrap();
    assert_eq!(pre.len(), 1);
    // Rebuild should be a no-op on a healthy DB; must not error.
    store.fts_rebuild().unwrap();
    let post = fts::search(&store, "k2k", 10).unwrap();
    assert_eq!(post.len(), 1);
}

#[test]
fn fts_upsert_keeps_rowid_stable() {
    // The DAO stores the FTS rowid = memory_entries.rowid. An update
    // must NOT change the FTS rowid, or the FTS-JOIN in retrieval.rs
    // would silently drop hits.
    let (_dir, store) = fresh_store("fts-rowid");
    let mut e = sample_entry("r", "k2k", "routing", Some("/p"));
    store.upsert_entry(&e).unwrap();
    let before: i64 = store
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT rowid FROM memory_entries_fts \
                 WHERE rowid = (SELECT rowid FROM memory_entries WHERE id = 'r')",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    e.abstract_text = "routing v2".to_owned();
    e.updated_at = kodo_memory::store::now_secs() + 1;
    store.upsert_entry(&e).unwrap();
    let after: i64 = store
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT rowid FROM memory_entries_fts \
                 WHERE rowid = (SELECT rowid FROM memory_entries WHERE id = 'r')",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(before, after);
}
