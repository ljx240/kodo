//! Immediate compilation and replay of the durable memory outbox.

use crate::model::{MemoryPrivacy, SafeMemoryEvent};
use crate::{compiler, ingest, MemoryResult, Store};
use std::sync::atomic::Ordering;

pub(crate) fn observe_and_compile(
    store: &Store,
    event: &SafeMemoryEvent,
    privacy: MemoryPrivacy,
) -> MemoryResult<()> {
    let prepared = ingest::prepare(event, privacy)?;
    if ingest::enqueue(store, &prepared)? {
        let result = compiler::compile(store, &prepared);
        record_compile_result(store, &prepared.event_id, result)?;
    }
    Ok(())
}

pub fn replay(store: &Store, privacy: MemoryPrivacy, limit: usize) -> MemoryResult<usize> {
    let mut compiled = 0;
    for event in store.outbox_pending(limit)? {
        // Existing outbox rows predate label and path sanitization.
        let result = ingest::prepare(&event, privacy)
            .and_then(|prepared| compiler::compile(store, &prepared));
        compiled += usize::from(record_compile_result(store, &event.event_id, result)?);
    }
    Ok(compiled)
}

fn record_compile_result(
    store: &Store,
    event_id: &str,
    result: MemoryResult<bool>,
) -> MemoryResult<bool> {
    match result {
        Ok(written) => {
            store.outbox_mark_compiled(event_id)?;
            if written {
                store
                    .metrics
                    .outbox_compiled
                    .fetch_add(1, Ordering::Relaxed);
            }
            Ok(written)
        }
        Err(error) => {
            store.outbox_mark_failed(event_id, &error.to_string())?;
            store.metrics.outbox_failed.fetch_add(1, Ordering::Relaxed);
            if crate::error::is_recoverable(&error) {
                Ok(false)
            } else {
                Err(error)
            }
        }
    }
}
