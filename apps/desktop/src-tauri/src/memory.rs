//! Tauri IPC for the Memory Engine.
//!
//! Six thin commands: status / list / search / inspect / disable / doctor.
//! All errors propagate as `String` so the React side can render them
//! verbatim (the existing convention in this crate).

use std::path::PathBuf;

use kodo_core::settings;
use kodo_memory::{doctor, model as m, retrieval, scope, SqliteMemory};

use crate::view::MemoryEntryView;

/// Open the on-disk store at `<state_dir>/memory/v1/memory.sqlite`.
/// Creates the parent directory if missing.
fn open_sqlite() -> Result<SqliteMemory, String> {
    let state_dir = settings::state_dir()
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep memory".to_owned())?;
    let path = state_dir.join("memory").join("v1").join("memory.sqlite");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    SqliteMemory::open(&path).map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStatusDto {
    pub path: String,
    pub row_counts: std::collections::BTreeMap<String, u64>,
    pub outbox_pending: u64,
    pub tombstones: u64,
    pub db_size_bytes: u64,
    pub mode: String,
}

#[tauri::command]
pub fn memory_status() -> Result<MemoryStatusDto, String> {
    let sqlite = open_sqlite()?;
    let store = sqlite.store();
    let path = sqlite.path().to_string_lossy().into_owned();
    let row_counts = store.row_counts().map_err(|e| e.to_string())?;
    let outbox_pending = store.outbox_count_pending().map_err(|e| e.to_string())?;
    let tombstones = row_counts.get("memory_tombstones").copied().unwrap_or(0);
    let db_size_bytes = std::fs::metadata(sqlite.path())
        .map(|m| m.len())
        .unwrap_or(0);
    Ok(MemoryStatusDto {
        path,
        row_counts,
        outbox_pending,
        tombstones,
        db_size_bytes,
        mode: "off".to_owned(),
    })
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryListArgs {
    pub project: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub limit: Option<usize>,
    pub include_history: Option<bool>,
}

#[tauri::command]
pub fn memory_list(args: MemoryListArgs) -> Result<Vec<MemoryEntryView>, String> {
    let sqlite = open_sqlite()?;
    let project = args
        .project
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let project_scope = scope::scope_for(&project, None, None);
    let mut filter = m::MemoryFilter {
        scope: Some(project_scope),
        limit: args.limit,
        include_history: args.include_history.unwrap_or(true),
        ..Default::default()
    };
    if let Some(k) = args.kind {
        filter.kind = m::MemoryKind::parse(&k);
    }
    if let Some(s) = args.status {
        filter.status = m::MemoryStatus::parse(&s);
    }
    let entries = sqlite.store().list(&filter).map_err(|e| e.to_string())?;
    Ok(entries.into_iter().map(MemoryEntryView::from).collect())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchArgs {
    pub project: Option<String>,
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnippetDto {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub abstract_text: String,
    pub source_uris: Vec<String>,
    pub validity: String,
    pub authority: String,
    pub scope_label: String,
}

#[tauri::command]
pub fn memory_search(args: MemorySearchArgs) -> Result<Vec<MemorySnippetDto>, String> {
    let sqlite = open_sqlite()?;
    let project = args
        .project
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let project_scope = scope::scope_for(&project, None, None);
    let query = m::MemoryQuery {
        scope: project_scope,
        text: args.query,
        related_paths: Vec::new(),
        related_symbols: Vec::new(),
        budget_tokens: 2000,
        require_current_evidence: false,
    };
    let report = retrieval::recall(sqlite.store(), &query).map_err(|e| e.to_string())?;
    let limit = args.limit.unwrap_or(report.pack.snippets.len());
    Ok(report
        .pack
        .snippets
        .into_iter()
        .take(limit)
        .map(|s| MemorySnippetDto {
            id: s.id,
            kind: s.kind.as_str().to_owned(),
            title: s.title,
            abstract_text: s.abstract_text,
            source_uris: s.source_uris,
            validity: s.validity.as_str().to_owned(),
            authority: s.authority.as_str().to_owned(),
            scope_label: s.scope_label,
        })
        .collect())
}

#[tauri::command]
pub fn memory_inspect(id: String) -> Result<Option<MemoryEntryView>, String> {
    let sqlite = open_sqlite()?;
    let entry = sqlite.store().get_entry(&id).map_err(|e| e.to_string())?;
    Ok(entry.map(MemoryEntryView::from))
}

#[tauri::command]
pub fn memory_disable(id: String) -> Result<(), String> {
    let sqlite = open_sqlite()?;
    sqlite.store().disable(&id).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDoctorDto {
    pub ok: bool,
    pub integrity: Vec<String>,
    pub row_counts: std::collections::BTreeMap<String, u64>,
    pub outbox_pending: u64,
    pub outbox_replayed: usize,
    pub tombstones: u64,
    pub notes: Vec<String>,
    pub db_size_bytes: u64,
}

#[tauri::command]
pub fn memory_doctor(apply: Option<bool>) -> Result<MemoryDoctorDto, String> {
    let sqlite = open_sqlite()?;
    let report = doctor::diagnose(
        sqlite.store(),
        sqlite.path(),
        apply.unwrap_or(false),
        m::MemoryPrivacy::High,
    )
    .map_err(|e| e.to_string())?;
    Ok(MemoryDoctorDto {
        ok: report.ok,
        integrity: report.integrity,
        row_counts: report.row_counts,
        outbox_pending: report.outbox_pending,
        outbox_replayed: report.outbox_replayed,
        tombstones: report.tombstones,
        notes: report.notes,
        db_size_bytes: report.db_size_bytes,
    })
}
