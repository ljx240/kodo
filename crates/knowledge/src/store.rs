use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row, Transaction};

use crate::error::{KnowledgeError, KnowledgeResult};
use crate::model::{
    BaseDraft, DocumentDetail, DocumentStatus, ImportDraft, KnowledgeBase, KnowledgeChunk,
    KnowledgeDocument, KnowledgeHit, KnowledgeSelection, SearchQuery, SourceKind, TurnKnowledge,
};
use crate::text::{
    content_hash, looks_hostile, make_chunks, query_terms, validate_text_body, TextChunk,
    MAX_CHUNKS_PER_DOCUMENT, MAX_SOURCE_CHARS,
};

const MAX_BASES: usize = 256;
const MAX_DOCUMENTS_PER_BASE: usize = 1_000;
const MAX_SELECTED_BASES: usize = 64;
const MAX_SEARCH_LIMIT: usize = 50;
const MAX_SEARCH_BUDGET: usize = 8_192;
const MAX_CITATIONS_PER_TURN: usize = 64;
const MAX_CANDIDATES: usize = 256;
const MAX_CHUNKS_PER_BASE: usize = 100_000;
const MAX_SEARCHABLE_CHUNKS: usize = 100_000;
const RRF_K: f64 = 60.0;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS knowledge_bases (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    project_path TEXT,
    created_at INTEGER NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0)
);
CREATE INDEX IF NOT EXISTS knowledge_bases_project_path
    ON knowledge_bases(project_path);

CREATE TABLE IF NOT EXISTS knowledge_documents (
    id TEXT PRIMARY KEY,
    base_id TEXT NOT NULL REFERENCES knowledge_bases(id),
    title TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('note', 'file', 'result', 'answer')),
    source_ref TEXT,
    status TEXT NOT NULL CHECK (status IN ('ready', 'disabled', 'quarantined')),
    verified INTEGER NOT NULL CHECK (verified IN (0, 1)),
    version INTEGER NOT NULL CHECK (version > 0),
    current_revision_id TEXT,
    content_hash TEXT NOT NULL,
    chunk_count INTEGER NOT NULL CHECK (chunk_count >= 0),
    import_key TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS knowledge_documents_import_key
    ON knowledge_documents(base_id, import_key);
CREATE UNIQUE INDEX IF NOT EXISTS knowledge_documents_source_ref
    ON knowledge_documents(base_id, source_kind, source_ref)
    WHERE source_ref IS NOT NULL;
CREATE INDEX IF NOT EXISTS knowledge_documents_current
    ON knowledge_documents(base_id, status, deleted_at, current_revision_id);

CREATE TABLE IF NOT EXISTS knowledge_revisions (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES knowledge_documents(id),
    version INTEGER NOT NULL CHECK (version > 0),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(document_id, version)
);
CREATE INDEX IF NOT EXISTS knowledge_revisions_document
    ON knowledge_revisions(document_id, version);

