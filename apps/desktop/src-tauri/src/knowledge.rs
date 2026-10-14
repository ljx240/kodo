//! Local knowledge IPC. File reads require a native picker; model requests
//! require a single-use preview bound to source revisions and destination.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kodo_agent::Provider;
use kodo_core::{session, settings, workspace};
use kodo_knowledge::{
    BaseDraft, DocumentDetail, DocumentStatus, ImportDraft, KnowledgeBase, KnowledgeDocument,
    KnowledgeHit, KnowledgeSelection, KnowledgeStore, SearchQuery, SourceKind,
};
use parking_lot::Mutex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use crate::apps::AppService;

const MAX_FILE_BYTES: u64 = kodo_knowledge::MAX_TEXT_BYTES as u64;
const PREVIEW_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Default)]
pub struct KnowledgeService {
    store: Option<Arc<KnowledgeStore>>,
    imports: Arc<Mutex<HashMap<String, ImportGrant>>>,
    turns: Arc<Mutex<HashMap<String, TurnGrant>>>,
}

struct ImportGrant {
    base_id: String,
    project_path: Option<String>,
    path: PathBuf,
    digest: String,
    draft: ImportDraft,
    created: Instant,
}

struct TurnGrant {
    session_id: String,
    project_path: Option<String>,
    question: String,
    selection: KnowledgeSelection,
    hits: Vec<KnowledgeHit>,
    destination: String,
    remote: bool,
    created: Instant,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    token: String,
    title: String,
    body_preview: String,
    character_count: usize,
    chunk_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedTurn {
    token: String,
    hits: Vec<KnowledgeHit>,
    remote: bool,
    provider_label: Option<String>,
    strict: bool,
}

impl KnowledgeService {
    pub fn boot() -> Self {
        let store = settings::state_dir()
            .and_then(|dir| KnowledgeStore::open(&dir).ok())
            .map(Arc::new);
        Self {
            store,
            ..Self::default()
        }
    }

    fn store(&self) -> Result<&Arc<KnowledgeStore>, String> {
        self.store
            .as_ref()
            .ok_or_else(|| "storage_unavailable".into())
    }

    pub fn consume_turn(
        &self,
        token: &str,
        session_id: &str,
        project_path: Option<&str>,
        question: &str,
        created_session: bool,
        provider: Option<&Provider>,
        remote_approved: bool,
    ) -> Result<(KnowledgeSelection, Vec<KnowledgeHit>), String> {
        let grant = self.turns.lock().remove(token).ok_or("preview_expired")?;
        if grant.created.elapsed() > PREVIEW_TTL
            || grant.session_id.is_empty() != created_session
            || (!grant.session_id.is_empty() && grant.session_id != session_id)
            || grant.project_path.as_deref() != project_path
            || grant.question != question
            || grant.destination != destination(provider)
        {
            return Err("preview_expired".into());
        }
        if grant.remote && !remote_approved {
            return Err("knowledge_remote_approval_required".into());
        }
        if !created_session
            && self
                .store()?
                .get_bindings(session_id, project_path)
                .map_err(code)?
                != grant.selection
        {
            return Err("source_changed".into());
        }
        for hit in &grant.hits {
            let detail = self
                .store()?
                .inspect_document(&hit.document_id, project_path)
                .map_err(code)?;
            if detail.document.status != DocumentStatus::Ready
                || detail.document.revision_id != hit.revision_id
                || !detail
                    .chunks
                    .iter()
                    .any(|chunk| chunk.id == hit.chunk_id && chunk.body == hit.quote)
            {
                return Err("source_changed".into());
            }
        }
        // Re-check all selected scopes even when the search produced no hits.
        self.store()?
            .search(&SearchQuery {
                base_ids: grant.selection.base_ids.clone(),
                project_path: project_path.map(str::to_owned),
                text: question.to_owned(),
                limit: 1,
                budget_tokens: 3000,
            })
            .map_err(code)?;
        Ok((grant.selection, grant.hits))
    }

    pub fn save_turn(
        &self,
        id: &str,
        turn_seq: u64,
        scope: Option<&str>,
        selection: &KnowledgeSelection,
        hits: &[KnowledgeHit],
        created_session: bool,
    ) -> Result<(), String> {
        if created_session {
            self.store()?
                .set_bindings(id, scope, selection)
                .map_err(code)?;
        } else if self.store()?.get_bindings(id, scope).map_err(code)? != *selection {
            return Err("source_changed".into());
        }
        self.store()?
            .record_citations(id, turn_seq, scope, hits)
            .map_err(code)?;
        Ok(())
    }

