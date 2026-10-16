//! Compile a `SafeMemoryEvent` into a `MemoryEntry`. Pure rules, no LLM.
//!
//! Strategy:
//! - `Reasoning` and `AgentMessage` kinds: skipped (we do not store the
//!   model's stream of thought or the user's message text).
//! - `FileChange`: skipped at compile time; the path is recorded as
//!   evidence only when the user pins it explicitly. (P1 will revisit.)
//! - `Search` / `FileRead` / `CommandExecution`: skipped by default; the
//!   shell's hook can attach a `user_pinned=true` extra flag to promote
//!   to an `episodic`.
//! - `Compaction` / `ContextBudget` / `Permission` / `Failover` /
//!   `PlanStep`: skipped.
//! - Anything with `quarantine_hint=hostile` in extra: written as
//!   `quarantined` (excluded from recall).
//! - Everything else: written as a `working` memory if a `task_id` is
//!   present, else silently dropped.
//!
//! Returns `Ok(true)` when an entry was written, `Ok(false)` when the
//! compiler deliberately skipped it.

use crate::error::MemoryResult;
use crate::model::{
    Authority, MemoryEntry, MemoryEventKind, MemoryKind, MemoryPrivacy, MemoryScope, MemoryStatus,
    SafeMemoryEvent, SourceTrust,
};
use crate::store::Store;
use sha2::{Digest, Sha256};

pub fn compile(
    store: &Store,
    event: &SafeMemoryEvent,
    privacy: MemoryPrivacy,
) -> MemoryResult<bool> {
    if event.extra.get("quarantine_hint").map(String::as_str) == Some("hostile") {
        let entry = make_entry(
            store,
            event,
            MemoryKind::Rule,
            MemoryStatus::Quarantined,
            Authority::Historical,
            SourceTrust::Untrusted,
            0.0,
            "quarantined by hostile-pattern scanner",
            Some("hostile content".to_owned()),
            privacy,
        )?;
        store.upsert_entry(&entry)?;
        return Ok(true);
    }

    // P0: only `user_pinned` events are promoted. The agent loop
    // (P0.4) decides which events to mark. Without an explicit pin, we
    // do not write a memory row.
    if event.extra.get("user_pinned").map(String::as_str) != Some("true") {
        return Ok(false);
    }

    // Require a project_id for pinned entries.
    if !event.extra.contains_key("project_id") {
        return Ok(false);
    }
    let project_id = event.extra.get("project_id").cloned().unwrap_or_default();

    // Require a task_id so we can write a working memory.
    if !event.extra.contains_key("task_id") {
        return Ok(false);
    }

    let kind = match event.kind {
        MemoryEventKind::FileChange => MemoryKind::Episodic,
        MemoryEventKind::FileRead => MemoryKind::Episodic,
        MemoryEventKind::CommandExecution => MemoryKind::Episodic,
        MemoryEventKind::PlanStep => MemoryKind::Procedural,
        MemoryEventKind::Permission => MemoryKind::Rule,
        _ => MemoryKind::Working,
    };
    let status = MemoryStatus::Active;
    let authority = Authority::UserRule;
    let source_trust = SourceTrust::UserConfirmed;

    let mut scope = MemoryScope {
        project_id: Some(project_id),
        session_id: Some(event.session_id.clone()),
        ..Default::default()
    };
    if let Some(t) = event.extra.get("task_id") {
        scope.task_id = Some(t.clone());
    }
    if let Some(p) = event.extra.get("branch") {
        scope.branch = Some(p.clone());
    }
    if let Some(r) = event.extra.get("repository_id") {
        scope.repository_id = Some(r.clone());
    }
    if let Some(w) = event.extra.get("worktree_id") {
        scope.worktree_id = Some(w.clone());
    }

    let title = title_for(event);
    let abstract_text = abstract_for(event);
    let body = serde_json::json!({
        "kind": event.kind.as_str(),
        "phase": event.phase.as_str(),
        "label": event.label,
        "path": event.project_path,
        "body": event.body,
    });
    let now = crate::store::now_secs();
    let content_hash = {
        let mut hasher = Sha256::new();
        hasher.update(title.as_bytes());
        hasher.update(abstract_text.as_bytes());
        hasher.update(event.body.as_deref().unwrap_or("").as_bytes());
        let digest = hasher.finalize();
        hex::encode(digest)
    };
    if let Some(existing) = store.by_content_hash(&content_hash)? {
        if existing.scope.matches(&scope) && existing.status != MemoryStatus::Rejected {
            return Ok(false);
        }
    }
    let entry = MemoryEntry {
        id: format!("mem-{}", &content_hash[..16]),
        kind,
        status,
        authority,
        scope,
        title,
        abstract_text,
        body,
        content_hash,
        source_trust,
        confidence: 0.7,
        verified: false,
        conditions: vec![],
        source_uris: event
            .extra
            .get("source_uris")
            .map(|s| s.split(',').map(|p| p.trim().to_owned()).collect())
            .unwrap_or_default(),
        valid_from: None,
        valid_until: None,
        last_verified_at: None,
        last_used_at: None,
        use_count: 0,
        created_at: now,
        updated_at: now,
        superseded_by: None,
        version: 1,
        candidate_skill: false,
    };
    let _ = now; // silence unused if no other fields need it
    let _ = privacy;
    store.upsert_entry(&entry)?;
    Ok(true)
}

fn make_entry(
    _store: &Store,
    event: &SafeMemoryEvent,
    kind: MemoryKind,
    status: MemoryStatus,
    authority: Authority,
    source_trust: SourceTrust,
    confidence: f32,
    title: &str,
    abstract_text: Option<String>,
    privacy: MemoryPrivacy,
) -> MemoryResult<MemoryEntry> {
    let now = crate::store::now_secs();
    let title = title.to_owned();
    let abstract_text = abstract_text.unwrap_or_default();
    let body = serde_json::json!({
        "kind": event.kind.as_str(),
        "phase": event.phase.as_str(),
        "label": event.label,
        "body": event.body,
    });
    let content_hash = {
        let mut hasher = Sha256::new();
        hasher.update(title.as_bytes());
        hasher.update(abstract_text.as_bytes());
        hasher.update(event.body.as_deref().unwrap_or("").as_bytes());
        let digest = hasher.finalize();
        hex::encode(digest)
    };
    let _ = privacy;
    Ok(MemoryEntry {
        id: format!("qmem-{}", &content_hash[..16]),
        kind,
        status,
        authority,
        scope: MemoryScope {
            project_id: event.extra.get("project_id").cloned(),
            session_id: Some(event.session_id.clone()),
            ..Default::default()
        },
        title,
        abstract_text,
        body,
        content_hash,
        source_trust,
        confidence,
        verified: false,
        conditions: vec![],
        source_uris: vec![],
        valid_from: None,
        valid_until: None,
        last_verified_at: None,
        last_used_at: None,
        use_count: 0,
        created_at: now,
        updated_at: now,
        superseded_by: None,
        version: 1,
        candidate_skill: false,
    })
}

fn title_for(event: &SafeMemoryEvent) -> String {
    if !event.label.is_empty() {
        return event.label.clone();
    }
    format!("{:?} {:?}", event.kind, event.phase)
}

fn abstract_for(event: &SafeMemoryEvent) -> String {
    if let Some(body) = &event.body {
        let trimmed: String = body.chars().take(240).collect();
        return trimmed;
    }
    event.label.clone()
}
