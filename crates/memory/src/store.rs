//! SQLite-backed store. One short-lived connection per operation — per the
//! implementation doc, this keeps the engine fail-open and lock-free.
//!
//! The store is the *only* module that issues raw SQL. The DAO methods
//! here are what the rest of the crate builds on. Higher-level logic
//! (recall, compile, replay) lives in the other modules and uses these
//! primitives.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value as Json;

use crate::error::MemoryResult;
use crate::metrics::Metrics;
use crate::model::{
    Authority, MemoryEntry, MemoryFilter, MemoryKind, MemoryScope, MemoryStatus, SourceTrust,
    ValidityLabel,
};

const MIGRATION_0001: &str = include_str!("../migrations/0001_initial.sql");
const MIGRATION_0002: &str = include_str!("../migrations/0002_tombstones.sql");

/// Clock used everywhere in the engine. Wrapped so tests can override it
/// without `chrono`.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Apply the standard KME pragmas. Called on every connection open.
fn apply_pragmas(conn: &Connection) -> MemoryResult<()> {
    // WAL is mandatory for read/write concurrency and crash safety.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    // 3s matches doc §13.2. Long enough for cross-process syncs, short
    // enough to fail fast in the hot path.
    conn.pragma_update(None, "busy_timeout", 3000_i64)?;
    // We use FOREIGN KEY in the schema. SQLite is FK-off by default.
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // synchronous=NORMAL is the WAL default; we make it explicit.
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    // Defensive: keep the temp store in memory.
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

fn open_connection(path: &Path) -> MemoryResult<Connection> {
    let conn = Connection::open(path)?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

/// Apply pending migrations idempotently. We record the version in
/// `schema_migrations` and skip already-applied versions on re-open.
pub fn apply_migrations(conn: &mut Connection) -> MemoryResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL,
            description TEXT NOT NULL
        )",
        [],
    )?;
    let applied: Vec<u32> = conn
        .prepare("SELECT version FROM schema_migrations")?
        .query_map([], |r| r.get::<_, i64>(0))?
        .filter_map(|r| r.ok())
        .map(|v| v as u32)
        .collect();

    let migrations: &[(u32, &str, &str)] = &[
        (1, MIGRATION_0001, "initial schema + fts5"),
        (2, MIGRATION_0002, "tombstones"),
    ];

    let now = now_secs() as i64;
    for (version, sql, desc) in migrations {
        if applied.contains(version) {
            continue;
        }
        // Each migration runs in its own transaction; the DDL is mostly
        // idempotent already (`IF NOT EXISTS` everywhere) so re-runs are
        // safe.
        let tx: Transaction<'_> = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at, description) \
             VALUES (?1, ?2, ?3)",
            params![*version as i64, now, *desc],
        )?;
        tx.commit()?;
    }
    Ok(())
}

/// The central store. Cheap to clone (`Arc` internals).
#[derive(Clone)]
pub struct Store {
    pub(crate) path: Arc<PathBuf>,
    pub(crate) metrics: Arc<Metrics>,
}

