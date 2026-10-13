//! Outbox replay. Invoked by `kodo memory doctor` and the Tauri
//! `memory_reindex` command. Not in the agent hot path.

use crate::compiler;
use crate::error::{MemoryError, MemoryResult};
use crate::model::{MemoryPrivacy, SafeMemoryEvent};
use crate::store::Store;

pub fn replay(store: &Store, privacy: MemoryPrivacy, limit: usize) -> MemoryResult<usize> {
    let mut compiled = 0usize;
    let pending = store.outbox_pending(limit)?;
    for event in pending {
        match compile_one(store, &event, privacy) {
            Ok(true) => {
                store.outbox_mark_compiled(&event.event_id)?;
                store
                    .metrics
                    .outbox_compiled
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                compiled += 1;
            }
            Ok(false) => {
                // The compiler intentionally decided not to write an
                // entry (e.g. a duplicate). Mark compiled to drop from
                // the queue.
                store.outbox_mark_compiled(&event.event_id)?;
            }
            Err(e) => {
                let msg = format!("{e}");
                store.outbox_mark_failed(&event.event_id, &msg)?;
                store
                    .metrics
                    .outbox_failed
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if !is_recoverable(&e) {
                    return Err(e);
                }
            }
        }
    }
    Ok(compiled)
}

fn is_recoverable(err: &MemoryError) -> bool {
    crate::error::is_recoverable(err)
}

fn compile_one(
    store: &Store,
    event: &SafeMemoryEvent,
    privacy: MemoryPrivacy,
) -> MemoryResult<bool> {
    compiler::compile(store, event, privacy)
}