CREATE TABLE IF NOT EXISTS knowledge_chunks (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES knowledge_documents(id),
    base_id TEXT NOT NULL REFERENCES knowledge_bases(id),
    revision_id TEXT NOT NULL REFERENCES knowledge_revisions(id),
    heading TEXT NOT NULL,
    line_start INTEGER NOT NULL CHECK (line_start > 0),
    line_end INTEGER NOT NULL CHECK (line_end >= line_start),
    body TEXT NOT NULL,
    search_terms TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS knowledge_chunks_revision
    ON knowledge_chunks(revision_id, line_start);
CREATE INDEX IF NOT EXISTS knowledge_chunks_base
    ON knowledge_chunks(base_id, document_id, revision_id);

CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_chunks_fts USING fts5(
    title, heading, body, search_terms,
    tokenize = 'unicode61'
);

CREATE TABLE IF NOT EXISTS knowledge_bindings (
    session_id TEXT NOT NULL,
    project_path_key TEXT NOT NULL,
    selection_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(session_id, project_path_key)
);

CREATE TABLE IF NOT EXISTS knowledge_turns (
    session_id TEXT NOT NULL,
    project_path_key TEXT NOT NULL,
    turn_seq INTEGER NOT NULL CHECK (turn_seq >= 0),
    created_at INTEGER NOT NULL,
    PRIMARY KEY(session_id, project_path_key, turn_seq)
);
CREATE TABLE IF NOT EXISTS knowledge_turn_citations (
    session_id TEXT NOT NULL,
    project_path_key TEXT NOT NULL,
    turn_seq INTEGER NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    base_id TEXT NOT NULL,
    document_id TEXT NOT NULL,
    revision_id TEXT NOT NULL,
    chunk_id TEXT NOT NULL,
    channels_json TEXT NOT NULL,
    PRIMARY KEY(session_id, project_path_key, turn_seq, position),
    UNIQUE(session_id, project_path_key, turn_seq, chunk_id),
    FOREIGN KEY(session_id, project_path_key, turn_seq)
        REFERENCES knowledge_turns(session_id, project_path_key, turn_seq)
);
PRAGMA user_version = 1;
"#;

/// SQLite-backed catalog. The connection mutex serializes a store instance;
/// SQLite's transaction and busy timeout handle multiple instances safely.
#[derive(Debug)]
pub struct KnowledgeStore {
    connection: Mutex<Connection>,
}

#[derive(Debug)]
struct ExistingImport {
    document: KnowledgeDocument,
    deleted: bool,
    revision_title: String,
    revision_body: String,
}

#[derive(Debug)]
struct RankedHit {
    hit: KnowledgeHit,
    score: f64,
}

impl KnowledgeStore {
    /// Opens `state_dir/knowledge/v1/catalog.sqlite` and initializes P0 schema.
    pub fn open(state_dir: impl AsRef<Path>) -> KnowledgeResult<Self> {
        let knowledge_directory = state_dir.as_ref().join("knowledge");
        std::fs::create_dir_all(&knowledge_directory)?;
        ensure_real_directory(&knowledge_directory)?;

        let directory = knowledge_directory.join("v1");
        #[cfg(unix)]
        ensure_private_directory(&directory)?;
        #[cfg(not(unix))]
        {
            std::fs::create_dir_all(&directory)?;
            ensure_real_directory(&directory)?;
        }

        let database = directory.join("catalog.sqlite");
        #[cfg(unix)]
        prepare_private_database(&database)?;
        let connection = Connection::open(&database)?;
        #[cfg(unix)]
        set_private_file_permissions(&database)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA secure_delete = ON;",
        )?;
        connection.execute_batch(SCHEMA)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn list_bases(&self, project_path: Option<&str>) -> KnowledgeResult<Vec<KnowledgeBase>> {
        validate_scope(project_path)?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, name, description, project_path, created_at, version
             FROM knowledge_bases
             WHERE project_path IS NULL OR (?1 IS NOT NULL AND project_path = ?1)
             ORDER BY project_path IS NOT NULL, name COLLATE NOCASE, id
             LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![project_path, MAX_BASES as i64], base_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(KnowledgeError::from)?;
        Ok(rows)
    }

    pub fn create_base(&self, draft: &BaseDraft) -> KnowledgeResult<KnowledgeBase> {
        validate_name(&draft.name, 128)?;
        validate_short_text(&draft.description, MAX_SOURCE_CHARS)?;
        validate_scope(draft.project_path.as_deref())?;

        let now = now_secs();
        let base = KnowledgeBase {
            id: uuid::Uuid::new_v4().to_string(),
            name: draft.name.trim().to_owned(),
            description: draft.description.clone(),
            project_path: draft.project_path.clone(),
            created_at: now,
            version: 1,
        };
        let mut connection = self.connection()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let count: i64 =
            transaction.query_row("SELECT COUNT(*) FROM knowledge_bases", [], |row| row.get(0))?;
        if count >= MAX_BASES as i64 {
            return Err(KnowledgeError::InvalidInput);
        }
        transaction.execute(
            "INSERT INTO knowledge_bases
                (id, name, description, project_path, created_at, version)
             VALUES (?1, ?2, ?3, ?4, ?5, 1)",
            params![
                base.id,
                base.name,
                base.description,
                base.project_path,
                now as i64
            ],
        )?;
        transaction.commit()?;
        Ok(base)
    }

    pub fn list_documents(
        &self,
        base_id: &str,
        project_path: Option<&str>,
    ) -> KnowledgeResult<Vec<KnowledgeDocument>> {
        validate_id(base_id)?;
        validate_scope(project_path)?;
        let connection = self.connection()?;
        ensure_base_access(&connection, base_id, project_path)?;
        let mut statement = connection.prepare(
            "SELECT d.id, d.base_id, d.title, d.source_kind, d.source_ref, d.status,
                    d.verified, d.version, d.current_revision_id, d.content_hash,
                    d.chunk_count, d.created_at, d.updated_at
             FROM knowledge_documents d
             WHERE d.base_id = ?1 AND d.deleted_at IS NULL
             ORDER BY d.updated_at DESC, d.id
             LIMIT ?2",
        )?;
        let rows = statement
            .query_map(
                params![base_id, MAX_DOCUMENTS_PER_BASE as i64],
                document_from_row,
            )?
            .collect::<Result<Vec<_>, _>>()
            .map_err(KnowledgeError::from)?;
        Ok(rows)
    }

    pub fn import_text(
        &self,
        base_id: &str,
        project_path: Option<&str>,
        draft: &ImportDraft,
    ) -> KnowledgeResult<KnowledgeDocument> {
        validate_id(base_id)?;
        validate_scope(project_path)?;
        validate_name(&draft.title, 512)?;
        validate_text_body(&draft.body)?;
        if crate::text::contains_secret_material(&draft.title) {
            return Err(KnowledgeError::SensitiveContent);
        }
        if let Some(source_ref) = draft.source_ref.as_deref() {
            if source_ref.chars().count() > MAX_SOURCE_CHARS || source_ref.contains('\0') {
                return Err(KnowledgeError::InvalidInput);
            }
            if crate::text::contains_secret_material(source_ref) {
                return Err(KnowledgeError::SensitiveContent);
            }
        }

        let title = draft.title.trim();
        let source_ref = draft
            .source_ref
            .as_deref()
            .filter(|source_ref| !source_ref.is_empty());
        let body_hash = content_hash(&draft.body);
        let hostile = looks_hostile(&draft.body) || looks_hostile(title);
        let new_chunks = make_chunks(title, &draft.body)?;
        let import_key = import_key(draft, source_ref, title, &body_hash);
        let now = now_secs();

        let mut connection = self.connection()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure_base_access(&transaction, base_id, project_path)?;

        let existing = find_existing_import(&transaction, base_id, &import_key)?;
        let document = if let Some(existing) = existing {
            if existing.deleted {
                return Err(KnowledgeError::NotFound);
            }
            let mut document = existing.document;
            let content_changed =
                existing.revision_title != title || existing.revision_body != draft.body;
            let verified_changed = document.verified != draft.verified;
            if !content_changed && !verified_changed {
                transaction.commit()?;
                return Ok(document);
            }

            let version = document
                .version
                .checked_add(1)
                .filter(|version| *version <= i64::MAX as u64)
                .ok_or(KnowledgeError::InvalidInput)?;
            let status = match document.status {
                DocumentStatus::Disabled => DocumentStatus::Disabled,
                DocumentStatus::Quarantined => DocumentStatus::Quarantined,
                DocumentStatus::Ready if hostile => DocumentStatus::Quarantined,
                DocumentStatus::Ready => DocumentStatus::Ready,
            };
            let (revision_id, chunk_count) = if content_changed {
                let revision_id = insert_revision(
                    &transaction,
                    &document.id,
                    base_id,
                    version,
                    title,
                    &draft.body,
                    &body_hash,
                    &new_chunks,
                    now,
                )?;
                (revision_id, new_chunks.len())
            } else {
                (document.revision_id.clone(), document.chunk_count)
            };
            transaction.execute(
                "UPDATE knowledge_documents
                 SET title = ?1, status = ?2, verified = ?3, version = ?4,
                     current_revision_id = ?5, content_hash = ?6, chunk_count = ?7,
                     updated_at = ?8
                 WHERE id = ?9 AND deleted_at IS NULL",
                params![
                    title,
                    status.as_str(),
                    i64::from(draft.verified),
                    version as i64,
                    revision_id,
                    body_hash,
                    chunk_count as i64,
                    now as i64,
                    document.id
                ],
            )?;
            document.title = title.to_owned();
            document.status = status;
            document.verified = draft.verified;
            document.version = version;
            document.revision_id = revision_id;
            document.content_hash = body_hash;
            document.chunk_count = chunk_count;
            document.updated_at = now;
            document
        } else {
            let count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM knowledge_documents WHERE base_id = ?1",
                params![base_id],
                |row| row.get(0),
            )?;
            if count >= MAX_DOCUMENTS_PER_BASE as i64 {
                return Err(KnowledgeError::InvalidInput);
            }

            let document_id = uuid::Uuid::new_v4().to_string();
            let status = if hostile {
                DocumentStatus::Quarantined
            } else {
                DocumentStatus::Ready
            };
            transaction.execute(
                "INSERT INTO knowledge_documents
                    (id, base_id, title, source_kind, source_ref, status, verified, version,
                     current_revision_id, content_hash, chunk_count, import_key,
                     created_at, updated_at, deleted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, NULL, ?8, 0, ?9, ?10, ?10, NULL)",
                params![
                    document_id,
                    base_id,
                    title,
                    draft.source_kind.as_str(),
                    source_ref,
                    status.as_str(),
                    i64::from(draft.verified),
                    body_hash,
                    import_key,
                    now as i64
                ],
            )?;
            let revision_id = insert_revision(
                &transaction,
                &document_id,
                base_id,
                1,
                title,
                &draft.body,
                &body_hash,
                &new_chunks,
                now,
            )?;
            transaction.execute(
                "UPDATE knowledge_documents
                 SET current_revision_id = ?1, chunk_count = ?2
                 WHERE id = ?3",
                params![revision_id, new_chunks.len() as i64, document_id],
            )?;
            KnowledgeDocument {
                id: document_id,
                base_id: base_id.to_owned(),
                title: title.to_owned(),
                source_kind: draft.source_kind,
                source_ref: source_ref.map(str::to_owned),
                status,
                verified: draft.verified,
                version: 1,
                revision_id,
                content_hash: body_hash,
                chunk_count: new_chunks.len(),
                created_at: now,
                updated_at: now,
            }
        };
        transaction.commit()?;
        Ok(document)
    }

    pub fn inspect_document(
        &self,
        id: &str,
        project_path: Option<&str>,
    ) -> KnowledgeResult<DocumentDetail> {
        validate_id(id)?;
        validate_scope(project_path)?;
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT d.id, d.base_id, d.title, d.source_kind, d.source_ref, d.status,
                        d.verified, d.version, d.current_revision_id, d.content_hash,
                        d.chunk_count, d.created_at, d.updated_at, r.body, r.id
                 FROM knowledge_documents d
                 JOIN knowledge_bases b ON b.id = d.base_id
                 JOIN knowledge_revisions r ON r.id = d.current_revision_id
                 WHERE d.id = ?1 AND d.deleted_at IS NULL
                   AND (b.project_path IS NULL OR (?2 IS NOT NULL AND b.project_path = ?2)
                 )",
                params![id, project_path],
                |row| {
                    Ok((
                        document_from_row(row)?,
                        row.get::<_, String>(13)?,
                        row.get::<_, String>(14)?,
                    ))
                },
            )
            .optional()?
            .ok_or(KnowledgeError::NotFound)?;
        if row.0.revision_id != row.2 {
            return Err(KnowledgeError::KnowledgeUnavailable);
        }
        let chunks = chunks_for_revision(&connection, &row.0.revision_id)?;
        Ok(DocumentDetail {
            document: row.0,
            body: row.1,
            chunks,
        })
    }

    pub fn set_enabled(
        &self,
        id: &str,
        project_path: Option<&str>,
        expected_version: u64,
        enabled: bool,
    ) -> KnowledgeResult<KnowledgeDocument> {
        validate_id(id)?;
        validate_scope(project_path)?;
        let expected = sql_u64(expected_version)?;
        let mut connection = self.connection()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut document = get_document(&transaction, id, project_path)?;
        if document.version != expected_version {
            return Err(KnowledgeError::RevisionConflict);
        }
        if document.status == DocumentStatus::Quarantined {
            if enabled {
                return Err(KnowledgeError::InvalidInput);
            }
            transaction.commit()?;
            return Ok(document);
        }
        let status = if enabled {
            DocumentStatus::Ready
        } else {
            DocumentStatus::Disabled
        };
        if status == document.status {
            transaction.commit()?;
            return Ok(document);
        }
        let now = now_secs();
        let changed = transaction.execute(
            "UPDATE knowledge_documents AS d
             SET status = ?1, version = version + 1, updated_at = ?2
             WHERE d.id = ?3 AND d.version = ?4 AND d.deleted_at IS NULL
               AND EXISTS (
                   SELECT 1 FROM knowledge_bases b WHERE b.id = d.base_id
                     AND (b.project_path IS NULL OR (?5 IS NOT NULL AND b.project_path = ?5))
               )",
            params![status.as_str(), now as i64, id, expected, project_path],
        )?;
        if changed != 1 {
            return Err(KnowledgeError::RevisionConflict);
        }
        document.status = status;
        document.version += 1;
        document.updated_at = now;
        transaction.commit()?;
        Ok(document)
    }

    pub fn delete_document(
        &self,
        id: &str,
        project_path: Option<&str>,
        expected_version: u64,
    ) -> KnowledgeResult<()> {
        validate_id(id)?;
        validate_scope(project_path)?;
        let expected = sql_u64(expected_version)?;
        let mut connection = self.connection()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let document = get_document(&transaction, id, project_path)?;
        if document.version != expected_version {
            return Err(KnowledgeError::RevisionConflict);
        }
        let now = now_secs();
        let changed = transaction.execute(
            "UPDATE knowledge_documents AS d
             SET deleted_at = ?1, version = version + 1, updated_at = ?1
             WHERE d.id = ?2 AND d.version = ?3 AND d.deleted_at IS NULL
               AND EXISTS (
                   SELECT 1 FROM knowledge_bases b WHERE b.id = d.base_id
                     AND (b.project_path IS NULL OR (?4 IS NOT NULL AND b.project_path = ?4))
               )",
            params![now as i64, id, expected, project_path],
        )?;
        if changed != 1 {
            return Err(KnowledgeError::RevisionConflict);
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn search(&self, query: &SearchQuery) -> KnowledgeResult<Vec<KnowledgeHit>> {
        validate_scope(query.project_path.as_deref())?;
        if query.base_ids.len() > MAX_SELECTED_BASES
            || query.limit > MAX_SEARCH_LIMIT
            || query.budget_tokens > MAX_SEARCH_BUDGET
        {
            return Err(KnowledgeError::InvalidInput);
        }
        validate_ids(&query.base_ids, MAX_SELECTED_BASES)?;
        let terms = query_terms(&query.text)?;
        if query.base_ids.is_empty() {
            return Ok(Vec::new());
        }

        let connection = self.connection()?;
        ensure_selected_bases_access(&connection, &query.base_ids, query.project_path.as_deref())?;
        if query.limit == 0 || query.budget_tokens == 0 {
            return Ok(Vec::new());
        }
        ensure_search_row_budget(&connection, &query.base_ids, query.project_path.as_deref())?;
        let fetch = query.limit.saturating_mul(8).min(MAX_CANDIDATES);
        let fts = fts_search(
            &connection,
            &terms,
            &query.base_ids,
            query.project_path.as_deref(),
            fetch,
        )?;
        let exact = exact_search(
            &connection,
            &terms,
            &query.base_ids,
            query.project_path.as_deref(),
            fetch,
        )?;

        let mut ranked: HashMap<String, RankedHit> = HashMap::new();
        for (rank, mut hit) in fts.into_iter().enumerate() {
            hit.channels.push("fts".to_owned());
            merge_hit(&mut ranked, hit, rank);
        }
        for (rank, mut hit) in exact.into_iter().enumerate() {
            if let Some(existing) = ranked.get_mut(&hit.chunk_id) {
                if !existing
                    .hit
                    .channels
                    .iter()
                    .any(|channel| channel == "exact")
                {
                    existing.hit.channels.push("exact".to_owned());
                }
                existing.score += 1.0 / (RRF_K + rank as f64 + 1.0);
            } else {
                hit.channels.push("exact".to_owned());
                merge_hit(&mut ranked, hit, rank);
            }
        }

        let mut ranked: Vec<RankedHit> = ranked.into_values().collect();
        ranked.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.document_id().cmp(right.document_id()))
                .then_with(|| left.line_start().cmp(&right.line_start()))
                .then_with(|| left.hit.chunk_id.cmp(&right.hit.chunk_id))
        });

        let mut hits = Vec::new();
        let mut tokens_used = 0usize;
        for candidate in ranked {
            let cost = estimate_tokens(&candidate.hit);
            if tokens_used.saturating_add(cost) > query.budget_tokens {
                continue;
            }
            tokens_used += cost;
            hits.push(candidate.hit);
            if hits.len() >= query.limit {
                break;
            }
        }
        Ok(hits)
    }

    pub fn get_bindings(
        &self,
        session_id: &str,
        project_path: Option<&str>,
    ) -> KnowledgeResult<KnowledgeSelection> {
        validate_session(session_id)?;
        validate_scope(project_path)?;
        let connection = self.connection()?;
        let raw = connection
            .query_row(
                "SELECT selection_json FROM knowledge_bindings
                 WHERE session_id = ?1 AND project_path_key = ?2",
                params![session_id, scope_key(project_path)],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(raw) = raw else {
            return Ok(KnowledgeSelection::default());
        };
        let selection: KnowledgeSelection = serde_json::from_str(&raw)?;
        validate_selection(&selection)?;
        ensure_selected_bases_access(&connection, &selection.base_ids, project_path)?;
        Ok(selection)
    }

    pub fn set_bindings(
        &self,
        session_id: &str,
        project_path: Option<&str>,
        selection: &KnowledgeSelection,
    ) -> KnowledgeResult<()> {
        validate_session(session_id)?;
        validate_scope(project_path)?;
        validate_selection(selection)?;
        let mut connection = self.connection()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure_selected_bases_access(&transaction, &selection.base_ids, project_path)?;
        let json = serde_json::to_string(selection)?;
        transaction.execute(
            "INSERT INTO knowledge_bindings(session_id, project_path_key, selection_json, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id, project_path_key)
             DO UPDATE SET selection_json = excluded.selection_json,
                           updated_at = excluded.updated_at",
            params![session_id, scope_key(project_path), json, now_secs() as i64],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn record_citations(
        &self,
        session_id: &str,
        turn_seq: u64,
        project_path: Option<&str>,
        hits: &[KnowledgeHit],
    ) -> KnowledgeResult<()> {
        validate_session(session_id)?;
        validate_scope(project_path)?;
        let turn_seq = sql_u64(turn_seq)?;
        if hits.len() > MAX_CITATIONS_PER_TURN {
            return Err(KnowledgeError::InvalidInput);
        }
        let mut seen = std::collections::HashSet::new();
        if hits.iter().any(|hit| !seen.insert(hit.chunk_id.as_str())) {
            return Err(KnowledgeError::InvalidInput);
        }

        let mut connection = self.connection()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT 1 FROM knowledge_turns
                 WHERE session_id = ?1 AND project_path_key = ?2 AND turn_seq = ?3",
                params![session_id, scope_key(project_path), turn_seq],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if existing {
            let previous =
                read_citations_from(&transaction, session_id, turn_seq as u64, project_path)?;
            if previous.hits == hits {
                transaction.commit()?;
                return Ok(());
            }
            return Err(KnowledgeError::RevisionConflict);
        }

        for hit in hits {
            validate_hit(hit)?;
            verify_hit(&transaction, hit, project_path)?;
        }
        transaction.execute(
            "INSERT INTO knowledge_turns(session_id, project_path_key, turn_seq, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                session_id,
                scope_key(project_path),
                turn_seq,
                now_secs() as i64
            ],
        )?;
        for (position, hit) in hits.iter().enumerate() {
            let channels = serde_json::to_string(&hit.channels)?;
            transaction.execute(
                "INSERT INTO knowledge_turn_citations
                    (session_id, project_path_key, turn_seq, position, base_id,
                     document_id, revision_id, chunk_id, channels_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    session_id,
                    scope_key(project_path),
                    turn_seq,
                    position as i64,
                    hit.base_id,
                    hit.document_id,
                    hit.revision_id,
                    hit.chunk_id,
                    channels
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn read_citations(
        &self,
        session_id: &str,
        turn_seq: u64,
        project_path: Option<&str>,
    ) -> KnowledgeResult<TurnKnowledge> {
        validate_session(session_id)?;
        validate_scope(project_path)?;
        sql_u64(turn_seq)?;
        let connection = self.connection()?;
        read_citations_from(&connection, session_id, turn_seq, project_path)
    }

    /// Returns whether the turn carried knowledge context. The durable turn
    /// marker survives empty evidence sets, disable, tombstoning, and lost
    /// citation rows so history filtering remains effective later.
    pub fn has_citations(&self, session_id: &str, turn_seq: u64) -> KnowledgeResult<bool> {
        validate_session(session_id)?;
        let turn_seq = sql_u64(turn_seq)?;
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT EXISTS (
                     SELECT 1 FROM knowledge_turns
                     WHERE session_id = ?1 AND turn_seq = ?2
                 )",
                params![session_id, turn_seq],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    fn connection(&self) -> KnowledgeResult<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| KnowledgeError::StorageUnavailable)
    }
}

fn ensure_real_directory(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "knowledge storage path is not a directory",
        ))
    }
}

#[cfg(unix)]
fn set_private_permissions(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(unix)]
fn ensure_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    loop {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                return set_private_permissions(path, 0o700);
            }
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "knowledge storage path is not a directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::DirBuilder::new().mode(0o700).create(path) {
                    Ok(()) => return set_private_permissions(path, 0o700),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(not(unix))]