impl Store {
    /// Open the store at `path`, creating the schema if needed.
    pub fn open(path: impl Into<PathBuf>, metrics: Arc<Metrics>) -> MemoryResult<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut conn = open_connection(&path)?;
        apply_migrations(&mut conn)?;
        Ok(Self {
            path: Arc::new(path),
            metrics,
        })
    }

    /// Open a fresh connection on demand. The closure receives a
    /// `&mut Connection` because SQLite transactions require
    /// `&mut self`. The connection is closed when the closure returns.
    pub fn with_conn<R>(
        &self,
        f: impl FnOnce(&mut Connection) -> MemoryResult<R>,
    ) -> MemoryResult<R> {
        let mut conn = open_connection(&self.path)?;
        f(&mut conn)
    }

    /// Force-run migrations on the file (idempotent).
    pub fn ensure_schema(&self) -> MemoryResult<()> {
        self.with_conn(apply_migrations)
    }

    // ---- DAO ---------------------------------------------------------------

    /// Insert or update an entry. Update is keyed on `id`; if the row
    /// already exists, `version` is incremented and `updated_at` bumped.
    /// The FTS index is kept in sync inside the same transaction.
    pub fn upsert_entry(&self, entry: &MemoryEntry) -> MemoryResult<()> {
        self.with_conn(|c| {
            let tx = c.transaction()?;
            let existing_version: Option<i64> = tx
                .query_row(
                    "SELECT version FROM memory_entries WHERE id = ?1",
                    params![entry.id],
                    |r| r.get(0),
                )
                .optional()?;
            let new_version = existing_version.map(|v| v + 1).unwrap_or(1);
            let body_json = serde_json::to_string(&entry.body)?;
            let conditions_json = serde_json::to_string(&entry.conditions)?;
            let source_uris_json = serde_json::to_string(&entry.source_uris)?;
            tx.execute(
                "INSERT INTO memory_entries(
                    id, kind, status, authority, title, abstract_text, body_json, content_hash,
                    source_trust, confidence, verified, candidate_skill,
                    project_id, repository_id, branch, worktree_id, session_id, task_id, subtask_id,
                    conditions_json, source_uris_json,
                    valid_from, valid_until, last_verified_at, last_used_at, use_count,
                    created_at, updated_at, superseded_by, version
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                    ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                    ?20, ?21,
                    ?22, ?23, ?24, ?25, ?26,
                    ?27, ?28, ?29, ?30
                )
                ON CONFLICT(id) DO UPDATE SET
                    kind = excluded.kind,
                    status = excluded.status,
                    authority = excluded.authority,
                    title = excluded.title,
                    abstract_text = excluded.abstract_text,
                    body_json = excluded.body_json,
                    content_hash = excluded.content_hash,
                    source_trust = excluded.source_trust,
                    confidence = excluded.confidence,
                    verified = excluded.verified,
                    candidate_skill = excluded.candidate_skill,
                    project_id = excluded.project_id,
                    repository_id = excluded.repository_id,
                    branch = excluded.branch,
                    worktree_id = excluded.worktree_id,
                    session_id = excluded.session_id,
                    task_id = excluded.task_id,
                    subtask_id = excluded.subtask_id,
                    conditions_json = excluded.conditions_json,
                    source_uris_json = excluded.source_uris_json,
                    valid_from = excluded.valid_from,
                    valid_until = excluded.valid_until,
                    last_verified_at = excluded.last_verified_at,
                    last_used_at = excluded.last_used_at,
                    use_count = excluded.use_count,
                    updated_at = excluded.updated_at,
                    superseded_by = excluded.superseded_by,
                    version = ?30
                ",
                params![
                    entry.id,
                    entry.kind.as_str(),
                    entry.status.as_str(),
                    entry.authority.as_str(),
                    entry.title,
                    entry.abstract_text,
                    body_json,
                    entry.content_hash,
                    entry.source_trust.as_str(),
                    entry.confidence,
                    entry.verified as i64,
                    entry.candidate_skill as i64,
                    entry.scope.project_id,
                    entry.scope.repository_id,
                    entry.scope.branch,
                    entry.scope.worktree_id,
                    entry.scope.session_id,
                    entry.scope.task_id,
                    entry.scope.subtask_id,
                    conditions_json,
                    source_uris_json,
                    entry.valid_from.map(|v| v as i64),
                    entry.valid_until.map(|v| v as i64),
                    entry.last_verified_at.map(|v| v as i64),
                    entry.last_used_at.map(|v| v as i64),
                    entry.use_count as i64,
                    entry.created_at as i64,
                    entry.updated_at as i64,
                    entry.superseded_by,
                    new_version,
                ],
            )?;
            // FTS sync. The entry is identified by rowid. We delete
            // the old FTS row first (no-op on first insert) and then
            // re-insert. We use `memory_entries.rowid` as the FTS rowid
            // so JOINs in retrieval.rs are simple. Plain DELETE works
            // here; the FTS5 'delete' command is reserved for contexts
            // where DELETE is not available.
            let rowid: i64 = tx.query_row(
                "SELECT rowid FROM memory_entries WHERE id = ?1",
                params![entry.id],
                |r| r.get(0),
            )?;
            if existing_version.is_some() {
                tx.execute(
                    "DELETE FROM memory_entries_fts WHERE rowid = ?1",
                    params![rowid],
                )?;
            }
            tx.execute(
                "INSERT INTO memory_entries_fts(rowid, title, abstract_text) VALUES (?1, ?2, ?3)",
                params![rowid, entry.title, entry.abstract_text],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn get_entry(&self, id: &str) -> MemoryResult<Option<MemoryEntry>> {
        self.with_conn(|c| {
            let row = c
                .query_row(
                    "SELECT * FROM memory_entries WHERE id = ?1",
                    params![id],
                    row_to_entry,
                )
                .optional()?;
            Ok(row)
        })
    }

    pub fn list(&self, filter: &MemoryFilter) -> MemoryResult<Vec<MemoryEntry>> {
        self.with_conn(|c| {
            let mut sql = String::from("SELECT * FROM memory_entries WHERE 1=1");
            let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            if let Some(kind) = filter.kind {
                sql.push_str(" AND kind = ?");
                args.push(Box::new(kind.as_str().to_owned()));
            }
            if let Some(status) = filter.status {
                sql.push_str(" AND status = ?");
                args.push(Box::new(status.as_str().to_owned()));
            }
            if let Some(p) = &filter.project_id {
                sql.push_str(" AND project_id = ?");
                args.push(Box::new(p.clone()));
            }
            if let Some(r) = &filter.repository_id {
                sql.push_str(" AND repository_id = ?");
                args.push(Box::new(r.clone()));
            }
            if !filter.include_history {
                sql.push_str(" AND status NOT IN ('superseded', 'rejected', 'quarantined')");
            }
            sql.push_str(" ORDER BY updated_at DESC");
            if let Some(limit) = filter.limit {
                sql.push_str(&format!(" LIMIT {}", limit.min(1024)));
            }
            let params_refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt
                .query_map(params_refs.as_slice(), row_to_entry)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Lookup by content hash (used for de-duplication at compile time).
    pub fn by_content_hash(&self, hash: &str) -> MemoryResult<Option<MemoryEntry>> {
        self.with_conn(|c| {
            c.query_row(
                "SELECT * FROM memory_entries WHERE content_hash = ?1",
                params![hash],
                row_to_entry,
            )
            .optional()
            .map_err(Into::into)
        })
    }

    /// Mark every entry that references any of the given paths (in their
    /// evidence rows) as `stale`. Returns the number of entries touched.
    pub fn mark_changed_paths(&self, paths: &[String]) -> MemoryResult<usize> {
        if paths.is_empty() {
            return Ok(0);
        }
        self.with_conn(|c| {
            let tx = c.transaction()?;
            let now = now_secs() as i64;
            // We use a temporary table to bind the path list, since
            // rusqlite's slice binding across many params is awkward.
            tx.execute(
                "CREATE TEMP TABLE IF NOT EXISTS tmp_paths(path TEXT PRIMARY KEY)",
                [],
            )?;
            tx.execute("DELETE FROM tmp_paths", [])?;
            {
                let mut stmt = tx.prepare("INSERT OR IGNORE INTO tmp_paths(path) VALUES (?1)")?;
                for p in paths {
                    stmt.execute(params![p])?;
                }
            }
            let n = tx.execute(
                "UPDATE memory_entries
                 SET status = 'stale', updated_at = ?1
                 WHERE id IN (
                     SELECT DISTINCT memory_id FROM memory_evidence
                     WHERE path IN (SELECT path FROM tmp_paths)
                 )",
                params![now],
            )?;
            tx.commit()?;
            Ok(n)
        })
    }

    /// Read the most recent evidence row's `digest` for `id`. Returns
    /// `None` when the entry has no anchored file evidence. Used by
    /// `validity::evaluate` to compare the stored file digest against a
    /// live one without taking a write lock.
    pub fn entry_file_digest(&self, id: &str) -> MemoryResult<Option<String>> {
        self.with_conn(|c| {
            let row: Option<String> = c
                .query_row(
                    "SELECT digest FROM memory_evidence
                     WHERE memory_id = ?1
                     ORDER BY captured_at DESC, id DESC
                     LIMIT 1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(row)
        })
    }

    /// Capture a piece of evidence for a memory. Used at compile time and
    /// for `mark_changed_paths` to know which entries to invalidate.
    pub fn insert_evidence(
        &self,
        memory_id: &str,
        path: &str,
        digest: &str,
        uri: Option<&str>,
    ) -> MemoryResult<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO memory_evidence(memory_id, path, digest, uri, captured_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![memory_id, path, digest, uri, now_secs() as i64],
            )?;
            Ok(())
        })
    }

    /// Update the `last_used_at` and bump `use_count`. Failures are
    /// non-fatal — callers may swallow them.
    pub fn touch_used(&self, id: &str) -> MemoryResult<()> {
        self.with_conn(|c| {
            let now = now_secs() as i64;
            c.execute(
                "UPDATE memory_entries
                 SET last_used_at = ?1,
                     use_count = use_count + 1
                 WHERE id = ?2",
                params![now, id],
            )?;
            Ok(())
        })
    }

    /// Soft-disable an entry (rejected, but kept for history). The user
    /// can re-activate via... nothing, currently. Tombstone is the
    /// permanent option.
    pub fn disable(&self, id: &str) -> MemoryResult<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE memory_entries SET status = 'rejected', updated_at = ?1 WHERE id = ?2",
                params![now_secs() as i64, id],
            )?;
            Ok(())
        })
    }

    /// Permanent delete with tombstone. The outbox worker skips
    /// `event_id`s whose target memory_id appears here.
    pub fn tombstone(&self, id: &str, reason: &str, by: &str) -> MemoryResult<()> {
        self.with_conn(|c| {
            let tx = c.transaction()?;
            let now = now_secs() as i64;
            tx.execute(
                "INSERT OR REPLACE INTO memory_tombstones(memory_id, reason, tombstoned_at, tombstoned_by) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, reason, now, by],
            )?;
            // FTS row first, then the entry. Plain DELETE works here.
            let rowid: Option<i64> = tx
                .query_row(
                    "SELECT rowid FROM memory_entries WHERE id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(r) = rowid {
                tx.execute(
                    "DELETE FROM memory_entries_fts WHERE rowid = ?1",
                    params![r],
                )?;
            }
            tx.execute("DELETE FROM memory_entries WHERE id = ?1", params![id])?;
            tx.commit()?;
            self.metrics.tombstones.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        })
    }

    pub fn is_tombstoned(&self, id: &str) -> MemoryResult<bool> {
        self.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM memory_tombstones WHERE memory_id = ?1",
                params![id],
                |r| r.get(0),
            )?;
            Ok(n > 0)
        })
    }

    // ---- Outbox ------------------------------------------------------------

    pub fn outbox_enqueue(&self, event: &crate::model::SafeMemoryEvent) -> MemoryResult<bool> {
        self.with_conn(|c| {
            // event_id is the dedupe key. If a row exists, this is a
            // replay; we update nothing and return false.
            let now = now_secs() as i64;
            let inserted = c.execute(
                "INSERT OR IGNORE INTO memory_outbox(
                    event_id, session_id, turn_seq, item_seq, phase, kind, source_digest,
                    observed_at, project_path, label, body, extra_json, enqueued_at, status
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 'pending')",
                params![
                    event.event_id,
                    event.session_id,
                    event.turn_seq as i64,
                    event.item_seq as i64,
                    event.phase.as_str(),
                    event.kind.as_str(),
                    event.source_digest,
                    event.observed_at as i64,
                    event.project_path,
                    event.label,
                    event.body,
                    serde_json::to_string(&event.extra)?,
                    now,
                ],
            )?;
            Ok(inserted == 1)
        })
    }

    pub fn outbox_pending(&self, limit: usize) -> MemoryResult<Vec<crate::model::SafeMemoryEvent>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT event_id, session_id, turn_seq, item_seq, phase, kind, source_digest, \
                        observed_at, project_path, label, body, extra_json \
                 FROM memory_outbox \
                 WHERE status IN ('pending', 'failed') AND attempts < 5 \
                 ORDER BY enqueued_at ASC \
                 LIMIT ?1",
            )?;
            let rows = stmt
                .query_map(params![limit as i64], |r| {
                    let phase_raw: String = r.get(4)?;
                    let kind_raw: String = r.get(5)?;
                    let phase = match phase_raw.as_str() {
                        "started" => crate::model::EventPhase::Started,
                        "completed" => crate::model::EventPhase::Completed,
                        "failed" => crate::model::EventPhase::Failed,
                        "blocked" => crate::model::EventPhase::Blocked,
                        _ => crate::model::EventPhase::Completed,
                    };
                    let kind = match kind_raw.as_str() {
                        "reasoning" => crate::model::MemoryEventKind::Reasoning,
                        "search" => crate::model::MemoryEventKind::Search,
                        "file_read" => crate::model::MemoryEventKind::FileRead,
                        "file_change" => crate::model::MemoryEventKind::FileChange,
                        "command_execution" => crate::model::MemoryEventKind::CommandExecution,
                        "model_call" => crate::model::MemoryEventKind::ModelCall,
                        "agent_message" => crate::model::MemoryEventKind::AgentMessage,
                        "compaction" => crate::model::MemoryEventKind::Compaction,
                        "failover" => crate::model::MemoryEventKind::Failover,
                        "plan_step" => crate::model::MemoryEventKind::PlanStep,
                        "permission" => crate::model::MemoryEventKind::Permission,
                        "context_budget" => crate::model::MemoryEventKind::ContextBudget,
                        _ => crate::model::MemoryEventKind::Reasoning,
                    };
                    let extra_json: String = r.get(11)?;
                    let extra: std::collections::BTreeMap<String, String> =
                        serde_json::from_str(&extra_json).unwrap_or_default();
                    Ok(crate::model::SafeMemoryEvent {
                        event_id: r.get(0)?,
                        session_id: r.get(1)?,
                        turn_seq: r.get::<_, i64>(2)? as u32,
                        item_seq: r.get::<_, i64>(3)? as u32,
                        phase,
                        kind,
                        source_digest: r.get(6)?,
                        observed_at: r.get::<_, i64>(7)? as u64,
                        project_path: r.get(8)?,
                        label: r.get(9)?,
                        body: r.get(10)?,
                        extra,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn outbox_mark_compiled(&self, event_id: &str) -> MemoryResult<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE memory_outbox SET status = 'compiled', processed_at = ?1 WHERE event_id = ?2",
                params![now_secs() as i64, event_id],
            )?;
            Ok(())
        })
    }

    pub fn outbox_mark_failed(&self, event_id: &str, err: &str) -> MemoryResult<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE memory_outbox
                 SET status = 'failed',
                     attempts = attempts + 1,
                     last_error = ?1,
                     last_attempted = ?2
                 WHERE event_id = ?3",
                params![err, now_secs() as i64, event_id],
            )?;
            Ok(())
        })
    }

    pub fn outbox_count_pending(&self) -> MemoryResult<u64> {
        self.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM memory_outbox WHERE status IN ('pending', 'failed')",
                [],
                |r| r.get(0),
            )?;
            Ok(n as u64)
        })
    }

    // ---- Usage -------------------------------------------------------------

    pub fn record_usage(&self, usage: &crate::model::MemoryUsage) -> MemoryResult<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT OR REPLACE INTO memory_usage(
                    run_id, memory_id, operation, score, token_cost, elapsed_ms, at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    usage.run_id,
                    usage.memory_id,
                    usage.operation.as_str(),
                    usage.score,
                    usage.token_cost.map(|v| v as i64),
                    usage.elapsed_ms.map(|v| v as i64),
                    usage.at as i64,
                ],
            )?;
            Ok(())
        })
    }

    // ---- Maintenance -------------------------------------------------------

    pub fn row_counts(&self) -> MemoryResult<std::collections::BTreeMap<String, u64>> {
        self.with_conn(|c| {
            let tables = [
                "memory_entries",
                "memory_evidence",
                "memory_links",
                "memory_outbox",
                "memory_usage",
                "memory_tombstones",
            ];
            let mut out = std::collections::BTreeMap::new();
            for t in tables {
                let sql = format!("SELECT COUNT(*) FROM {t}");
                let n: i64 = c.query_row(&sql, [], |r| r.get(0))?;
                out.insert(t.to_owned(), n as u64);
            }
            Ok(out)
        })
    }

    pub fn integrity_check(&self) -> MemoryResult<Vec<String>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("PRAGMA integrity_check")?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// FTS rebuild. Call after a manual schema patch or suspected drift.
    pub fn fts_rebuild(&self) -> MemoryResult<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO memory_entries_fts(memory_entries_fts) VALUES('rebuild')",
                [],
            )?;
            Ok(())
        })
    }
}

