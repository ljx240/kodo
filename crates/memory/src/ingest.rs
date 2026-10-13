//! Ingest path. `SafeMemoryEvent` comes in; we hand it to the store's
//! outbox. The hot path is the `observe` trait method on `MemoryHandle`,
//! which delegates here.

use crate::error::MemoryResult;
use crate::model::SafeMemoryEvent;
use crate::sanitizer;
use crate::store::Store;

pub fn observe(
    store: &Store,
    event: &SafeMemoryEvent,
    privacy: crate::model::MemoryPrivacy,
) -> MemoryResult<bool> {
    // Sanitize the body, if any, before persistence.
    let mut sanitized = event.clone();
    if let Some(body) = &event.body {
        sanitized.body = Some(sanitizer::scrub(body, privacy)?);
    }
    // Refuse prompt-injection-shaped events outright: route to the
    // outbox as usual but mark it for quarantine at compile time.
    if let Some(body) = &sanitized.body {
        if sanitizer::looks_hostile(body) {
            sanitized
                .extra
                .insert("quarantine_hint".to_owned(), "hostile".to_owned());
        }
    }
    let inserted = store.outbox_enqueue(&sanitized)?;
    if inserted {
        store
            .metrics
            .outbox_enqueued
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    } else {
        store
            .metrics
            .outbox_deduped
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(inserted)
}