    pub fn safe_history(
        &self,
        dir: &Path,
        found: &session::Session,
    ) -> Result<Vec<(String, String)>, String> {
        // Keep provenance in the session's own append-only log. Deleting or
        // losing the separate index must never make knowledge answers eligible
        // for transmission in a later, ordinary conversation turn.
        let used = session::knowledge_turns(dir, &found.id).map_err(|_| "knowledge_unavailable")?;
        let mut filtered = found.clone();
        let mut safe = Vec::new();
        for (index, turn) in found.turns.iter().enumerate() {
            if !used.contains(&((index + 1) as u64)) {
                safe.push(turn.clone());
            }
        }
        filtered.turns = safe;
        Ok(kodo_shell::turn::conversation_history(&filtered))
    }

    pub fn context(
        &self,
        apps: &AppService,
        id: &str,
        scope: Option<&str>,
        selection: &KnowledgeSelection,
        hits: &[KnowledgeHit],
    ) -> Result<kodo_agent::knowledge::KnowledgeContext, String> {
        let observations = serde_json::to_string(&serde_json::json!({
            "hits": hits.iter().enumerate().map(|(index, hit)| serde_json::json!({
                "citation": format!("K{}", index + 1),
                "title": hit.title,
                "heading": hit.heading,
                "lineStart": hit.line_start,
                "lineEnd": hit.line_end,
                "quote": hit.quote,
            })).collect::<Vec<_>>()
        }))
        .map_err(|_| "invalid_input")?;
        let store = self.store()?.clone();
        let apps = apps.store()?.clone();
        let scope = scope.map(str::to_owned);
        let id = id.to_owned();
        let selected = selection.clone();
        let evidence = hits.to_vec();
        let guard = Arc::new(move || {
            if !apps.list_apps().ok().is_some_and(|rows| {
                rows.into_iter()
                    .any(|app| app.manifest.app_id == "knowledge" && app.preference.enabled)
            }) {
                return false;
            }
            let Ok(current) = store.get_bindings(&id, scope.as_deref()) else {
                return false;
            };
            if current != selected {
                return false;
            }
            evidence.iter().all(|hit| {
                store
                    .inspect_document(&hit.document_id, scope.as_deref())
                    .ok()
                    .is_some_and(|detail| {
                        detail.document.status == DocumentStatus::Ready
                            && detail.document.revision_id == hit.revision_id
                            && detail
                                .chunks
                                .iter()
                                .any(|chunk| chunk.id == hit.chunk_id && chunk.body == hit.quote)
                    })
            })
        });
        Ok(kodo_agent::knowledge::KnowledgeContext {
            observations,
            strict: selection.strict,
            has_evidence: !hits.is_empty(),
            guard: Some(guard),
        })
    }
}

pub fn mark_knowledge_turn(dir: &Path, id: &str, turn_seq: u64) -> Result<(), String> {
    if !valid_session(id) || turn_seq == 0 {
        return Err("invalid_input".into());
    }
    session::record_knowledge_turn(dir, id, turn_seq).map_err(|_| "storage_unavailable".to_owned())
}

fn code(error: kodo_knowledge::KnowledgeError) -> String {
    error.code().to_owned()
}

pub fn ensure_enabled(apps: &AppService) -> Result<(), String> {
    let enabled = apps
        .store()?
        .list_apps()
        .map_err(|e| e.code().to_owned())?
        .into_iter()
        .any(|app| app.manifest.app_id == "knowledge" && app.preference.enabled);
    if enabled {
        Ok(())
    } else {
        Err("app_disabled".into())
    }
}

pub fn canonical_project(path: Option<&str>) -> Result<Option<String>, String> {
    let Some(path) = path.filter(|path| !path.trim().is_empty()) else {
        return Ok(None);
    };
    let canonical = fs::canonicalize(path).map_err(|_| "scope_denied")?;
    if !canonical.is_dir() {
        return Err("scope_denied".into());
    }
    Ok(Some(canonical.to_string_lossy().into_owned()))
}

fn registered_project(path: Option<&str>) -> Result<Option<String>, String> {
    let canonical = canonical_project(path)?;
    if let Some(scope) = canonical.as_deref() {
        let log = workspace::log_path().ok_or("storage_unavailable")?;
        let projects = workspace::load(&log).map_err(|_| "storage_unavailable")?;
        if !projects.iter().any(|project| {
            canonical_project(project.path.to_str())
                .ok()
                .flatten()
                .as_deref()
                == Some(scope)
        }) {
            return Err("scope_denied".into());
        }
    }
    Ok(canonical)
}

fn valid_session(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 160
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

fn session_scope(id: &str, hint: Option<&str>) -> Result<Option<String>, String> {
    if id.is_empty() {
        return registered_project(hint);
    }
    if !valid_session(id) {
        return Err("invalid_input".into());
    }
    let found = session::load(&crate::sessions()?, id).map_err(|_| "not_found")?;
    let actual = canonical_project(found.project.to_str())?;
    let hinted = canonical_project(hint)?;
    if hinted != actual {
        return Err("scope_denied".into());
    }
    Ok(actual)
}

fn destination(provider: Option<&Provider>) -> String {
    provider
        .map(|p| {
            format!(
                "{}\n{}\n{}\n{}",
                p.template,
                p.endpoint,
                p.resolved_model_id(),
                provider_ready(Some(p))
            )
        })
        .unwrap_or_default()
}

fn provider_ready(provider: Option<&Provider>) -> bool {
    provider
        .is_some_and(|p| !p.api_key.trim().is_empty() && !p.resolved_model_id().trim().is_empty())
}

fn provider_label(provider: &Provider) -> String {
    let endpoint = if provider.endpoint.trim().is_empty() {
        match provider.template.as_str() {
            "anthropic" => "https://api.anthropic.com",
            "deepseek" => "https://api.deepseek.com",
            _ => "https://api.openai.com",
        }
    } else {
        provider.endpoint.trim()
    };
    // Never show endpoint credentials, query strings, or key-bearing paths.
    let origin = tauri::Url::parse(endpoint)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|| "未识别的接收地址".to_owned());
    format!(
        "{} · {} · {}",
        provider.template,
        origin,
        provider.resolved_model_id()
    )
}

fn is_remote(provider: Option<&Provider>) -> bool {
    let Some(provider) = provider else {
        return false;
    };
    let endpoint = provider.endpoint.to_ascii_lowercase();
    let Some(rest) = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
    else {
        return true;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return true;
    }
    let local = if authority.starts_with('[') {
        authority == "[::1]"
            || authority
                .strip_prefix("[::1]:")
                .is_some_and(|port| port.parse::<u16>().is_ok())
    } else {
        let mut parts = authority.split(':');
        let host = parts.next().unwrap_or_default();
        let port = parts.next();
        matches!(host, "localhost" | "127.0.0.1")
            && parts.next().is_none()
            && port.is_none_or(|port| port.parse::<u16>().is_ok())
    };
    !local
}

fn digest(body: &str) -> String {
    format!("{:x}", Sha256::digest(body.as_bytes()))
}

fn forbidden_path(path: &Path) -> bool {
    path.components().any(|part| {
        let value = part.as_os_str().to_string_lossy().to_ascii_lowercase();
        value.starts_with(".env")
            || matches!(
                value.as_str(),
                "secrets" | "credentials" | "sessions" | ".kodo" | ".git" | "id_rsa" | "id_ed25519"
            )
            || value.starts_with("production")
            || [".pem", ".key", ".p12", ".jks", ".log"]
                .iter()
                .any(|extension| value.ends_with(extension))
    })
}

fn read_selected_file(
    path: &Path,
    selected_canonical: Option<&Path>,
) -> Result<(PathBuf, String), String> {
    if forbidden_path(path) {
        return Err("sensitive_content".into());
    }
    let original = fs::symlink_metadata(path).map_err(|_| "not_found")?;
    if original.file_type().is_symlink() || !original.is_file() {
        return Err("invalid_input".into());
    }
    let canonical = fs::canonicalize(path).map_err(|_| "not_found")?;
    if selected_canonical.is_some_and(|selected| selected != canonical) {
        return Err("source_changed".into());
    }
    if forbidden_path(&canonical) {
        return Err("sensitive_content".into());
    }
    let extension = canonical
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "md" | "txt") {
        return Err("unsupported_format".into());
    }
    let expected = fs::metadata(&canonical).map_err(|_| "not_found")?;
    if expected.len() > MAX_FILE_BYTES {
        return Err("invalid_input".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(&canonical).map_err(|_| "source_changed")?;
    let opened = file.metadata().map_err(|_| "source_changed")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if expected.ino() != opened.ino() || expected.dev() != opened.dev() {
            return Err("source_changed".into());
        }
    }
    if !opened.is_file() || opened.len() > MAX_FILE_BYTES {
        return Err("invalid_input".into());
    }
    let mut body = String::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|_| "unsupported_format")?;
    if body.len() as u64 > MAX_FILE_BYTES {
        return Err("invalid_input".into());
    }
    Ok((canonical, body))
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| "storage_unavailable".to_owned())?
}