fn row_to_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEntry> {
    let kind_raw: String = r.get("kind")?;
    let status_raw: String = r.get("status")?;
    let authority_raw: String = r.get("authority")?;
    let source_trust_raw: String = r.get("source_trust")?;
    let body_json: String = r.get("body_json")?;
    let conditions_json: String = r.get("conditions_json")?;
    let source_uris_json: String = r.get("source_uris_json")?;
    let body: Json = serde_json::from_str(&body_json).unwrap_or(Json::Null);
    let conditions: Vec<String> = serde_json::from_str(&conditions_json).unwrap_or_default();
    let source_uris: Vec<String> = serde_json::from_str(&source_uris_json).unwrap_or_default();
    let kind = MemoryKind::parse(&kind_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::<dyn std::error::Error + Send + Sync>::from(format!("bad kind: {kind_raw}")),
        )
    })?;
    let status = MemoryStatus::parse(&status_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::<dyn std::error::Error + Send + Sync>::from(format!("bad status: {status_raw}")),
        )
    })?;
    let authority = Authority::parse(&authority_raw).unwrap_or(Authority::Historical);
    let source_trust = SourceTrust::parse(&source_trust_raw).unwrap_or(SourceTrust::Untrusted);
    let scope = MemoryScope {
        project_id: r.get("project_id")?,
        repository_id: r.get("repository_id")?,
        branch: r.get("branch")?,
        worktree_id: r.get("worktree_id")?,
        session_id: r.get("session_id")?,
        task_id: r.get("task_id")?,
        subtask_id: r.get("subtask_id")?,
    };
    let valid_from = r.get::<_, Option<i64>>("valid_from")?.map(|v| v as u64);
    let valid_until = r.get::<_, Option<i64>>("valid_until")?.map(|v| v as u64);
    let last_verified_at = r
        .get::<_, Option<i64>>("last_verified_at")?
        .map(|v| v as u64);
    let last_used_at = r.get::<_, Option<i64>>("last_used_at")?.map(|v| v as u64);
    let created_at = r.get::<_, i64>("created_at")? as u64;
    let updated_at = r.get::<_, i64>("updated_at")? as u64;
    let use_count = r.get::<_, i64>("use_count")? as u32;
    let version = r.get::<_, i64>("version")? as u32;
    let verified = r.get::<_, i64>("verified")? != 0;
    let candidate_skill = r.get::<_, i64>("candidate_skill")? != 0;
    Ok(MemoryEntry {
        id: r.get("id")?,
        kind,
        status,
        authority,
        scope,
        title: r.get("title")?,
        abstract_text: r.get("abstract_text")?,
        body,
        content_hash: r.get("content_hash")?,
        source_trust,
        confidence: r.get("confidence")?,
        verified,
        conditions,
        source_uris,
        valid_from,
        valid_until,
        last_verified_at,
        last_used_at,
        use_count,
        created_at,
        updated_at,
        superseded_by: r.get("superseded_by")?,
        version,
        candidate_skill,
    })
}

