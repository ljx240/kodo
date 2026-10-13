mod _helpers;

use _helpers::fresh_store;
use kodo_memory::ingest;
use kodo_memory::model::{EventPhase, MemoryEventKind, MemoryPrivacy, SafeMemoryEvent};
use kodo_memory::store::now_secs;
use std::collections::BTreeMap;

fn event_with_body(body: &str) -> SafeMemoryEvent {
    SafeMemoryEvent {
        event_id: format!("e-{}", kodo_memory::store::now_secs()),
        session_id: "s1".to_owned(),
        turn_seq: 0,
        item_seq: 0,
        phase: EventPhase::Completed,
        kind: MemoryEventKind::FileRead,
        source_digest: "d".to_owned(),
        observed_at: now_secs(),
        project_path: None,
        label: "x".to_owned(),
        body: Some(body.to_owned()),
        extra: BTreeMap::new(),
    }
}

#[test]
fn secret_body_is_redacted_before_enqueue() {
    let (_dir, store) = fresh_store("redact");
    let evt = event_with_body("token=sk-1234567890ABCDEFGH");
    ingest::observe(&store, &evt, MemoryPrivacy::Low).unwrap();
    let pending = store.outbox_pending(10).unwrap();
    assert_eq!(pending.len(), 1);
    let stored = pending[0].body.clone().unwrap();
    assert!(!stored.contains("sk-1234567890ABCDEFGH"), "got: {stored}");
}

#[test]
fn high_privacy_refuses_secret_body() {
    let (_dir, store) = fresh_store("redact-high");
    let evt = event_with_body("AKIA1234567890ABCDEFGH");
    let r = ingest::observe(&store, &evt, MemoryPrivacy::High);
    assert!(r.is_err());
}