#[tauri::command]
pub async fn knowledge_list_bases(
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    project_path: Option<String>,
) -> Result<Vec<KnowledgeBase>, String> {
    ensure_enabled(&apps)?;
    let service = service.inner().clone();
    blocking(move || {
        let scope = registered_project(project_path.as_deref())?;
        service.store()?.list_bases(scope.as_deref()).map_err(code)
    })
    .await
}

#[tauri::command]
pub async fn knowledge_create_base(
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    mut draft: BaseDraft,
) -> Result<KnowledgeBase, String> {
    ensure_enabled(&apps)?;
    let service = service.inner().clone();
    blocking(move || {
        draft.project_path = registered_project(draft.project_path.as_deref())?;
        service.store()?.create_base(&draft).map_err(code)
    })
    .await
}

#[tauri::command]
pub async fn knowledge_list_documents(
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    base_id: String,
    project_path: Option<String>,
) -> Result<Vec<KnowledgeDocument>, String> {
    ensure_enabled(&apps)?;
    let service = service.inner().clone();
    blocking(move || {
        let scope = registered_project(project_path.as_deref())?;
        service
            .store()?
            .list_documents(&base_id, scope.as_deref())
            .map_err(code)
    })
    .await
}

#[tauri::command]
pub async fn knowledge_inspect_document(
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    id: String,
    project_path: Option<String>,
) -> Result<DocumentDetail, String> {
    ensure_enabled(&apps)?;
    let service = service.inner().clone();
    blocking(move || {
        let scope = registered_project(project_path.as_deref())?;
        service
            .store()?
            .inspect_document(&id, scope.as_deref())
            .map_err(code)
    })
    .await
}

