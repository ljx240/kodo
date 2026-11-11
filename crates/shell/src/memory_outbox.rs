//! Save a bounded, verified result after the session's completion is durable.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kodo_core::{
    session::{Item, ItemKind},
    settings,
};
use kodo_memory::model::{EventPhase, MemoryEventKind, SafeMemoryEvent};
use kodo_memory::{MemoryHandle, MemoryScope, SqliteMemory};

pub const MEMORY_ENABLED_SETTING: &str = "memory-enabled";

pub fn memory_enabled() -> bool {
    settings::settings_path()
        .and_then(|path| settings::read(&path, MEMORY_ENABLED_SETTING))
        .as_deref()
        == Some("true")
}

pub(crate) struct TurnMemory {
    store: Arc<SqliteMemory>,
    project: PathBuf,
    turn_seq: u32,
    answer: Option<VerifiedAnswer>,
    paths: BTreeSet<String>,
}

struct VerifiedAnswer {
    item_id: u32,
    at: u64,
    text: String,
}

impl TurnMemory {
    pub(crate) fn open(project: &Path, turn_seq: u32) -> Option<Self> {
        if !memory_enabled() {
            return None;
        }
        let state = settings::state_dir()?;
        match SqliteMemory::open_at(&state) {
            Ok(store) => Some(Self::new(store, project, turn_seq)),
            Err(error) => {
                eprintln!("[kodo-memory] fail-open: open store: {error}");
                None
            }
        }
    }

    pub(crate) fn new(store: SqliteMemory, project: &Path, turn_seq: u32) -> Self {
        Self {
            store: Arc::new(store),
            project: project.to_owned(),
            turn_seq,
            answer: None,
            paths: BTreeSet::new(),
        }
    }

    pub(crate) fn handle(&self) -> MemoryHandle {
        MemoryHandle::new(self.store.clone())
    }

    pub(crate) fn record(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::FileChange { changes, .. } => {
                self.paths
                    .extend(changes.iter().map(|change| change.path.clone()));
            }
            ItemKind::AgentMessage { .. } => {
                self.answer = verified_text(item).map(|text| VerifiedAnswer {
                    item_id: item.id,
                    at: item.at,
                    text: text.trim().to_owned(),
                });
            }
            _ => {}
        }
    }

    pub(crate) fn finish(&self, session_id: &str) {
        if let Some(answer) = &self.answer {
            let scope = kodo_memory::scope::scope_for(&self.project, None, None);
            if let Some(event) =
                event_for_answer(answer, &scope, session_id, self.turn_seq, &self.paths)
            {
                self.store.remember(&event);
            }
        }
    }
}

pub(crate) fn verified_text(item: &Item) -> Option<&str> {
    match &item.kind {
        ItemKind::AgentMessage {
            text,
            delivery,
            verification,
            plain: false,
            ..
        } if delivery == "ready" && verification == "passed" && !text.trim().is_empty() => {
            Some(text)
        }
        _ => None,
    }
}

fn event_for_answer(
    answer: &VerifiedAnswer,
    scope: &MemoryScope,
    session_id: &str,
    turn_seq: u32,
    paths: &BTreeSet<String>,
) -> Option<SafeMemoryEvent> {
    let project_id = scope.project_id.as_ref()?;
    let mut extra = BTreeMap::from([
        ("verified_turn".to_owned(), "true".to_owned()),
        ("project_id".to_owned(), project_id.clone()),
        (
            "source_uris".to_owned(),
            format!(
                "kodo://sessions/{session_id}/turns/{turn_seq}/items/{}",
                answer.item_id
            ),
        ),
        (
            "evidence_paths".to_owned(),
            serde_json::to_string(paths).expect("file paths serialize"),
        ),
    ]);
    for (key, value) in [
        ("repository_id", &scope.repository_id),
        ("branch", &scope.branch),
        ("worktree_id", &scope.worktree_id),
    ] {
        if let Some(value) = value {
            extra.insert(key.to_owned(), value.clone());
        }
    }
    Some(SafeMemoryEvent {
        event_id: format!(
            "turn:{session_id}:{turn_seq}:{}:completed:agent_message",
            answer.item_id
        ),
        session_id: session_id.to_owned(),
        turn_seq,
        item_seq: answer.item_id,
        phase: EventPhase::Completed,
        kind: MemoryEventKind::AgentMessage,
        source_digest: String::new(),
        observed_at: answer.at,
        project_path: None,
        label: String::new(),
        body: Some(answer.text.clone()),
        extra,
    })
}