fn ensure_private_directory(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    ensure_real_directory(path)
}

#[cfg(unix)]
fn prepare_private_database(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    loop {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                return set_private_file_permissions(path);
            }
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "knowledge catalog is not a regular file",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(path)
                {
                    Ok(file) => {
                        drop(file);
                        return set_private_file_permissions(path);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "knowledge catalog is not a regular file",
        ));
    }
    set_private_permissions(path, 0o600)
}

fn base_from_row(row: &Row<'_>) -> rusqlite::Result<KnowledgeBase> {
    Ok(KnowledgeBase {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        project_path: row.get(3)?,
        created_at: from_sql_u64(row.get(4)?),
        version: from_sql_u64(row.get(5)?),
    })
}

fn document_from_row(row: &Row<'_>) -> rusqlite::Result<KnowledgeDocument> {
    let source_kind: String = row.get(3)?;
    let status: String = row.get(5)?;
    Ok(KnowledgeDocument {
        id: row.get(0)?,
        base_id: row.get(1)?,
        title: row.get(2)?,
        source_kind: SourceKind::parse(&source_kind),
        source_ref: row.get(4)?,
        status: DocumentStatus::parse(&status),
        verified: row.get::<_, i64>(6)? != 0,
        version: from_sql_u64(row.get(7)?),
        revision_id: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
        content_hash: row.get(9)?,
        chunk_count: usize::try_from(row.get::<_, i64>(10)?).unwrap_or(0),
        created_at: from_sql_u64(row.get(11)?),
        updated_at: from_sql_u64(row.get(12)?),
    })
}

