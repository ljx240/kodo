mod _helpers;

use _helpers::{fresh_store, sample_entry};
use kodo_memory::ingest;
use kodo_memory::model::{EventPhase, MemoryEventKind, MemoryPrivacy, SafeMemoryEvent};
use kodo_memory::outbox;
use kodo_memory::store::now_secs;
use std::collections::BTreeMap;

#[test]
fn tombstone_blocks_replay_revival() {
    let (_dir, store) = fresh_store("tomb-replay");
    // Write a normal memory and confirm it appears.
    let e = sample_entry("alive", "alive", "alive", Some("/proj"));
    store.upsert_entry(&e).unwrap();
    // Tombstone it.
    store.tombstone("alive", "user", "test").unwrap();
    // Now an outbox replay that *would* produce another write does not
    // bring the entry back.
    let evt = SafeMemoryEvent {
        event_id: format!("e-{}", now_secs()),
        session_id: "s".to_owned(),
        turn_seq: 0,
        item_seq: 0,
        phase: EventPhase::Completed,
        kind: MemoryEventKind::FileRead,
        source_digest: "d".to_owned(),
        observed_at: now_secs(),
        project_path: Some("src/lib.rs".to_owned()),
        label: "Read".to_owned(),
        body: None,
        extra: BTreeMap::new(),
    };
    ingest::observe(&store, &evt, MemoryPrivacy::Low).unwrap();
    let _ = outbox::replay(&store, MemoryPrivacy::Low, 100).unwrap();
    assert!(store.get_entry("alive").unwrap().is_none());
    assert!(store.is_tombstoned("alive").unwrap());
}