#[tauri::command]
pub async fn knowledge_import_text(
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    base_id: String,
    project_path: Option<String>,
    draft: ImportDraft,
) -> Result<KnowledgeDocument, String> {
    ensure_enabled(&apps)?;
    if draft.source_kind == SourceKind::File {
        return Err("invalid_input".into());
    }
    let service = service.inner().clone();
    blocking(move || {
        let scope = registered_project(project_path.as_deref())?;
        service
            .store()?
            .import_text(&base_id, scope.as_deref(), &draft)
            .map_err(code)
    })
    .await
}

#[tauri::command]
pub async fn knowledge_pick_import(
    app: AppHandle,
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    base_id: String,
    project_path: Option<String>,
) -> Result<Vec<ImportPreview>, String> {
    ensure_enabled(&apps)?;
    let service = service.inner().clone();
    blocking(move || {
        let scope = registered_project(project_path.as_deref())?;
        service
            .store()?
            .list_documents(&base_id, scope.as_deref())
            .map_err(code)?;
        let Some(paths) = app
            .dialog()
            .file()
            .set_title("导入知识资料")
            .add_filter("Markdown / 文本", &["md", "txt"])
            .blocking_pick_files()
        else {
            return Ok(Vec::new());
        };
        if paths.len() > 20 {
            return Err("invalid_input".into());
        }
        let mut grants = Vec::new();
        for selected in paths {
            let path = selected.into_path().map_err(|_| "invalid_input")?;
            let (path, body) = read_selected_file(&path, None)?;
            let title = path
                .file_name()
                .and_then(|part| part.to_str())
                .ok_or("invalid_input")?
                .to_owned();
            let token = Uuid::new_v4().to_string();
            let (character_count, chunk_count) =
                kodo_knowledge::preview_text(&body).map_err(code)?;
            let preview = ImportPreview {
                token: token.clone(),
                title: title.clone(),
                body_preview: body.chars().take(2400).collect(),
                character_count,
                chunk_count,
            };
            let grant = ImportGrant {
                base_id: base_id.clone(),
                project_path: scope.clone(),
                digest: digest(&body),
                draft: ImportDraft {
                    title,
                    body,
                    source_kind: SourceKind::File,
                    source_ref: Some(path.to_string_lossy().into_owned()),
                    verified: false,
                },
                path,
                created: Instant::now(),
            };
            grants.push((token, preview, grant));
        }
        let mut pending = service.imports.lock();
        pending.retain(|_, grant| grant.created.elapsed() < PREVIEW_TTL);
        if pending.len() + grants.len() > 40 {
            return Err("invalid_input".into());
        }
        Ok(grants
            .into_iter()
            .map(|(token, preview, grant)| {
                pending.insert(token, grant);
                preview
            })
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn knowledge_confirm_import(
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    token: String,
) -> Result<KnowledgeDocument, String> {
    ensure_enabled(&apps)?;
    let service = service.inner().clone();
    blocking(move || {
        let grant = service
            .imports
            .lock()
            .remove(&token)
            .ok_or("preview_expired")?;
        if grant.created.elapsed() > PREVIEW_TTL {
            return Err("preview_expired".into());
        }
        let (_, body) = read_selected_file(&grant.path, Some(&grant.path))?;
        if digest(&body) != grant.digest {
            return Err("source_changed".into());
        }
        service
            .store()?
            .import_text(&grant.base_id, grant.project_path.as_deref(), &grant.draft)
            .map_err(code)
    })
    .await
}

#[tauri::command]
pub async fn knowledge_set_enabled(
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    id: String,
    project_path: Option<String>,
    expected_version: u64,
    enabled: bool,
) -> Result<KnowledgeDocument, String> {
    ensure_enabled(&apps)?;
    let service = service.inner().clone();
    blocking(move || {
        let scope = registered_project(project_path.as_deref())?;
        service
            .store()?
            .set_enabled(&id, scope.as_deref(), expected_version, enabled)
            .map_err(code)
    })
    .await
}

#[tauri::command]
pub async fn knowledge_delete_document(
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    id: String,
    project_path: Option<String>,
    expected_version: u64,
) -> Result<(), String> {
    ensure_enabled(&apps)?;
    let service = service.inner().clone();
    blocking(move || {
        let scope = registered_project(project_path.as_deref())?;
        service
            .store()?
            .delete_document(&id, scope.as_deref(), expected_version)
            .map_err(code)
    })
    .await
}

#[tauri::command]
pub async fn knowledge_search(
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    mut query: SearchQuery,
) -> Result<Vec<KnowledgeHit>, String> {
    ensure_enabled(&apps)?;
    let service = service.inner().clone();
    blocking(move || {
        query.project_path = registered_project(query.project_path.as_deref())?;
        service.store()?.search(&query).map_err(code)
    })
    .await
}

#[tauri::command]
pub async fn knowledge_get_bindings(
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    session_id: String,
    project_path: Option<String>,
) -> Result<KnowledgeSelection, String> {
    ensure_enabled(&apps)?;
    let service = service.inner().clone();
    blocking(move || {
        let scope = session_scope(&session_id, project_path.as_deref())?;
        if session_id.is_empty() {
            return Ok(KnowledgeSelection::default());
        }
        service
            .store()?
            .get_bindings(&session_id, scope.as_deref())
            .map_err(code)
    })
    .await
}

#[tauri::command]
pub async fn knowledge_set_bindings(
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    session_id: String,
    project_path: Option<String>,
    selection: KnowledgeSelection,
) -> Result<KnowledgeSelection, String> {
    ensure_enabled(&apps)?;
    let service = service.inner().clone();
    blocking(move || {
        let scope = session_scope(&session_id, project_path.as_deref())?;
        service
            .store()?
            .set_bindings(&session_id, scope.as_deref(), &selection)
            .map_err(code)?;
        Ok(selection)
    })
    .await
}

#[tauri::command]
pub async fn knowledge_prepare_turn(
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    session_id: String,
    project_path: Option<String>,
    selection: KnowledgeSelection,
    text: String,
) -> Result<PreparedTurn, String> {
    ensure_enabled(&apps)?;
    let service = service.inner().clone();
    blocking(move || {
        let scope = session_scope(&session_id, project_path.as_deref())?;
        if !session_id.is_empty()
            && service
                .store()?
                .get_bindings(&session_id, scope.as_deref())
                .map_err(code)?
                != selection
        {
            return Err("source_changed".into());
        }
        let hits = service
            .store()?
            .search(&SearchQuery {
                base_ids: selection.base_ids.clone(),
                project_path: scope.clone(),
                text: text.clone(),
                limit: 6,
                budget_tokens: 3000,
            })
            .map_err(code)?;
        let provider = kodo_shell::ShellConfig::load(kodo_shell::Overrides::default()).provider;
        let remote =
            !hits.is_empty() && provider_ready(provider.as_ref()) && is_remote(provider.as_ref());
        let provider_label = provider.as_ref().map(provider_label);
        let token = Uuid::new_v4().to_string();
        let mut pending = service.turns.lock();
        pending.retain(|_, grant| grant.created.elapsed() < PREVIEW_TTL);
        if pending.len() >= 40 {
            return Err("invalid_input".into());
        }
        pending.insert(
            token.clone(),
            TurnGrant {
                session_id,
                project_path: scope,
                question: text,
                selection: selection.clone(),
                hits: hits.clone(),
                destination: destination(provider.as_ref()),
                remote,
                created: Instant::now(),
            },
        );
        Ok(PreparedTurn {
            token,
            hits,
            remote,
            provider_label,
            strict: selection.strict,
        })
    })
    .await
}

#[tauri::command]
pub async fn knowledge_turn_citations(
    service: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    session_id: String,
    turn_seq: u64,
) -> Result<Vec<KnowledgeHit>, String> {
    ensure_enabled(&apps)?;
    let service = service.inner().clone();
    blocking(move || {
        if !valid_session(&session_id) {
            return Err("invalid_input".into());
        }
        let found = session::load(&crate::sessions()?, &session_id).map_err(|_| "not_found")?;
        let scope = canonical_project(found.project.to_str())?;
        Ok(service
            .store()?
            .read_citations(&session_id, turn_seq, scope.as_deref())
            .map_err(code)?
            .hits)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("kodo-knowledge-test-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(root: &Path) -> (KnowledgeService, KnowledgeSelection, Vec<KnowledgeHit>) {
        let store = Arc::new(KnowledgeStore::open(root).unwrap());
        let base = store
            .create_base(&BaseDraft {
                name: "测试资料".into(),
                description: "".into(),
                project_path: None,
            })
            .unwrap();
        store
            .import_text(
                &base.id,
                None,
                &ImportDraft {
                    title: "发布流程".into(),
                    body: "# 发布\n发布前需进行回归验证。".into(),
                    source_kind: SourceKind::Note,
                    source_ref: None,
                    verified: true,
                },
            )
            .unwrap();
        let selection = KnowledgeSelection {
            base_ids: vec![base.id],
            strict: true,
        };
        store
            .set_bindings("session-test", None, &selection)
            .unwrap();
        let hits = store
            .search(&SearchQuery {
                base_ids: selection.base_ids.clone(),
                project_path: None,
                text: "发布".into(),
                limit: 6,
                budget_tokens: 3000,
            })
            .unwrap();
        assert!(!hits.is_empty());
        (
            KnowledgeService {
                store: Some(store),
                ..KnowledgeService::default()
            },
            selection,
            hits,
        )
    }

    fn grant(
        service: &KnowledgeService,
        selection: &KnowledgeSelection,
        hits: &[KnowledgeHit],
        provider: &Provider,
    ) -> String {
        let token = Uuid::new_v4().to_string();
        service.turns.lock().insert(
            token.clone(),
            TurnGrant {
                session_id: "session-test".into(),
                project_path: None,
                question: "发布".into(),
                selection: selection.clone(),
                hits: hits.to_vec(),
                destination: destination(Some(provider)),
                remote: true,
                created: Instant::now(),
            },
        );
        token
    }

    #[test]
    fn preview_is_single_use_and_bound_to_approval_question_and_destination() {
        let root = Sandbox::new();
        let (service, selection, hits) = fixture(&root.0);
        let provider = Provider::new(
            "openai",
            "test-only",
            "https://model.example.test/v1",
            "test",
        );
        let token = grant(&service, &selection, &hits, &provider);
        assert_eq!(
            service
                .consume_turn(
                    &token,
                    "session-test",
                    None,
                    "发布",
                    false,
                    Some(&provider),
                    false
                )
                .unwrap_err(),
            "knowledge_remote_approval_required"
        );
        assert_eq!(
            service
                .consume_turn(
                    &token,
                    "session-test",
                    None,
                    "发布",
                    false,
                    Some(&provider),
                    true
                )
                .unwrap_err(),
            "preview_expired"
        );
        let token = grant(&service, &selection, &hits, &provider);
        assert_eq!(
            service
                .consume_turn(
                    &token,
                    "session-test",
                    None,
                    "changed question",
                    false,
                    Some(&provider),
                    true
                )
                .unwrap_err(),
            "preview_expired"
        );
        let token = grant(&service, &selection, &hits, &provider);
        let different = Provider::new(
            "openai",
            "test-only",
            "https://different.example.test/v1",
            "test",
        );
        assert_eq!(
            service
                .consume_turn(
                    &token,
                    "session-test",
                    None,
                    "发布",
                    false,
                    Some(&different),
                    true
                )
                .unwrap_err(),
            "preview_expired"
        );
        let token = grant(&service, &selection, &hits, &provider);
        assert!(service
            .consume_turn(
                &token,
                "session-test",
                None,
                "发布",
                false,
                Some(&provider),
                true
            )
            .is_ok());
    }

    #[test]
    fn making_a_provider_ready_requires_a_new_preview_and_approval() {
        let root = Sandbox::new();
        let (service, selection, hits) = fixture(&root.0);
        let offline = Provider::new("openai", "", "https://model.example.test/v1", "test");
        let token = grant(&service, &selection, &hits, &offline);
        service.turns.lock().get_mut(&token).unwrap().remote = false;
        let ready = Provider::new(
            "openai",
            "test-only",
            "https://model.example.test/v1",
            "test",
        );
        assert_eq!(
            service
                .consume_turn(
                    &token,
                    "session-test",
                    None,
                    "发布",
                    false,
                    Some(&ready),
                    false
                )
                .unwrap_err(),
            "preview_expired"
        );
    }

    #[test]
    fn unbinding_and_disabling_invalidate_pending_approval() {
        let root = Sandbox::new();
        let (service, selection, hits) = fixture(&root.0);
        let provider = Provider::new(
            "openai",
            "test-only",
            "https://model.example.test/v1",
            "test",
        );
        let token = grant(&service, &selection, &hits, &provider);
        service
            .store()
            .unwrap()
            .set_bindings("session-test", None, &KnowledgeSelection::default())
            .unwrap();
        assert_eq!(
            service
                .consume_turn(
                    &token,
                    "session-test",
                    None,
                    "发布",
                    false,
                    Some(&provider),
                    true
                )
                .unwrap_err(),
            "source_changed"
        );
        assert!(service
            .store()
            .unwrap()
            .get_bindings("session-test", None)
            .unwrap()
            .base_ids
            .is_empty());
        service
            .store()
            .unwrap()
            .set_bindings("session-test", None, &selection)
            .unwrap();
        let token = grant(&service, &selection, &hits, &provider);
        service
            .store()
            .unwrap()
            .set_enabled(&hits[0].document_id, None, 1, false)
            .unwrap();
        assert_eq!(
            service
                .consume_turn(
                    &token,
                    "session-test",
                    None,
                    "发布",
                    false,
                    Some(&provider),
                    true
                )
                .unwrap_err(),
            "source_changed"
        );
    }

    #[test]
    fn dispatch_guard_rechecks_app_binding_and_source() {
        let root = Sandbox::new();
        let (service, selection, hits) = fixture(&root.0);
        let apps = AppService::for_test(kodo_shell::AppStore::open(&root.0).unwrap());
        let context = service
            .context(&apps, "session-test", None, &selection, &hits)
            .unwrap();
        assert!(context.is_current());
        apps.store()
            .unwrap()
            .set_app_enabled("knowledge", false)
            .unwrap();
        assert!(!context.is_current());
        apps.store()
            .unwrap()
            .set_app_enabled("knowledge", true)
            .unwrap();
        assert!(context.is_current());
        service
            .store()
            .unwrap()
            .set_bindings("session-test", None, &KnowledgeSelection::default())
            .unwrap();
        assert!(!context.is_current());
        service
            .store()
            .unwrap()
            .set_bindings("session-test", None, &selection)
            .unwrap();
        service
            .store()
            .unwrap()
            .set_enabled(&hits[0].document_id, None, 1, false)
            .unwrap();
        assert!(!context.is_current());
    }

    #[test]
    fn history_stays_filtered_even_when_the_catalog_is_unavailable() {
        let root = Sandbox::new();
        let dir = root.0.join("sessions");
        let id = session::open(&dir, Path::new(""), "test", 1, session::Mode::Work).unwrap();
        session::record_ask(&dir, &id, 2, "ordinary question").unwrap();
        session::record_turn_complete(&dir, &id, 3).unwrap();
        session::record_ask(&dir, &id, 4, "knowledge question").unwrap();
        mark_knowledge_turn(&dir, &id, 2).unwrap();
        let answer = session::Item {
            id: 1,
            at: 5,
            status: session::Status::Done,
            duration_ms: None,
            kind: session::ItemKind::agent_message("knowledge-derived answer", Vec::new()),
        };
        session::record_item(&dir, &id, &answer, session::Phase::Completed).unwrap();
        session::record_turn_complete(&dir, &id, 6).unwrap();
        let found = session::load(&dir, &id).unwrap();
        let history = KnowledgeService::default()
            .safe_history(&dir, &found)
            .unwrap();
        assert_eq!(history, vec![("user".into(), "ordinary question".into())]);
    }

    #[test]
    fn destination_label_shows_origin_without_credentials_or_query() {
        let provider = Provider::new(
            "openai",
            "test-only",
            "https://user:secret@model.example.test:9443/private-key?key=hidden",
            "test",
        );
        let label = provider_label(&provider);
        assert!(label.contains("https://model.example.test:9443"));
        for hidden in ["user", "secret", "private-key", "hidden"] {
            assert!(!label.contains(hidden));
        }
    }

    #[test]
    fn only_exact_loopback_destinations_are_local() {
        for endpoint in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:8080",
            "http://[::1]:3000/v1",
        ] {
            let provider = Provider::new("openai", "", endpoint, "local");
            assert!(!is_remote(Some(&provider)));
        }
        for endpoint in [
            "https://api.example.test/v1",
            "http://localhost.example.test",
            "http://localhost@remote.test",
            "http://127.0.0.1.evil",
            "http://[::1].evil",
        ] {
            let provider = Provider::new("openai", "", endpoint, "local");
            assert!(is_remote(Some(&provider)));
        }
    }

    #[test]
    fn selected_files_enforce_utf8_size_and_canonical_snapshot() {
        let root = Sandbox::new();
        let path = root.0.join("guide.md");
        fs::write(&path, "# 原文\n本地验证内容").unwrap();
        let (selected, body) = read_selected_file(&path, None).unwrap();
        assert!(body.contains("本地验证内容"));
        assert!(read_selected_file(&path, Some(&selected)).is_ok());
        fs::write(&path, vec![b'a'; kodo_knowledge::MAX_TEXT_BYTES + 1]).unwrap();
        assert_eq!(
            read_selected_file(&path, Some(&selected)).unwrap_err(),
            "invalid_input"
        );
        fs::write(&path, [0xff, 0xfe]).unwrap();
        assert_eq!(
            read_selected_file(&path, Some(&selected)).unwrap_err(),
            "unsupported_format"
        );
    }

    #[cfg(unix)]
    #[test]
    fn replacing_a_selected_parent_with_a_symlink_invalidates_confirmation() {
        let root = Sandbox::new();
        let original = root.0.join("selected");
        let other = root.0.join("unselected");
        fs::create_dir_all(&original).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::write(original.join("guide.md"), "approved content").unwrap();
        fs::write(other.join("guide.md"), "unselected content").unwrap();
        let (selected, _) = read_selected_file(&original.join("guide.md"), None).unwrap();
        fs::rename(&original, root.0.join("original-copy")).unwrap();
        std::os::unix::fs::symlink(&other, &original).unwrap();
        assert_eq!(
            read_selected_file(&selected, Some(&selected)).unwrap_err(),
            "source_changed"
        );
    }

    #[test]
    fn sensitive_and_non_text_sources_are_blocked_before_content_read() {
        for path in [
            "/tmp/.env.txt",
            "/tmp/secrets/info.md",
            "/tmp/.kodo/notes.md",
            "/tmp/production/settings.txt",
            "/tmp/id_rsa",
            "/tmp/session.log",
        ] {
            assert!(forbidden_path(Path::new(path)));
        }
        assert!(!forbidden_path(Path::new("/tmp/knowledge/guide.md")));
        assert!(!valid_session("../credentials"));
        assert!(valid_session("session-123"));
    }
}
