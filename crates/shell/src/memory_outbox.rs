//! Bridge from the session log into the memory outbox.
//!
//! The session log is the source of truth. After the log has been
//! written, the turn sink calls [`write_event`] for the kinds of items
//! the memory engine is allowed to read: file reads, file changes,
//! public agent messages, and the high-signal search / command
//! executions. Private reasoning and model-call traces are deliberately
//! not forwarded — they live in the trace file, not the memory DB.
//!
//! All failures here are non-fatal: the memory engine is a derived
//! index, and a dropped event just means the engine has one less data
//! point. The function returns `false` on any error so the caller can
//! log a metric counter without breaking the run.

use std::collections::BTreeMap;

use kodo_core::session::{Item, ItemKind, Phase};
use kodo_memory::model::{EventPhase, MemoryEventKind, SafeMemoryEvent};
use kodo_memory::MemoryHandle;

/// The maximum body size we will forward to the outbox in one go. Any
/// payload over the cap is dropped — the memory engine prefers
/// short, scoped observations over a per-turn transcript.
const MAX_BODY_BYTES: usize = 32 * 1024;

/// Open the on-disk store at `<state_dir>/memory/v1/memory.sqlite` and
/// wrap it in a `MemoryHandle`. Returns `Ok(None)` when `state_dir()`
/// is unavailable (HOME unset, sandbox restriction) — fail-open.
pub fn open_default_handle() -> kodo_memory::error::MemoryResult<Option<MemoryHandle>> {
    let Some(state_dir) = kodo_core::settings::state_dir() else {
        return Ok(None);
    };
    let path = state_dir.join("memory").join("v1").join("memory.sqlite");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    Ok(Some(MemoryHandle::from(kodo_memory::SqliteMemory::open(
        path,
    )?)))
}

/// Build a `SafeMemoryEvent` from a freshly-logged `Item`. Returns
/// `None` when the item is a private kind the memory engine must not
/// see (Reasoning, Thinking, ModelCall, PlanStep, Permission,
/// ContextBudget, Failover, Compaction). Returning `None` is the
/// intended path for the majority of items.
pub fn event_for_item(item: &Item, phase: Phase) -> Option<SafeMemoryEvent> {
    let (kind, project_path, label, body) = match &item.kind {
        ItemKind::Search { query, detail } => (
            MemoryEventKind::Search,
            None,
            truncate_label("search", query),
            cap_body(detail),
        ),
        ItemKind::FileRead { path, detail } => (
            MemoryEventKind::FileRead,
            Some(path.clone()),
            truncate_label("read", path),
            cap_body(detail),
        ),
        ItemKind::FileChange { changes, .. } => {
            let first_path = changes.first().map(|c| c.path.clone());
            let label = first_path
                .clone()
                .unwrap_or_else(|| "file_change".to_owned());
            (
                MemoryEventKind::FileChange,
                first_path,
                truncate_label("edit", &label),
                None,
            )
        }
        ItemKind::CommandExecution { command, cwd, .. } => (
            MemoryEventKind::CommandExecution,
            None,
            truncate_label("command", command),
            cap_body(cwd),
        ),
        ItemKind::AgentMessage { text, .. } => (
            MemoryEventKind::AgentMessage,
            None,
            truncate_label("message", text),
            cap_body(text),
        ),
        // Private or transport-level kinds. Not forwarded.
        ItemKind::Reasoning { .. }
        | ItemKind::Thinking { .. }
        | ItemKind::ModelCall { .. }
        | ItemKind::Compaction { .. }
        | ItemKind::Failover { .. }
        | ItemKind::PlanStep { .. }
        | ItemKind::Permission { .. }
        | ItemKind::ContextBudget { .. } => return None,
    };

    let event_phase = match phase {
        Phase::Started => EventPhase::Started,
        Phase::Completed => EventPhase::Completed,
        Phase::Failed => EventPhase::Failed,
    };

    Some(SafeMemoryEvent {
        // A stable id: session/turn/item/phase/kind keeps the
        // outbox's idempotency check valid under retries.
        event_id: format!(
            "item-{}-{}-{}",
            item.id,
            event_phase.as_str(),
            kind.as_str()
        ),
        session_id: String::new(), // filled in by the caller if known
        turn_seq: 0,
        item_seq: item.id,
        phase: event_phase,
        kind,
        source_digest: String::new(),
        observed_at: item.at,
        project_path,
        label,
        body,
        extra: BTreeMap::new(),
    })
}

/// Forward `event` to the outbox. Returns `true` on `Ok(true)`, `false`
/// on any error or `Ok(false)` (the outbox may reject the event for
/// policy reasons — secret-shaped body, etc.). The caller treats both
/// `false` outcomes identically: bump a metric counter and move on.
pub fn write_event(handle: &MemoryHandle, event: &SafeMemoryEvent) -> bool {
    handle.observe(event).unwrap_or(false)
}

fn truncate_label(prefix: &str, body: &str) -> String {
    const MAX: usize = 80;
    if body.len() <= MAX {
        format!("{prefix}:{body}")
    } else {
        let end = body.char_indices().nth(MAX).map(|(i, _)| i).unwrap_or(MAX);
        format!("{}:{}…", prefix, &body[..end])
    }
}

fn cap_body(body: &str) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    if body.len() <= MAX_BODY_BYTES {
        return Some(body.to_owned());
    }
    let end = body
        .char_indices()
        .nth(MAX_BODY_BYTES)
        .map(|(i, _)| i)
        .unwrap_or(MAX_BODY_BYTES);
    Some(format!("{}…[truncated]", &body[..end]))
}
