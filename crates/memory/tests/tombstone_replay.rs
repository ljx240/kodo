mod _helpers;

use _helpers::fresh_store;
use kodo_memory::ingest;
use kodo_memory::model::{EventPhase, MemoryEventKind, MemoryPrivacy, SafeMemoryEvent};
use kodo_memory::outbox;
use kodo_memory::store::now_secs;
use std::collections::BTreeMap;

#[test]
fn tombstone_blocks_replay_revival() {
    let (_dir, store) = fresh_store("tomb-replay");
    let mut evt = SafeMemoryEvent {
        event_id: "first".into(),
        session_id: "s".to_owned(),
        turn_seq: 0,
        item_seq: 0,
        phase: EventPhase::Completed,
        kind: MemoryEventKind::FileRead,
        source_digest: "d".to_owned(),
        observed_at: now_secs(),
        project_path: Some("src/lib.rs".to_owned()),
        label: "Read".to_owned(),
        body: Some("routing fixture".into()),
        extra: BTreeMap::from([
            ("user_pinned".into(), "true".into()),
            ("project_id".into(), "/proj".into()),
            ("task_id".into(), "task".into()),
        ]),
    };
    ingest::observe(&store, &evt, MemoryPrivacy::Low).unwrap();
    assert_eq!(outbox::replay(&store, MemoryPrivacy::Low, 100).unwrap(), 1);
    let id = store.list(&Default::default()).unwrap()[0].id.clone();

    store.disable(&id).unwrap();
    evt.event_id = "duplicate after disable".into();
    ingest::observe(&store, &evt, MemoryPrivacy::Low).unwrap();
    assert_eq!(outbox::replay(&store, MemoryPrivacy::Low, 100).unwrap(), 0);
    assert_eq!(
        store.get_entry(&id).unwrap().unwrap().status,
        kodo_memory::MemoryStatus::Rejected
    );

    store.tombstone(&id, "user", "test").unwrap();
    evt.event_id = "duplicate after delete".into();
    ingest::observe(&store, &evt, MemoryPrivacy::Low).unwrap();
    assert_eq!(outbox::replay(&store, MemoryPrivacy::Low, 100).unwrap(), 0);
    assert!(store.get_entry(&id).unwrap().is_none());
    assert!(store.is_tombstoned(&id).unwrap());

    evt.event_id = "same content in another project".into();
    evt.extra.insert("project_id".into(), "/other".into());
    ingest::observe(&store, &evt, MemoryPrivacy::Low).unwrap();
    assert_eq!(outbox::replay(&store, MemoryPrivacy::Low, 100).unwrap(), 1);
    let entries = store.list(&Default::default()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].scope.project_id.as_deref(), Some("/other"));
}
