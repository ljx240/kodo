mod _helpers;

use _helpers::{fresh_store, sample_entry};
use kodo_memory::MemoryStatus;

#[test]
fn insert_get_list_by_scope() {
    let (_dir, store) = fresh_store("crud");
    let a = sample_entry("a", "alpha", "abstract alpha", Some("/proj/a"));
    let b = sample_entry("b", "beta", "abstract beta", Some("/proj/b"));
    store.upsert_entry(&a).unwrap();
    store.upsert_entry(&b).unwrap();

    let all = store.list(&kodo_memory::MemoryFilter::default()).unwrap();
    assert_eq!(all.len(), 2);

    let only_a = store
        .list(&kodo_memory::MemoryFilter {
            project_id: Some("/proj/a".to_owned()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(only_a.len(), 1);
    assert_eq!(only_a[0].id, "a");
}

#[test]
fn update_increments_version() {
    let (_dir, store) = fresh_store("version");
    let mut e = sample_entry("u", "title", "abstract", None);
    store.upsert_entry(&e).unwrap();
    let first_version = store.get_entry("u").unwrap().unwrap().version;
    e.abstract_text = "changed".to_owned();
    e.updated_at = kodo_memory::store::now_secs() + 1;
    store.upsert_entry(&e).unwrap();
    let after = store.get_entry("u").unwrap().unwrap();
    assert_eq!(after.version, first_version + 1);
    assert_eq!(after.abstract_text, "changed");
}

#[test]
fn tombstone_suppresses_replay() {
    let (_dir, store) = fresh_store("tomb");
    let e = sample_entry("die", "rip", "rest in peace", Some("/proj"));
    store.upsert_entry(&e).unwrap();
    store.tombstone("die", "user", "tester").unwrap();
    assert!(store.get_entry("die").unwrap().is_none());
    assert!(store.is_tombstoned("die").unwrap());
    let entries = store.list(&kodo_memory::MemoryFilter::default()).unwrap();
    assert!(!entries.iter().any(|x| x.id == "die"));
}

#[test]
fn disable_marks_rejected() {
    let (_dir, store) = fresh_store("disable");
    let e = sample_entry("d", "soft die", "soft", None);
    store.upsert_entry(&e).unwrap();
    store.disable("d").unwrap();
    let got = store.get_entry("d").unwrap().unwrap();
    assert_eq!(got.status, MemoryStatus::Rejected);
}
