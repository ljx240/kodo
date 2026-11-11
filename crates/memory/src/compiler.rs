//! Convert sanitized outbox events into scoped memory entries.

use crate::model::{
    Authority, EventPhase, MemoryEntry, MemoryEventKind, MemoryKind, MemoryScope, MemoryStatus,
    SafeMemoryEvent, SourceTrust,
};
use crate::{MemoryError, MemoryResult, Store};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

pub(crate) fn compile(store: &Store, event: &SafeMemoryEvent) -> MemoryResult<bool> {
    let flag =
        |key: &str, expected: &str| event.extra.get(key).is_some_and(|value| value == expected);
    let quarantined = flag("quarantine_hint", "hostile");
    let verified = event.kind == MemoryEventKind::AgentMessage
        && event.phase == EventPhase::Completed
        && flag("verified_turn", "true");
    let pinned = flag("user_pinned", "true") && event.extra.contains_key("task_id");
    let scope = MemoryScope {
        project_id: event.extra.get("project_id").cloned(),
        repository_id: event.extra.get("repository_id").cloned(),
        branch: event.extra.get("branch").cloned(),
        worktree_id: event.extra.get("worktree_id").cloned(),
        session_id: (!verified).then(|| event.session_id.clone()),
        task_id: event.extra.get("task_id").cloned(),
        ..Default::default()
    };
    if !quarantined && (scope.project_id.is_none() || !(verified || pinned)) {
        return Ok(false);
    }

    let (kind, status, authority, trust, confidence, title, abstract_text) = if quarantined {
        (
            MemoryKind::Rule,
            MemoryStatus::Quarantined,
            Authority::Historical,
            SourceTrust::Untrusted,
            0.0,
            "quarantined by hostile-pattern scanner".to_owned(),
            "hostile content".to_owned(),
        )
    } else if verified {
        let summary = event
            .body
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| MemoryError::InvalidEvent("verified memory needs a summary".into()))?;
        let title = summary
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or(summary);
        (
            MemoryKind::Episodic,
            MemoryStatus::Active,
            Authority::Historical,
            SourceTrust::ToolVerified,
            0.7,
            title.trim().chars().take(80).collect(),
            summary.to_owned(),
        )
    } else {
        let kind = match event.kind {
            MemoryEventKind::FileChange
            | MemoryEventKind::FileRead
            | MemoryEventKind::CommandExecution => MemoryKind::Episodic,
            MemoryEventKind::PlanStep => MemoryKind::Procedural,
            MemoryEventKind::Permission => MemoryKind::Rule,
            _ => MemoryKind::Working,
        };
        let title = if event.label.is_empty() {
            format!("{:?} {:?}", event.kind, event.phase)
        } else {
            event.label.clone()
        };
        let abstract_text = event
            .body
            .as_deref()
            .map(|body| body.chars().take(240).collect())
            .unwrap_or_else(|| event.label.clone());
        (
            kind,
            MemoryStatus::Active,
            Authority::UserRule,
            SourceTrust::UserConfirmed,
            0.7,
            title,
            abstract_text,
        )
    };

    // Keep the former content-only ID available for existing tombstones.
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    hasher.update(abstract_text.as_bytes());
    hasher.update(event.body.as_deref().unwrap_or("").as_bytes());
    let content_hash = hex::encode(hasher.finalize());
    let prefix = if quarantined { "qmem" } else { "mem" };
    let legacy_id = format!("{prefix}-{}", &content_hash[..16]);
    let paths: Vec<String> = if quarantined {
        Vec::new()
    } else if let Some(encoded) = event.extra.get("evidence_paths") {
        serde_json::from_str(encoded)?
    } else {
        event.project_path.iter().cloned().collect()
    };
    for path in &paths {
        if path.is_empty()
            || Path::new(path).components().any(|part| {
                matches!(
                    part,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(MemoryError::InvalidEvent(
                "memory evidence path must be project-relative".into(),
            ));
        }
    }

    let identity = serde_json::to_vec(&(
        &scope,
        authority.as_str(),
        &title,
        &abstract_text,
        &event.body,
        &paths,
    ))?;
    let scoped_hash = hex::encode(Sha256::digest(identity));
    let now = crate::store::now_secs();
    let entry = MemoryEntry {
        id: format!("{prefix}-{}", &scoped_hash[..32]),
        kind,
        status,
        authority,
        scope,
        title,
        abstract_text,
        body: serde_json::json!({
            "kind": event.kind.as_str(),
            "phase": event.phase.as_str(),
            "label": event.label,
            "path": event.project_path,
            "paths": paths,
            "body": event.body,
        }),
        content_hash,
        source_trust: trust,
        confidence,
        verified: verified && !quarantined,
        conditions: Vec::new(),
        source_uris: if quarantined {
            Vec::new()
        } else {
            event
                .extra
                .get("source_uris")
                .map(|uris| uris.split(',').map(|uri| uri.trim().to_owned()).collect())
                .unwrap_or_default()
        },
        valid_from: None,
        valid_until: None,
        last_verified_at: (verified && !quarantined).then_some(event.observed_at),
        last_used_at: None,
        use_count: 0,
        created_at: now,
        updated_at: now,
        superseded_by: None,
        version: 1,
        candidate_skill: false,
    };
    store.insert_compiled_entry(&entry, &legacy_id, &paths)
}
