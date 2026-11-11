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
    enqueue(store, &prepare(event, privacy)?)
}

pub(crate) fn prepare(
    event: &SafeMemoryEvent,
    privacy: crate::model::MemoryPrivacy,
) -> MemoryResult<SafeMemoryEvent> {
    let mut sanitized = event.clone();
    sanitized.label = sanitizer::scrub(&event.label, privacy)?;
    if let Some(body) = &event.body {
        sanitized.body = Some(sanitizer::scrub(body, privacy)?);
    }
    if let Some(path) = &event.project_path {
        sanitized.project_path = Some(sanitizer::scrub(path, privacy)?);
    }
    for value in sanitized.extra.values_mut() {
        *value = sanitizer::scrub(value, privacy)?;
    }
    if let Some(uris) = sanitized.extra.get("source_uris") {
        for uri in uris.split(',').map(str::trim) {
            crate::evidence::validate_source_uri(uri)?;
        }
    }
    if sanitizer::looks_hostile(&sanitized.label)
        || sanitized
            .body
            .as_deref()
            .is_some_and(sanitizer::looks_hostile)
    {
        sanitized
            .extra
            .insert("quarantine_hint".into(), "hostile".into());
    }
    if sanitized
        .extra
        .get("verified_turn")
        .is_some_and(|value| value == "true")
    {
        if let Some(body) = &mut sanitized.body {
            *body = body.chars().take(240).collect();
        }
    }
    Ok(sanitized)
}

pub(crate) fn enqueue(store: &Store, event: &SafeMemoryEvent) -> MemoryResult<bool> {
    let inserted = store.outbox_enqueue(event)?;
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
