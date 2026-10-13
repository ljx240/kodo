mod _helpers;

use _helpers::fresh_store;
use kodo_memory::model::{EventPhase, MemoryEventKind, SafeMemoryEvent};
use kodo_memory::store::now_secs;
use std::collections::BTreeMap;

fn event(event_id: &str) -> SafeMemoryEvent {
    SafeMemoryEvent {
        event_id: event_id.to_owned(),
        session_id: "s1".to_owned(),
        turn_seq: 0,
        item_seq: 0,
        phase: EventPhase::Completed,
        kind: MemoryEventKind::FileRead,
        source_digest: "d".to_owned(),
        observed_at: now_secs(),
        project_path: Some("src/lib.rs".to_owned()),
        label: "Read src/lib.rs".to_owned(),
        body: None,
        extra: BTreeMap::new(),
    }
}

#[test]
fn enqueue_twice_yields_one_row() {
    let (_dir, store) = fresh_store("dedupe");
    let first = store.outbox_enqueue(&event("e1")).unwrap();
    let second = store.outbox_enqueue(&event("e1")).unwrap();
    assert!(first);
    assert!(!second);
    assert_eq!(store.outbox_count_pending().unwrap(), 1);
}

#[test]
fn mark_compiled_drops_from_pending() {
    let (_dir, store) = fresh_store("mark");
    let e = event("e2");
    store.outbox_enqueue(&e).unwrap();
    store.outbox_mark_compiled("e2").unwrap();
    assert_eq!(store.outbox_count_pending().unwrap(), 0);
}