fn ensure_base_access(
    connection: &Connection,
    base_id: &str,
    project_path: Option<&str>,
) -> KnowledgeResult<()> {
    let base_path = connection
        .query_row(
            "SELECT project_path FROM knowledge_bases WHERE id = ?1",
            params![base_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .ok_or(KnowledgeError::NotFound)?;
    if base_accessible(base_path.as_deref(), project_path) {
        Ok(())
    } else {
        Err(KnowledgeError::ScopeDenied)
    }
}

fn ensure_selected_bases_access(
    connection: &Connection,
    base_ids: &[String],
    project_path: Option<&str>,
) -> KnowledgeResult<()> {
    validate_ids(base_ids, MAX_SELECTED_BASES)?;
    if base_ids.is_empty() {
        return Ok(());
    }
    let placeholders = numbered_placeholders(2, base_ids.len());
    let sql = format!(
        "SELECT COUNT(*) FROM knowledge_bases b
         WHERE (b.project_path IS NULL OR (?1 IS NOT NULL AND b.project_path = ?1))
           AND b.id IN ({placeholders})"
    );
    let mut values = vec![option_text_value(project_path)];
    values.extend(base_ids.iter().cloned().map(Value::Text));
    let count: i64 =
        connection.query_row(&sql, params_from_iter(values.iter()), |row| row.get(0))?;
    if count as usize != base_ids.len() {
        return Err(KnowledgeError::ScopeDenied);
    }
    Ok(())
}

fn ensure_search_row_budget(
    connection: &Connection,
    base_ids: &[String],
    project_path: Option<&str>,
) -> KnowledgeResult<()> {
    let ids = numbered_placeholders(2, base_ids.len());
    let sql = format!(
        "SELECT COUNT(*) FROM (
             SELECT 1
             FROM knowledge_chunks c
             JOIN knowledge_documents d ON d.id = c.document_id
             JOIN knowledge_bases b ON b.id = d.base_id
             JOIN knowledge_revisions r ON r.id = c.revision_id
             WHERE (b.project_path IS NULL OR (?1 IS NOT NULL AND b.project_path = ?1))
               AND b.id IN ({ids})
               AND d.deleted_at IS NULL AND d.status = 'ready'
               AND d.current_revision_id = r.id AND c.revision_id = r.id
             LIMIT {}
         )",
        MAX_SEARCHABLE_CHUNKS + 1
    );
    let mut values = vec![option_text_value(project_path)];
    values.extend(base_ids.iter().cloned().map(Value::Text));
    let count: i64 =
        connection.query_row(&sql, params_from_iter(values.iter()), |row| row.get(0))?;
    if count as usize > MAX_SEARCHABLE_CHUNKS {
        return Err(KnowledgeError::InvalidInput);
    }
    Ok(())
}

fn base_accessible(base_path: Option<&str>, caller_path: Option<&str>) -> bool {
    match base_path {
        None => true,
        Some(base_path) => caller_path == Some(base_path),
    }
}

fn find_existing_import(
    connection: &Connection,
    base_id: &str,
    key: &str,
) -> KnowledgeResult<Option<ExistingImport>> {
    connection
        .query_row(
            "SELECT d.id, d.base_id, d.title, d.source_kind, d.source_ref, d.status,
                    d.verified, d.version, d.current_revision_id, d.content_hash,
                    d.chunk_count, d.created_at, d.updated_at, d.deleted_at,
                    r.title, r.body
             FROM knowledge_documents d
             LEFT JOIN knowledge_revisions r ON r.id = d.current_revision_id
             WHERE d.base_id = ?1 AND d.import_key = ?2",
            params![base_id, key],
            |row| {
                Ok(ExistingImport {
                    document: document_from_row(row)?,
                    deleted: row.get::<_, Option<i64>>(13)?.is_some(),
                    revision_title: row.get::<_, Option<String>>(14)?.unwrap_or_default(),
                    revision_body: row.get::<_, Option<String>>(15)?.unwrap_or_default(),
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn import_key(
    draft: &ImportDraft,
    source_ref: Option<&str>,
    title: &str,
    body_hash: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut identity = Vec::new();
    identity.extend_from_slice(draft.source_kind.as_str().as_bytes());
    identity.push(0);
    if let Some(source_ref) = source_ref {
        identity.extend_from_slice(b"source\0");
        identity.extend_from_slice(source_ref.as_bytes());
    } else {
        identity.extend_from_slice(b"content\0");
        identity.extend_from_slice(title.as_bytes());
        identity.push(0);
        identity.extend_from_slice(body_hash.as_bytes());
    }
    hex::encode(Sha256::digest(identity))
}

fn insert_revision(
    transaction: &Transaction<'_>,
    document_id: &str,
    base_id: &str,
    version: u64,
    title: &str,
    body: &str,
    body_hash: &str,
    chunks: &[TextChunk],
    created_at: u64,
) -> KnowledgeResult<String> {
    if chunks.is_empty() || chunks.len() > MAX_CHUNKS_PER_DOCUMENT {
        return Err(KnowledgeError::InvalidInput);
    }
    let existing_chunks: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM knowledge_chunks WHERE base_id = ?1",
        params![base_id],
        |row| row.get(0),
    )?;
    if usize::try_from(existing_chunks)
        .unwrap_or(usize::MAX)
        .saturating_add(chunks.len())
        > MAX_CHUNKS_PER_BASE
    {
        return Err(KnowledgeError::InvalidInput);
    }
    let revision_id = uuid::Uuid::new_v4().to_string();
    transaction.execute(
        "INSERT INTO knowledge_revisions
            (id, document_id, version, title, body, content_hash, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            revision_id,
            document_id,
            sql_u64(version)?,
            title,
            body,
            body_hash,
            sql_u64(created_at)?
        ],
    )?;
    for chunk in chunks {
        let chunk_id = uuid::Uuid::new_v4().to_string();
        transaction.execute(
            "INSERT INTO knowledge_chunks
                (id, document_id, base_id, revision_id, heading, line_start,
                 line_end, body, search_terms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                chunk_id,
                document_id,
                base_id,
                revision_id,
                chunk.heading,
                chunk.line_start as i64,
                chunk.line_end as i64,
                chunk.body,
                chunk.search_terms
            ],
        )?;
        let row_id = transaction.last_insert_rowid();
        transaction.execute(
            "INSERT INTO knowledge_chunks_fts(rowid, title, heading, body, search_terms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![row_id, title, chunk.heading, chunk.body, chunk.search_terms],
        )?;
    }
    Ok(revision_id)
}

fn chunks_for_revision(
    connection: &Connection,
    revision_id: &str,
) -> KnowledgeResult<Vec<KnowledgeChunk>> {
    let mut statement = connection.prepare(
        "SELECT id, revision_id, heading, line_start, line_end, body
         FROM knowledge_chunks WHERE revision_id = ?1
         ORDER BY line_start, rowid",
    )?;
    let rows = statement
        .query_map(params![revision_id], |row| {
            Ok(KnowledgeChunk {
                id: row.get(0)?,
                revision_id: row.get(1)?,
                heading: row.get(2)?,
                line_start: usize::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                line_end: usize::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                body: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(KnowledgeError::from)?;
    Ok(rows)
}

fn get_document(
    connection: &Connection,
    id: &str,
    project_path: Option<&str>,
) -> KnowledgeResult<KnowledgeDocument> {
    connection
        .query_row(
            "SELECT d.id, d.base_id, d.title, d.source_kind, d.source_ref, d.status,
                    d.verified, d.version, d.current_revision_id, d.content_hash,
                    d.chunk_count, d.created_at, d.updated_at
             FROM knowledge_documents d
             JOIN knowledge_bases b ON b.id = d.base_id
             WHERE d.id = ?1 AND d.deleted_at IS NULL
               AND (b.project_path IS NULL OR (?2 IS NOT NULL AND b.project_path = ?2))",
            params![id, project_path],
            document_from_row,
        )
        .optional()?
        .ok_or(KnowledgeError::NotFound)
}

fn fts_search(
    connection: &Connection,
    terms: &[String],
    base_ids: &[String],
    project_path: Option<&str>,
    limit: usize,
) -> KnowledgeResult<Vec<KnowledgeHit>> {
    let expression = terms
        .iter()
        .map(|term| format!("\"{}\"", term.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ");
    let ids = numbered_placeholders(3, base_ids.len());
    let limit_index = 3 + base_ids.len();
    let sql = format!(
        "SELECT b.id, d.id, r.id, c.id, r.title, c.heading,
                c.line_start, c.line_end, c.body
         FROM knowledge_chunks_fts f
         JOIN knowledge_chunks c ON c.rowid = f.rowid
         JOIN knowledge_documents d ON d.id = c.document_id
         JOIN knowledge_bases b ON b.id = d.base_id
         JOIN knowledge_revisions r ON r.id = c.revision_id
         WHERE knowledge_chunks_fts MATCH ?2
           AND (b.project_path IS NULL OR (?1 IS NOT NULL AND b.project_path = ?1))
           AND b.id IN ({ids})
           AND d.deleted_at IS NULL AND d.status = 'ready'
           AND d.current_revision_id = r.id AND c.revision_id = r.id
         ORDER BY bm25(knowledge_chunks_fts), c.rowid
         LIMIT ?{limit_index}"
    );
    let mut values = vec![option_text_value(project_path), Value::Text(expression)];
    values.extend(base_ids.iter().cloned().map(Value::Text));
    values.push(Value::Integer(limit as i64));
    let mut statement = connection.prepare(&sql)?;
    let rows = statement
        .query_map(params_from_iter(values.iter()), hit_from_row)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(KnowledgeError::from)?;
    Ok(rows)
}

fn exact_search(
    connection: &Connection,
    terms: &[String],
    base_ids: &[String],
    project_path: Option<&str>,
    limit: usize,
) -> KnowledgeResult<Vec<KnowledgeHit>> {
    let terms_start = 2 + base_ids.len();
    let terms_end = terms_start + terms.len();
    let exact_score = (terms_start..terms_end)
        .map(|index| {
            format!(
                "CASE WHEN instr(' ' || c.search_terms || ' ', ' ' || ?{index} || ' ') > 0
                      THEN 1 ELSE 0 END"
            )
        })
        .collect::<Vec<_>>()
        .join(" + ");
    let ids = numbered_placeholders(2, base_ids.len());
    let limit_index = terms_end;
    let sql = format!(
        "WITH candidates AS (
             SELECT b.id AS base_id, d.id AS document_id, r.id AS revision_id,
                    c.id AS chunk_id, r.title, c.heading, c.line_start, c.line_end,
                    c.body AS quote, ({exact_score}) AS exact_score
             FROM knowledge_chunks c
             JOIN knowledge_documents d ON d.id = c.document_id
             JOIN knowledge_bases b ON b.id = d.base_id
             JOIN knowledge_revisions r ON r.id = c.revision_id
             WHERE (b.project_path IS NULL OR (?1 IS NOT NULL AND b.project_path = ?1))
               AND b.id IN ({ids})
               AND d.deleted_at IS NULL AND d.status = 'ready'
               AND d.current_revision_id = r.id AND c.revision_id = r.id
         )
         SELECT base_id, document_id, revision_id, chunk_id, title, heading,
                line_start, line_end, quote
         FROM candidates WHERE exact_score > 0
         ORDER BY exact_score DESC, document_id, line_start, chunk_id
         LIMIT ?{limit_index}"
    );
    let mut values = vec![option_text_value(project_path)];
    values.extend(base_ids.iter().cloned().map(Value::Text));
    values.extend(terms.iter().cloned().map(Value::Text));
    values.push(Value::Integer(limit as i64));
    let mut statement = connection.prepare(&sql)?;
    let rows = statement
        .query_map(params_from_iter(values.iter()), hit_from_row)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(KnowledgeError::from)?;
    Ok(rows)
}

fn hit_from_row(row: &Row<'_>) -> rusqlite::Result<KnowledgeHit> {
    Ok(KnowledgeHit {
        base_id: row.get(0)?,
        document_id: row.get(1)?,
        revision_id: row.get(2)?,
        chunk_id: row.get(3)?,
        title: row.get(4)?,
        heading: row.get(5)?,
        line_start: usize::try_from(row.get::<_, i64>(6)?).unwrap_or(0),
        line_end: usize::try_from(row.get::<_, i64>(7)?).unwrap_or(0),
        quote: row.get(8)?,
        channels: Vec::new(),
    })
}

fn merge_hit(ranked: &mut HashMap<String, RankedHit>, hit: KnowledgeHit, rank: usize) {
    ranked.insert(
        hit.chunk_id.clone(),
        RankedHit {
            hit,
            score: 1.0 / (RRF_K + rank as f64 + 1.0),
        },
    );
}

impl RankedHit {
    fn document_id(&self) -> &str {
        &self.hit.document_id
    }

    fn line_start(&self) -> usize {
        self.hit.line_start
    }
}

fn estimate_tokens(hit: &KnowledgeHit) -> usize {
    let mut total = 0usize;
    let mut non_cjk_run = 0usize;
    for ch in hit
        .title
        .chars()
        .chain(hit.heading.chars())
        .chain(hit.quote.chars())
    {
        if is_cjk(ch) {
            total = total.saturating_add(non_cjk_run.saturating_add(3) / 4);
            non_cjk_run = 0;
            total = total.saturating_add(1);
        } else if ch.is_alphanumeric() {
            non_cjk_run = non_cjk_run.saturating_add(1);
        } else {
            total = total.saturating_add(non_cjk_run.saturating_add(3) / 4);
            non_cjk_run = 0;
        }
    }
    total
        .saturating_add(non_cjk_run.saturating_add(3) / 4)
        .max(1)
}

fn verify_hit(
    connection: &Connection,
    hit: &KnowledgeHit,
    project_path: Option<&str>,
) -> KnowledgeResult<()> {
    let actual = connection
        .query_row(
            "SELECT b.id, d.id, r.id, c.id, r.title, c.heading,
                    c.line_start, c.line_end, c.body
             FROM knowledge_chunks c
             JOIN knowledge_documents d ON d.id = c.document_id
             JOIN knowledge_bases b ON b.id = d.base_id
             JOIN knowledge_revisions r ON r.id = c.revision_id
             WHERE b.id = ?1 AND d.id = ?2 AND r.id = ?3 AND c.id = ?4
               AND d.deleted_at IS NULL AND d.status = 'ready'
               AND (b.project_path IS NULL OR (?5 IS NOT NULL AND b.project_path = ?5)
             )",
            params![
                hit.base_id,
                hit.document_id,
                hit.revision_id,
                hit.chunk_id,
                project_path
            ],
            hit_from_row,
        )
        .optional()?
        .ok_or(KnowledgeError::NotFound)?;
    if actual.base_id != hit.base_id
        || actual.document_id != hit.document_id
        || actual.revision_id != hit.revision_id
        || actual.chunk_id != hit.chunk_id
        || actual.title != hit.title
        || actual.heading != hit.heading
        || actual.line_start != hit.line_start
        || actual.line_end != hit.line_end
        || actual.quote != hit.quote
    {
        return Err(KnowledgeError::InvalidInput);
    }
    Ok(())
}

fn validate_hit(hit: &KnowledgeHit) -> KnowledgeResult<()> {
    validate_id(&hit.base_id)?;
    validate_id(&hit.document_id)?;
    validate_id(&hit.revision_id)?;
    validate_id(&hit.chunk_id)?;
    if hit.channels.is_empty()
        || hit.channels.len() > 2
        || hit
            .channels
            .iter()
            .any(|channel| !matches!(channel.as_str(), "fts" | "exact"))
        || hit
            .channels
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != hit.channels.len()
    {
        return Err(KnowledgeError::InvalidInput);
    }
    Ok(())
}

fn read_citations_from(
    connection: &Connection,
    session_id: &str,
    turn_seq: u64,
    project_path: Option<&str>,
) -> KnowledgeResult<TurnKnowledge> {
    let turn_seq_sql = sql_u64(turn_seq)?;
    let scope = scope_key(project_path);
    let exists = connection
        .query_row(
            "SELECT 1 FROM knowledge_turns
             WHERE session_id = ?1 AND project_path_key = ?2 AND turn_seq = ?3",
            params![session_id, scope, turn_seq_sql],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Err(KnowledgeError::NotFound);
    }
    let mut statement = connection.prepare(
        "SELECT b.id, d.id, r.id, c.id, r.title, c.heading,
                c.line_start, c.line_end, c.body, tc.channels_json
         FROM knowledge_turn_citations tc
         JOIN knowledge_chunks c ON c.id = tc.chunk_id AND c.revision_id = tc.revision_id
         JOIN knowledge_revisions r ON r.id = c.revision_id AND r.id = tc.revision_id
         JOIN knowledge_documents d ON d.id = c.document_id AND d.id = tc.document_id
         JOIN knowledge_bases b ON b.id = d.base_id AND b.id = tc.base_id
         WHERE tc.session_id = ?1 AND tc.project_path_key = ?2 AND tc.turn_seq = ?3
           AND d.deleted_at IS NULL
           AND (b.project_path IS NULL OR (?4 IS NOT NULL AND b.project_path = ?4))
         ORDER BY tc.position",
    )?;
    let rows = statement
        .query_map(
            params![session_id, scope, turn_seq_sql, project_path],
            |row| {
                let mut hit = hit_from_row(row)?;
                hit.channels =
                    serde_json::from_str(&row.get::<_, String>(9)?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            9,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                Ok(hit)
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;

    // The number of persisted citation refs must match the visible joined rows.
    // This turns deleted or inaccessible sources into a non-leaking failure.
    let stored_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM knowledge_turn_citations
         WHERE session_id = ?1 AND project_path_key = ?2 AND turn_seq = ?3",
        params![session_id, scope, turn_seq_sql],
        |row| row.get(0),
    )?;
    if rows.len() as i64 != stored_count {
        return Err(KnowledgeError::NotFound);
    }
    Ok(TurnKnowledge {
        session_id: session_id.to_owned(),
        turn_seq,
        hits: rows,
    })
}

fn validate_selection(selection: &KnowledgeSelection) -> KnowledgeResult<()> {
    validate_ids(&selection.base_ids, MAX_SELECTED_BASES)
}

fn validate_ids(ids: &[String], max: usize) -> KnowledgeResult<()> {
    if ids.len() > max {
        return Err(KnowledgeError::InvalidInput);
    }
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        validate_id(id)?;
        if !seen.insert(id) {
            return Err(KnowledgeError::InvalidInput);
        }
    }
    Ok(())
}

fn validate_id(value: &str) -> KnowledgeResult<()> {
    if value.is_empty() || value.len() > 128 || value.chars().any(|ch| ch.is_control()) {
        return Err(KnowledgeError::InvalidInput);
    }
    Ok(())
}

fn validate_session(value: &str) -> KnowledgeResult<()> {
    if value.trim().is_empty()
        || value.chars().count() > 256
        || value.chars().any(|ch| ch.is_control())
    {
        return Err(KnowledgeError::InvalidInput);
    }
    Ok(())
}

fn validate_scope(project_path: Option<&str>) -> KnowledgeResult<()> {
    if project_path.is_some_and(|path| {
        path.trim().is_empty() || path.chars().count() > MAX_SOURCE_CHARS || path.contains('\0')
    }) {
        return Err(KnowledgeError::InvalidInput);
    }
    Ok(())
}

fn validate_name(value: &str, max_chars: usize) -> KnowledgeResult<()> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > max_chars
        || trimmed.chars().any(|ch| ch.is_control())
    {
        return Err(KnowledgeError::InvalidInput);
    }
    Ok(())
}

fn validate_short_text(value: &str, max_chars: usize) -> KnowledgeResult<()> {
    if value.chars().count() > max_chars || value.contains('\0') {
        return Err(KnowledgeError::InvalidInput);
    }
    Ok(())
}

fn sql_u64(value: u64) -> KnowledgeResult<i64> {
    i64::try_from(value).map_err(|_| KnowledgeError::InvalidInput)
}

fn from_sql_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn numbered_placeholders(start: usize, count: usize) -> String {
    (start..start + count)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn option_text_value(value: Option<&str>) -> Value {
    value.map_or(Value::Null, |value| Value::Text(value.to_owned()))
}

fn scope_key(project_path: Option<&str>) -> String {
    project_path.unwrap_or_default().to_owned()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3400..=0x4DBF |
        0x4E00..=0x9FFF |
        0xF900..=0xFAFF |
        0x20000..=0x2FA1F)
}