// Re-export ValidityLabel so other modules can import it from `store`
// without going to model. (Doc-comment only; nothing to do here.)
#[allow(unused_imports)]
use ValidityLabel as _UnusedValidityReexport;

// One straightforward unit test: open, migrate, list.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Authority, MemoryEntry, MemoryKind, MemoryScope, MemoryStatus, SourceTrust,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_db() -> Store {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("kodo-memory-test-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.sqlite");
        // remove leftover WAL files from any prior run
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        Store::open(path, Arc::new(Metrics::new())).unwrap()
    }

    fn sample_entry(id: &str) -> MemoryEntry {
        let now = now_secs();
        MemoryEntry {
            id: id.to_owned(),
            kind: MemoryKind::Episodic,
            status: MemoryStatus::Active,
            authority: Authority::Historical,
            scope: MemoryScope::default(),
            title: "fix".to_owned(),
            abstract_text: "fix".to_owned(),
            body: serde_json::json!({}),
            content_hash: format!("h-{id}"),
            source_trust: SourceTrust::Untrusted,
            confidence: 0.5,
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
        }
    }

    #[test]
    fn opens_and_migrates() {
        let store = tmp_db();
        let counts = store.row_counts().unwrap();
        assert!(counts.get("memory_entries").copied().unwrap() == 0);
    }

    #[test]
    fn upsert_then_get_round_trip() {
        let store = tmp_db();
        let now = now_secs();
        let entry = MemoryEntry {
            id: "test-1".to_owned(),
            kind: MemoryKind::Episodic,
            status: MemoryStatus::Active,
            authority: Authority::Historical,
            scope: MemoryScope {
                project_id: Some("/tmp/proj".to_owned()),
                ..Default::default()
            },
            title: "k2k-rust 未知表路由".to_owned(),
            abstract_text: "k2k 路由在 catalog 缺失时落到 default topic".to_owned(),
            body: serde_json::json!({ "steps": ["a", "b"] }),
            content_hash: "deadbeef".to_owned(),
            source_trust: SourceTrust::ToolVerified,
            confidence: 0.7,
            verified: true,
            conditions: vec!["k2k-rust only".to_owned()],
            source_uris: vec!["kodo://sessions/abc".to_owned()],
            valid_from: None,
            valid_until: None,
            last_verified_at: Some(now),
            last_used_at: None,
            use_count: 0,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            version: 1,
            candidate_skill: false,
        };
        store.upsert_entry(&entry).unwrap();
        let got = store.get_entry("test-1").unwrap().unwrap();
        assert_eq!(got.id, "test-1");
        assert_eq!(got.kind, MemoryKind::Episodic);
        assert_eq!(got.title, entry.title);
        assert_eq!(got.scope.project_id.as_deref(), Some("/tmp/proj"));
    }

    #[test]
    fn mark_changed_paths_marks_stale() {
        let store = tmp_db();
        let entry = sample_entry("ep-1");
        store.upsert_entry(&entry).unwrap();
        store
            .insert_evidence("ep-1", "src/main.rs", "digest1", None)
            .unwrap();
        let n = store
            .mark_changed_paths(&["src/main.rs".to_owned()])
            .unwrap();
        assert_eq!(n, 1);
        let got = store.get_entry("ep-1").unwrap().unwrap();
        assert_eq!(got.status, MemoryStatus::Stale);
    }

    #[test]
    fn outbox_enqueue_is_idempotent() {
        let store = tmp_db();
        let evt = crate::model::SafeMemoryEvent {
            event_id: "evt-1".to_owned(),
            session_id: "s1".to_owned(),
            turn_seq: 0,
            item_seq: 0,
            phase: crate::model::EventPhase::Completed,
            kind: crate::model::MemoryEventKind::FileRead,
            source_digest: "d".to_owned(),
            observed_at: now_secs(),
            project_path: Some("src/lib.rs".to_owned()),
            label: "Read src/lib.rs".to_owned(),
            body: None,
            extra: Default::default(),
        };
        let first = store.outbox_enqueue(&evt).unwrap();
        let second = store.outbox_enqueue(&evt).unwrap();
        assert!(first);
        assert!(!second);
        let n = store.outbox_count_pending().unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn tombstone_removes_entry_and_blocks_replay() {
        let store = tmp_db();
        let entry = sample_entry("die-1");
        store.upsert_entry(&entry).unwrap();
        store.tombstone("die-1", "user delete", "user").unwrap();
        assert!(store.get_entry("die-1").unwrap().is_none());
        assert!(store.is_tombstoned("die-1").unwrap());
    }
}
