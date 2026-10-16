//! Scheduler: persistent store, dispatcher loop, resource admission, job
//! execution contract.
//!
//! The scheduler is the runtime that turns a `ScheduledTask` into actual runs.
//! It is deliberately split into:
//!
//! - [`SchedulerStore`] — SQLite-backed fact store. Everything that needs to
//!   survive a restart lives here. The store never executes side effects; it
//!   only writes transactional records.
//! - [`Scheduler`] — in-memory facade combining the store, the clock, the
//!   resource guard and the job registry. Its [`Scheduler::tick`] method
//!   returns the next batch of runs to dispatch; callers actually run them.
//! - [`JobRegistry`] — maps `job_type` to a [`Job`] handler. Jobs run inside
//!   the scheduler process on a worker thread, with cancellation honored.
//! - [`ResourceGuard`] — shared admission: at most one in-flight run per
//!   declared resource key (`agent:<path>`, `job:<resource>`, …).
//!
//! Agent runs are *not* executed by this crate — they need the desktop's
//! `run::start` driver and its approval rendezvous. The scheduler hands the
//! Tauri shell a [`DispatchedRun`] and the shell drives the run from there.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use kodo_core::scheduler::{
    next_occurrences, validate, Clock, Lifecycle, MisfirePolicy, Outcome, OverlapPolicy, Schedule,
    ScheduledTask, TaskKind, TaskRun, TaskState, TriggerSource, UnixSeconds, PREVIEW_COUNT,
};

/// Stable error code strings the GUI maps to inline messages. Anything
/// outside this list is treated as "unknown failure" so the frontend never
/// tries to parse Rust error strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerError {
    /// Underlying SQLite call failed. The error message is redacted before
    /// reaching the GUI; the structured code is what the frontend keys on.
    StorageUnavailable,
    /// The caller supplied an invalid time rule (see
    /// [`kodo_core::scheduler::ScheduleError`] for the variants).
    InvalidSchedule,
    /// Expected revision did not match. The GUI re-fetches and retries.
    RevisionConflict,
    /// The referenced task does not exist or was archived.
    UnknownTask,
    /// Shared-resource admission refused the new run (another run already
    /// owns every declared key, or concurrency limits are exceeded).
    ResourceBusy,
    /// Caller asked for an unknown job_type or supplied params that did not
    /// validate.
    UnknownJob,
    /// Caller asked for a run that is already finished.
    AlreadyFinished,
    /// Caller asked to cancel a run that has no live execution.
    NoLiveRun,
}

impl std::fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SchedulerError::StorageUnavailable => "storage_unavailable",
            SchedulerError::InvalidSchedule => "invalid_schedule",
            SchedulerError::RevisionConflict => "revision_conflict",
            SchedulerError::UnknownTask => "unknown_task",
            SchedulerError::ResourceBusy => "resource_busy",
            SchedulerError::UnknownJob => "unknown_job",
            SchedulerError::AlreadyFinished => "already_finished",
            SchedulerError::NoLiveRun => "no_live_run",
        })
    }
}

impl std::error::Error for SchedulerError {}

impl From<rusqlite::Error> for SchedulerError {
    fn from(error: rusqlite::Error) -> Self {
        eprintln!("kodo scheduler: storage error: {error}");
        SchedulerError::StorageUnavailable
    }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// SQLite-backed scheduler store. Each method is a short transaction;
/// long-running work (job execution, Agent dispatch) lives outside the
/// store. The store's single source of truth for "has this scheduled
/// moment already been queued?" is the UNIQUE constraint on `occurrence_key`.
pub struct SchedulerStore {
    conn: Mutex<Connection>,
}

impl SchedulerStore {
    /// Open or create the store at `<state_dir>/scheduler/v1/scheduler.sqlite`.
    pub fn open(state_dir: &Path) -> Result<Self, SchedulerError> {
        let dir = state_dir.join("scheduler").join("v1");
        std::fs::create_dir_all(&dir).map_err(|error| {
            eprintln!("kodo scheduler: cannot create {}: {error}", dir.display());
            SchedulerError::StorageUnavailable
        })?;
        let path = dir.join("scheduler.sqlite");
        let conn = Connection::open(&path).map_err(|error| {
            eprintln!("kodo scheduler: cannot open {}: {error}", path.display());
            SchedulerError::StorageUnavailable
        })?;
        // WAL keeps reads from blocking writers and survives crashes with a
        // clean checkpoint. busy_timeout = 5s matches kodo-memory.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| {
                eprintln!("kodo scheduler: WAL pragma failed: {error}");
                SchedulerError::StorageUnavailable
            })?;
        // rusqlite's `pragma_update` only accepts `ToSql`; pass the
        // millisecond value directly to keep the call from instantiating a
        // `Duration` we then need to convert again.
        conn.pragma_update(None, "busy_timeout", 5_000_i64)
            .map_err(|error| {
                eprintln!("kodo scheduler: busy_timeout pragma failed: {error}");
                SchedulerError::StorageUnavailable
            })?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), SchedulerError> {
        let conn = self.conn.lock();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS scheduler_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS scheduled_tasks (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                state TEXT NOT NULL,
                revision INTEGER NOT NULL,
                schedule_generation INTEGER NOT NULL,
                schedule_json TEXT NOT NULL,
                task_json TEXT NOT NULL,
                timezone TEXT NOT NULL,
                misfire_policy TEXT NOT NULL,
                grace_seconds INTEGER NOT NULL,
                overlap_policy TEXT NOT NULL,
                timeout_seconds INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                next_fire_at INTEGER,
                archived INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS task_runs (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                occurrence_key TEXT NOT NULL UNIQUE,
                trigger_source TEXT NOT NULL,
                schedule_generation INTEGER NOT NULL,
                scheduled_for INTEGER NOT NULL,
                queued_at INTEGER NOT NULL,
                started_at INTEGER,
                finished_at INTEGER,
                config_revision INTEGER NOT NULL,
                config_snapshot_json TEXT NOT NULL,
                lifecycle TEXT NOT NULL,
                outcome TEXT,
                delivery TEXT,
                verification TEXT,
                session_id TEXT,
                report_ref TEXT,
                reason_code TEXT,
                error_summary TEXT,
                skipped_count INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (task_id) REFERENCES scheduled_tasks(id)
            );
            CREATE INDEX IF NOT EXISTS idx_task_runs_task_started
                ON task_runs(task_id, started_at);
            CREATE INDEX IF NOT EXISTS idx_task_runs_lifecycle
                ON task_runs(lifecycle);
            CREATE TABLE IF NOT EXISTS task_attempts (
                run_id TEXT NOT NULL,
                attempt_no INTEGER NOT NULL,
                started_at INTEGER,
                finished_at INTEGER,
                outcome TEXT,
                error_class TEXT,
                error_summary TEXT,
                PRIMARY KEY (run_id, attempt_no),
                FOREIGN KEY (run_id) REFERENCES task_runs(id)
            );
            "#,
        )?;
        let version: i64 = conn
            .query_row(
                "SELECT value FROM scheduler_meta WHERE key = 'schema_version'",
                [],
                |row| {
                    let raw: String = row.get(0)?;
                    Ok(raw.parse::<i64>().unwrap_or(0))
                },
            )
            .optional()?
            .unwrap_or(0);
        if version < 1 {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "INSERT OR REPLACE INTO scheduler_meta(key, value) VALUES('schema_version', '1')",
                [],
            )?;
            tx.commit()?;
        }
        Ok(())
    }

    // -- task CRUD ----------------------------------------------------------

    pub fn upsert_task(&self, task: &ScheduledTask) -> Result<(), SchedulerError> {
        let schedule_json = serde_json::to_string(&task.schedule).map_err(|error| {
            eprintln!("kodo scheduler: schedule_json encode failed: {error}");
            SchedulerError::StorageUnavailable
        })?;
        let task_json = serde_json::to_string(&task.task).map_err(|error| {
            eprintln!("kodo scheduler: task_json encode failed: {error}");
            SchedulerError::StorageUnavailable
        })?;
        let conn = self.conn.lock();
        conn.execute(
            r#"INSERT INTO scheduled_tasks(
                id, title, state, revision, schedule_generation,
                schedule_json, task_json, timezone, misfire_policy,
                grace_seconds, overlap_policy, timeout_seconds,
                created_at, updated_at, next_fire_at, archived
            ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
            ON CONFLICT(id) DO UPDATE SET
                title=excluded.title,
                state=excluded.state,
                revision=excluded.revision,
                schedule_generation=excluded.schedule_generation,
                schedule_json=excluded.schedule_json,
                task_json=excluded.task_json,
                timezone=excluded.timezone,
                misfire_policy=excluded.misfire_policy,
                grace_seconds=excluded.grace_seconds,
                overlap_policy=excluded.overlap_policy,
                timeout_seconds=excluded.timeout_seconds,
                updated_at=excluded.updated_at,
                next_fire_at=excluded.next_fire_at,
                archived=excluded.archived"#,
            params![
                task.id,
                task.title,
                task_state_str(task.state),
                task.revision as i64,
                task.schedule_generation as i64,
                schedule_json,
                task_json,
                task.timezone,
                misfire_str(task.misfire_policy),
                task.grace_seconds as i64,
                overlap_str(task.overlap_policy),
                task.timeout_seconds.map(|v| v as i64),
                task.created_at,
                task.updated_at,
                task.next_fire_at,
                task.archived as i64,
            ],
        )?;
        Ok(())
    }

    pub fn get_task(&self, id: &str) -> Result<Option<ScheduledTask>, SchedulerError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, title, state, revision, schedule_generation, schedule_json, \
             task_json, timezone, misfire_policy, grace_seconds, overlap_policy, \
             timeout_seconds, created_at, updated_at, next_fire_at, archived \
             FROM scheduled_tasks WHERE id = ?",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(read_task_row(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_tasks(&self, include_archived: bool) -> Result<Vec<ScheduledTask>, SchedulerError> {
        let conn = self.conn.lock();
        let sql = if include_archived {
            "SELECT id, title, state, revision, schedule_generation, schedule_json, \
             task_json, timezone, misfire_policy, grace_seconds, overlap_policy, \
             timeout_seconds, created_at, updated_at, next_fire_at, archived \
             FROM scheduled_tasks ORDER BY updated_at DESC"
        } else {
            "SELECT id, title, state, revision, schedule_generation, schedule_json, \
             task_json, timezone, misfire_policy, grace_seconds, overlap_policy, \
             timeout_seconds, created_at, updated_at, next_fire_at, archived \
             FROM scheduled_tasks WHERE archived = 0 ORDER BY updated_at DESC"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], read_task_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Atomic "schedule rule changed → bump generation → push cursor forward"
    /// pair. The cursor is the next-fire-at we display in the listing; the
    /// dispatcher ignores it and recomputes from `(after, generation)` so an
    /// edit can never replay already-consumed moments.
    pub fn set_next_fire_at(
        &self,
        task_id: &str,
        next: Option<UnixSeconds>,
    ) -> Result<(), SchedulerError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE scheduled_tasks SET next_fire_at = ? WHERE id = ?",
            params![next, task_id],
        )?;
        Ok(())
    }

    pub fn set_state(&self, task_id: &str, state: TaskState) -> Result<(), SchedulerError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE scheduled_tasks SET state = ?, updated_at = ? WHERE id = ?",
            params![task_state_str(state), unix_now(), task_id],
        )?;
        Ok(())
    }

    pub fn archive_task(&self, task_id: &str) -> Result<(), SchedulerError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE scheduled_tasks SET archived = 1, state = ?, updated_at = ? WHERE id = ?",
            params![task_state_str(TaskState::Archived), unix_now(), task_id],
        )?;
        Ok(())
    }

    // -- run lifecycle -------------------------------------------------------

    /// Insert a queued run. Returns `false` if the same `occurrence_key`
    /// already exists — the dispatcher treats that as "another worker beat
    /// us to it" and moves on.
    pub fn insert_queued_run(&self, run: &TaskRun) -> Result<bool, SchedulerError> {
        let conn = self.conn.lock();
        let snapshot = serde_json::to_string(&run.config_snapshot).map_err(|error| {
            eprintln!("kodo scheduler: snapshot encode failed: {error}");
            SchedulerError::StorageUnavailable
        })?;
        let result = conn.execute(
            r#"INSERT OR IGNORE INTO task_runs(
                id, task_id, occurrence_key, trigger_source,
                schedule_generation, scheduled_for, queued_at,
                config_revision, config_snapshot_json,
                lifecycle, skipped_count
            ) VALUES (?,?,?,?,?,?,?,?,?,?,?)"#,
            params![
                run.id,
                run.task_id,
                run.occurrence_key,
                trigger_source_str(run.trigger_source),
                run.schedule_generation as i64,
                run.scheduled_for,
                run.queued_at,
                run.config_revision as i64,
                snapshot,
                lifecycle_str(run.lifecycle),
                run.skipped_count as i64,
            ],
        )?;
        Ok(result == 1)
    }

    /// Mark a queued run as `Running`. Returns the prior lifecycle so the
    /// caller can detect stale state.
    pub fn mark_running(
        &self,
        run_id: &str,
        started_at: UnixSeconds,
    ) -> Result<Lifecycle, SchedulerError> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let prior = read_lifecycle(&tx, run_id)?;
        match prior {
            Lifecycle::Queued => {}
            other => {
                return Err(runtime_error(format!(
                    "mark_running refused: run {run_id} is in lifecycle {other:?}"
                )));
            }
        }
        tx.execute(
            "UPDATE task_runs SET lifecycle = ?, started_at = ? WHERE id = ?",
            params![lifecycle_str(Lifecycle::Running), started_at, run_id],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO task_attempts(run_id, attempt_no, started_at) \
             VALUES(?, COALESCE((SELECT MAX(attempt_no)+1 FROM task_attempts WHERE run_id = ?), 1), ?)",
            params![run_id, run_id, started_at],
        )?;
        tx.commit()?;
        Ok(Lifecycle::Queued)
    }

    /// Stamp the final outcome on a run. Records `reason_code` and
    /// `error_summary` (already redacted by the caller) and closes the
    /// latest attempt row.
    pub fn mark_finished(
        &self,
        run_id: &str,
        finished_at: UnixSeconds,
        outcome: Outcome,
        delivery: Option<&str>,
        verification: Option<&str>,
        session_id: Option<&str>,
        report_ref: Option<&str>,
        reason_code: Option<&str>,
        error_summary: Option<&str>,
    ) -> Result<(), SchedulerError> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE task_runs SET lifecycle = ?, finished_at = ?, outcome = ?, \
             delivery = COALESCE(?, delivery), verification = COALESCE(?, verification), \
             session_id = COALESCE(?, session_id), report_ref = COALESCE(?, report_ref), \
             reason_code = COALESCE(?, reason_code), error_summary = COALESCE(?, error_summary) \
             WHERE id = ?",
            params![
                lifecycle_str(Lifecycle::Finished),
                finished_at,
                outcome_str(outcome),
                delivery,
                verification,
                session_id,
                report_ref,
                reason_code,
                error_summary,
                run_id,
            ],
        )?;
        tx.execute(
            "UPDATE task_attempts SET finished_at = ?, outcome = ?, \
             error_class = COALESCE(?, error_class), error_summary = COALESCE(?, error_summary) \
             WHERE run_id = ? AND finished_at IS NULL",
            params![
                finished_at,
                outcome_str(outcome),
                reason_code,
                error_summary,
                run_id,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Stamp a cancel request on a `Running` run. Returns `false` when the
    /// run is already finished (the GUI treats that as success).
    pub fn request_cancel(&self, run_id: &str) -> Result<bool, SchedulerError> {
        let conn = self.conn.lock();
        let updated = conn.execute(
            "UPDATE task_runs SET lifecycle = ? \
             WHERE id = ? AND lifecycle IN (?, ?)",
            params![
                lifecycle_str(Lifecycle::Cancelling),
                run_id,
                lifecycle_str(Lifecycle::Queued),
                lifecycle_str(Lifecycle::Running),
            ],
        )?;
        Ok(updated == 1)
    }

    pub fn get_run(&self, run_id: &str) -> Result<Option<TaskRun>, SchedulerError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, task_id, occurrence_key, trigger_source, schedule_generation, \
             scheduled_for, queued_at, started_at, finished_at, config_revision, \
             config_snapshot_json, lifecycle, outcome, delivery, verification, \
             session_id, report_ref, reason_code, error_summary, skipped_count \
             FROM task_runs WHERE id = ?",
        )?;
        let mut rows = stmt.query(params![run_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(read_run_row(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_runs(&self, task_id: &str, limit: usize) -> Result<Vec<TaskRun>, SchedulerError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, task_id, occurrence_key, trigger_source, schedule_generation, \
             scheduled_for, queued_at, started_at, finished_at, config_revision, \
             config_snapshot_json, lifecycle, outcome, delivery, verification, \
             session_id, report_ref, reason_code, error_summary, skipped_count \
             FROM task_runs WHERE task_id = ? ORDER BY queued_at DESC LIMIT ?",
        )?;
        let rows = stmt.query_map(params![task_id, limit as i64], read_run_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn list_inflight(&self) -> Result<Vec<TaskRun>, SchedulerError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, task_id, occurrence_key, trigger_source, schedule_generation, \
             scheduled_for, queued_at, started_at, finished_at, config_revision, \
             config_snapshot_json, lifecycle, outcome, delivery, verification, \
             session_id, report_ref, reason_code, error_summary, skipped_count \
             FROM task_runs WHERE lifecycle IN (?, ?, ?) ORDER BY queued_at ASC",
        )?;
        let rows = stmt.query_map(
            params![
                lifecycle_str(Lifecycle::Queued),
                lifecycle_str(Lifecycle::Running),
                lifecycle_str(Lifecycle::Cancelling),
            ],
            read_run_row,
        )?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Record a `Skipped` row for an occurrence that the dispatcher decided
    /// not to run. Keeps the audit trail even when no `task_runs` row exists.
    /// The `config_snapshot_json` column is still a NOT-NULL `TaskKind`
    /// because the schema is uniform; we insert a real (empty) `TaskKind`
    /// so the row deserializes without a special case in the read path.
    pub fn record_skipped(
        &self,
        task_id: &str,
        occurrence_key: &str,
        scheduled_for: UnixSeconds,
        reason_code: &str,
        skipped_count: u32,
    ) -> Result<(), SchedulerError> {
        let conn = self.conn.lock();
        let now = unix_now();
        let id = format!("skip-{occurrence_key}");
        // A real, deserializable placeholder. `Job` is the right choice
        // for a scheduler-internal "no work" row because the column type
        // has to deserialize as `TaskKind`; the run-history view treats
        // these as opaque.
        let placeholder = serde_json::to_string(&TaskKind::Job {
            job_type: "_skipped".into(),
            params: serde_json::json!({}),
            resources: Vec::new(),
        })
        .map_err(|error| {
            eprintln!("kodo scheduler: placeholder encode failed: {error}");
            SchedulerError::StorageUnavailable
        })?;
        conn.execute(
            "INSERT OR IGNORE INTO task_runs(\
             id, task_id, occurrence_key, trigger_source, schedule_generation, \
             scheduled_for, queued_at, finished_at, config_revision, \
             config_snapshot_json, lifecycle, outcome, reason_code, skipped_count) \
             VALUES(?, ?, ?, ?, 0, ?, ?, ?, 0, ?, 'finished', 'skipped', ?, ?)",
            params![
                id,
                task_id,
                occurrence_key,
                trigger_source_str(TriggerSource::Scheduled),
                scheduled_for,
                now,
                now,
                placeholder,
                reason_code,
                skipped_count as i64,
            ],
        )?;
        Ok(())
    }
}

fn read_task_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduledTask> {
    let state: String = row.get(2)?;
    let schedule_json: String = row.get(5)?;
    let task_json: String = row.get(6)?;
    let misfire: String = row.get(8)?;
    let overlap: String = row.get(10)?;
    Ok(ScheduledTask {
        id: row.get(0)?,
        title: row.get(1)?,
        state: parse_task_state(&state).unwrap_or(TaskState::Enabled),
        revision: row.get::<_, i64>(3)? as u64,
        schedule_generation: row.get::<_, i64>(4)? as u64,
        schedule: serde_json::from_str(&schedule_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        task: serde_json::from_str(&task_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        timezone: row.get(7)?,
        misfire_policy: parse_misfire(&misfire).unwrap_or(MisfirePolicy::LatestOnce),
        grace_seconds: row.get::<_, i64>(9)? as u64,
        overlap_policy: parse_overlap(&overlap).unwrap_or(OverlapPolicy::Skip),
        timeout_seconds: row.get::<_, Option<i64>>(11)?.map(|v| v as u64),
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        next_fire_at: row.get(14)?,
        archived: row.get::<_, i64>(15)? != 0,
    })
}

fn read_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRun> {
    let lifecycle: String = row.get(11)?;
    let outcome: Option<String> = row.get(12)?;
    let snapshot: String = row.get(10)?;
    Ok(TaskRun {
        id: row.get(0)?,
        task_id: row.get(1)?,
        occurrence_key: row.get(2)?,
        trigger_source: parse_trigger_source(&row.get::<_, String>(3)?)
            .unwrap_or(TriggerSource::Scheduled),
        schedule_generation: row.get::<_, i64>(4)? as u64,
        scheduled_for: row.get(5)?,
        queued_at: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
        config_revision: row.get::<_, i64>(9)? as u64,
        config_snapshot: serde_json::from_str(&snapshot).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                10,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        lifecycle: parse_lifecycle(&lifecycle).unwrap_or(Lifecycle::Queued),
        outcome: outcome.as_deref().and_then(parse_outcome),
        delivery: row.get(13)?,
        verification: row.get(14)?,
        session_id: row.get(15)?,
        report_ref: row.get(16)?,
        reason_code: row.get(17)?,
        error_summary: row.get(18)?,
        skipped_count: row.get::<_, i64>(19)? as u32,
        attempts: Vec::new(),
    })
}

fn read_lifecycle(tx: &Transaction<'_>, run_id: &str) -> Result<Lifecycle, SchedulerError> {
    let raw: Option<String> = tx
        .query_row(
            "SELECT lifecycle FROM task_runs WHERE id = ?",
            params![run_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(raw) = raw else {
        return Err(SchedulerError::UnknownTask);
    };
    Ok(parse_lifecycle(&raw).unwrap_or(Lifecycle::Queued))
}

fn task_state_str(state: TaskState) -> &'static str {
    match state {
        TaskState::Enabled => "enabled",
        TaskState::Paused => "paused",
        TaskState::Finished => "finished",
        TaskState::Archived => "archived",
    }
}
fn parse_task_state(raw: &str) -> Option<TaskState> {
    match raw {
        "enabled" => Some(TaskState::Enabled),
        "paused" => Some(TaskState::Paused),
        "finished" => Some(TaskState::Finished),
        "archived" => Some(TaskState::Archived),
        _ => None,
    }
}
fn misfire_str(policy: MisfirePolicy) -> &'static str {
    match policy {
        MisfirePolicy::LatestOnce => "latestOnce",
        MisfirePolicy::Skip => "skip",
        MisfirePolicy::Coalesce => "coalesce",
    }
}
fn parse_misfire(raw: &str) -> Option<MisfirePolicy> {
    match raw {
        "latestOnce" => Some(MisfirePolicy::LatestOnce),
        "skip" => Some(MisfirePolicy::Skip),
        "coalesce" => Some(MisfirePolicy::Coalesce),
        _ => None,
    }
}
fn overlap_str(policy: OverlapPolicy) -> &'static str {
    match policy {
        OverlapPolicy::Skip => "skip",
        OverlapPolicy::QueueLatest => "queueLatest",
        OverlapPolicy::Allow => "allow",
    }
}
fn parse_overlap(raw: &str) -> Option<OverlapPolicy> {
    match raw {
        "skip" => Some(OverlapPolicy::Skip),
        "queueLatest" => Some(OverlapPolicy::QueueLatest),
        "allow" => Some(OverlapPolicy::Allow),
        _ => None,
    }
}
fn lifecycle_str(l: Lifecycle) -> &'static str {
    match l {
        Lifecycle::Queued => "queued",
        Lifecycle::Running => "running",
        Lifecycle::Cancelling => "cancelling",
        Lifecycle::Finished => "finished",
    }
}
fn parse_lifecycle(raw: &str) -> Option<Lifecycle> {
    match raw {
        "queued" => Some(Lifecycle::Queued),
        "running" => Some(Lifecycle::Running),
        "cancelling" => Some(Lifecycle::Cancelling),
        "finished" => Some(Lifecycle::Finished),
        _ => None,
    }
}
fn outcome_str(o: Outcome) -> &'static str {
    match o {
        Outcome::Succeeded => "succeeded",
        Outcome::Partial => "partial",
        Outcome::Blocked => "blocked",
        Outcome::Failed => "failed",
        Outcome::Cancelled => "cancelled",
        Outcome::TimedOut => "timedOut",
        Outcome::Interrupted => "interrupted",
        Outcome::Skipped => "skipped",
    }
}
fn parse_outcome(raw: &str) -> Option<Outcome> {
    match raw {
        "succeeded" => Some(Outcome::Succeeded),
        "partial" => Some(Outcome::Partial),
        "blocked" => Some(Outcome::Blocked),
        "failed" => Some(Outcome::Failed),
        "cancelled" => Some(Outcome::Cancelled),
        "timedOut" => Some(Outcome::TimedOut),
        "interrupted" => Some(Outcome::Interrupted),
        "skipped" => Some(Outcome::Skipped),
        _ => None,
    }
}
fn trigger_source_str(s: TriggerSource) -> &'static str {
    match s {
        TriggerSource::Scheduled => "scheduled",
        TriggerSource::Manual => "manual",
    }
}
fn parse_trigger_source(raw: &str) -> Option<TriggerSource> {
    match raw {
        "scheduled" => Some(TriggerSource::Scheduled),
        "manual" => Some(TriggerSource::Manual),
        _ => None,
    }
}
fn unix_now() -> UnixSeconds {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
fn runtime_error(message: String) -> SchedulerError {
    eprintln!("kodo scheduler: {message}");
    SchedulerError::StorageUnavailable
}

// ---------------------------------------------------------------------------
// Resource admission
// ---------------------------------------------------------------------------

/// Shared admission: a run is allowed when every declared resource key is
/// free and global concurrency is below the limit. The guard is in-memory;
/// each kodo process has its own. Multiple kodo processes sharing the same
/// store is deliberately out of scope for the first version (the docs §7
/// require an OS-level dispatch lock; we leave the dispatch lock to the
/// caller and only guarantee admission within one process).
#[derive(Debug, Default)]
pub struct ResourceGuard {
    inner: Mutex<ResourceInner>,
}

#[derive(Debug, Default)]
struct ResourceInner {
    keys: HashMap<String, String>,
    total: usize,
}

#[derive(Debug, Clone)]
pub struct AdmissionLimits {
    pub total: usize,
    pub per_agent: usize,
}

impl Default for AdmissionLimits {
    fn default() -> Self {
        Self {
            total: 2,
            per_agent: 1,
        }
    }
}

impl ResourceGuard {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ResourceInner::default()),
        }
    }

    /// Returns `Ok(handle)` when the run is admitted. Caller must release via
    /// [`Self::release`] regardless of outcome — leaking a handle stalls
    /// later dispatches forever.
    pub fn try_admit(
        &self,
        run_id: &str,
        resource_keys: &[String],
        limits: &AdmissionLimits,
    ) -> Result<ResourceHandle, SchedulerError> {
        let mut inner = self.inner.lock();
        // Per-agent: at most one. We treat each agent resource_key as its
        // own bucket — `agent:/p` vs `agent:/q` are different projects.
        if resource_keys.iter().any(|k| k.starts_with("agent:"))
            && resource_keys
                .iter()
                .filter(|k| k.starts_with("agent:"))
                .filter(|k| inner.keys.contains_key(*k))
                .count()
                >= limits.per_agent
        {
            return Err(SchedulerError::ResourceBusy);
        }
        for key in resource_keys {
            if inner.keys.contains_key(key) {
                return Err(SchedulerError::ResourceBusy);
            }
        }
        if inner.total >= limits.total {
            return Err(SchedulerError::ResourceBusy);
        }
        for key in resource_keys {
            inner.keys.insert(key.clone(), run_id.to_owned());
        }
        inner.total += 1;
        Ok(ResourceHandle {
            run_id: run_id.to_owned(),
            keys: resource_keys.to_vec(),
            guard: self as *const _ as *const () as usize,
        })
    }

    fn release_handle(&self, handle: ResourceHandle) {
        let mut inner = self.inner.lock();
        for key in &handle.keys {
            if inner.keys.remove(key).is_some() {
                inner.total = inner.total.saturating_sub(1);
            }
        }
    }
}

pub struct ResourceHandle {
    run_id: String,
    keys: Vec<String>,
    // Pointer-identity marker: guarantees `release` is called on the same
    // guard that issued the handle. Cheaper than `Arc`.
    guard: usize,
}

impl ResourceHandle {
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
}

// We can't return a handle carrying `&self` because it would borrow forever.
// The pointer-identity trick lets the caller pass the handle back without
// keeping a reference; the guard validates identity before releasing.
//
// Safety: `ResourceGuard::release` receives the handle back and matches on
// the pointer. As long as the guard outlives the handle, this is sound.
unsafe impl Send for ResourceHandle {}
unsafe impl Sync for ResourceHandle {}

pub fn release(handle: ResourceHandle, guard: &ResourceGuard) {
    let handle_guard = handle.guard as *const ResourceGuard;
    if std::ptr::eq(handle_guard, guard) {
        guard.release_handle(handle);
    } else {
        eprintln!("kodo scheduler: resource handle leaked (wrong guard)");
    }
}

// ---------------------------------------------------------------------------
// Job contract
// ---------------------------------------------------------------------------

/// Outcome of one job invocation. The runtime treats `succeeded` /
/// `partial` as success-ish and `failed` as failure; `blocked` short-
/// circuits automatic retry.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobReport {
    pub outcome: JobOutcome,
    /// How many items the job handled. Job-defined semantics.
    pub processed: u32,
    pub succeeded: u32,
    pub failed: u32,
    /// Stable reason code for failures (e.g. `io_error`, `parse_error`,
    /// `blocked_needs_user`); absent when `outcome` is `Succeeded`.
    pub reason_code: Option<String>,
    /// Free-form short summary (already redacted by the handler).
    pub summary: Option<String>,
    /// Reference to a richer report (path or URI). Job-defined.
    pub report_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JobOutcome {
    Succeeded,
    Partial,
    Blocked,
    Failed,
    Cancelled,
}

impl From<JobOutcome> for Outcome {
    fn from(value: JobOutcome) -> Self {
        match value {
            JobOutcome::Succeeded => Outcome::Succeeded,
            JobOutcome::Partial => Outcome::Partial,
            JobOutcome::Blocked => Outcome::Blocked,
            JobOutcome::Failed => Outcome::Failed,
            JobOutcome::Cancelled => Outcome::Cancelled,
        }
    }
}

/// Cancellation token handed to a running job. Long-running work checks
/// `is_cancelled` between batches and returns a partial `JobReport` early.
#[derive(Debug, Clone)]
pub struct JobCancellation {
    flag: Arc<std::sync::atomic::AtomicBool>,
}

impl JobCancellation {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
    pub fn cancel(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Default for JobCancellation {
    fn default() -> Self {
        Self::new()
    }
}

/// Context passed to a job. Carries the run id, the deadline (a wall-clock
/// instant) and the cancellation token. Job handlers must respect both.
pub struct JobContext {
    pub run_id: String,
    pub deadline: Option<UnixSeconds>,
    pub cancel: JobCancellation,
}

/// One registered job handler. Implementations must be idempotent: the
/// runtime may call them again after a crash on the same logical batch
/// (the docs §7 contract).
pub trait Job: Send + Sync {
    fn job_type(&self) -> &str;
    fn execute(&self, params: &serde_json::Value, ctx: &JobContext) -> JobReport;
}

/// Registry of `job_type` handlers. Tests inject their own; production
/// builds register `workspace_refresh` + `memory_outbox_replay` from
/// `scheduler_jobs`.
#[derive(Default, Clone)]
pub struct JobRegistry {
    handlers: Arc<RwLock<HashMap<String, Arc<dyn Job>>>>,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, handler: Arc<dyn Job>) {
        let key = handler.job_type().to_owned();
        self.handlers.write().insert(key, handler);
    }

    pub fn get(&self, job_type: &str) -> Option<Arc<dyn Job>> {
        self.handlers.read().get(job_type).cloned()
    }

    pub fn types(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.handlers.read().keys().cloned().collect();
        keys.sort();
        keys
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// Dispatcher output: one run that the caller must execute. The scheduler
/// has already inserted the queued row; the caller drives the executor.
#[derive(Debug, Clone)]
pub struct DispatchedRun {
    pub run: TaskRun,
    pub task: ScheduledTask,
}

#[derive(Debug, Clone)]
pub struct DispatcherConfig {
    pub limits: AdmissionLimits,
    pub max_grace_seconds: u64,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            limits: AdmissionLimits::default(),
            max_grace_seconds: 3600,
        }
    }
}

pub struct Scheduler {
    store: SchedulerStore,
    clock: Arc<dyn Clock>,
    resources: ResourceGuard,
    jobs: JobRegistry,
    config: DispatcherConfig,
}

impl Scheduler {
    pub fn open(state_dir: &Path, clock: Arc<dyn Clock>) -> Result<Self, SchedulerError> {
        Self::open_with(
            state_dir,
            clock,
            JobRegistry::new(),
            DispatcherConfig::default(),
        )
    }

    pub fn open_with(
        state_dir: &Path,
        clock: Arc<dyn Clock>,
        jobs: JobRegistry,
        config: DispatcherConfig,
    ) -> Result<Self, SchedulerError> {
        let store = SchedulerStore::open(state_dir)?;
        Ok(Self {
            store,
            clock,
            resources: ResourceGuard::new(),
            jobs,
            config,
        })
    }

    pub fn store(&self) -> &SchedulerStore {
        &self.store
    }

    pub fn jobs(&self) -> &JobRegistry {
        &self.jobs
    }

    /// Mutable view of the job registry. The Tauri binding uses this at
    /// boot to register the default handlers; tests use it to register
    /// fakes on top of an empty registry. Both happen at construction
    /// time, before the first tick.
    pub fn jobs_mut(&mut self) -> &mut JobRegistry {
        &mut self.jobs
    }

    /// Returns the next five future fire-at instants (UTC seconds) for the
    /// given schedule, sharing the calculator with the dispatcher.
    pub fn preview_schedule(
        &self,
        schedule: &Schedule,
        after: UnixSeconds,
    ) -> Result<Vec<UnixSeconds>, SchedulerError> {
        next_occurrences(schedule, after, PREVIEW_COUNT)
            .map_err(|_| SchedulerError::InvalidSchedule)
    }

    /// Compute and persist the next `next_fire_at` for one task. Called by
    /// the GUI after every edit and by the dispatcher after each tick.
    pub fn refresh_next_fire_at(&self, task_id: &str) -> Result<(), SchedulerError> {
        let Some(task) = self.store.get_task(task_id)? else {
            return Err(SchedulerError::UnknownTask);
        };
        if !task.state.dispatcher_should_run() || task.archived {
            self.store.set_next_fire_at(task_id, None)?;
            return Ok(());
        }
        let now = self.clock.now_unix();
        let next = next_occurrences(&task.schedule, now, 1)
            .map_err(|_| SchedulerError::InvalidSchedule)?;
        let next = next.into_iter().next();
        self.store.set_next_fire_at(task_id, next)?;
        Ok(())
    }

    /// Insert a new task. Caller passes the fully-validated
    /// `ScheduledTask`; storage enforces the unique key.
    pub fn create_task(&self, mut task: ScheduledTask) -> Result<ScheduledTask, SchedulerError> {
        let now = self.clock.now_unix();
        task.created_at = now;
        task.updated_at = now;
        task.revision = 1;
        task.schedule_generation = 1;
        task.archived = false;
        validate(&task.schedule, now).map_err(|_| SchedulerError::InvalidSchedule)?;
        self.store.upsert_task(&task)?;
        self.refresh_next_fire_at(&task.id)?;
        Ok(task)
    }

    /// Update an existing task. `expected_revision` prevents lost updates
    /// (the GUI re-fetches and retries when it sees a mismatch).
    pub fn update_task(
        &self,
        expected_revision: u64,
        mut task: ScheduledTask,
    ) -> Result<ScheduledTask, SchedulerError> {
        let Some(previous) = self.store.get_task(&task.id)? else {
            return Err(SchedulerError::UnknownTask);
        };
        if previous.revision != expected_revision {
            return Err(SchedulerError::RevisionConflict);
        }
        let now = self.clock.now_unix();
        task.created_at = previous.created_at;
        task.updated_at = now;
        task.revision = previous.revision + 1;
        task.archived = previous.archived;
        // Schedule edits bump the generation so an edit never replays
        // already-consumed moments. Pure title/state edits do not.
        if previous.schedule != task.schedule {
            task.schedule_generation = previous.schedule_generation + 1;
        } else {
            task.schedule_generation = previous.schedule_generation;
        }
        validate(&task.schedule, now).map_err(|_| SchedulerError::InvalidSchedule)?;
        self.store.upsert_task(&task)?;
        self.refresh_next_fire_at(&task.id)?;
        Ok(task)
    }

    pub fn set_enabled(
        &self,
        task_id: &str,
        enabled: bool,
    ) -> Result<ScheduledTask, SchedulerError> {
        // Read first so an unknown id returns UnknownTask instead of a
        // silent state row update on a non-existent task.
        if self.store.get_task(task_id)?.is_none() {
            return Err(SchedulerError::UnknownTask);
        }
        let new_state = if enabled {
            TaskState::Enabled
        } else {
            TaskState::Paused
        };
        self.store.set_state(task_id, new_state)?;
        self.refresh_next_fire_at(task_id)?;
        self.store
            .get_task(task_id)?
            .ok_or(SchedulerError::UnknownTask)
    }

    pub fn archive_task(&self, task_id: &str) -> Result<(), SchedulerError> {
        let Some(task) = self.store.get_task(task_id)? else {
            return Err(SchedulerError::UnknownTask);
        };
        // Refuse to archive while a live run owns the task's resources;
        // the caller cancels first and re-archives once finished.
        if task.state == TaskState::Enabled {
            return Err(SchedulerError::ResourceBusy);
        }
        self.store.archive_task(task_id)?;
        self.refresh_next_fire_at(task_id)?;
        Ok(())
    }

    /// Materialize a manual "run now" as a fresh `task_run`. The occurrence
    /// key is `manual:<id>:<request_id>`, so a duplicate click cannot create
    /// two queued runs.
    pub fn enqueue_manual(
        &self,
        task_id: &str,
        request_id: &str,
    ) -> Result<DispatchedRun, SchedulerError> {
        let task = self
            .store
            .get_task(task_id)?
            .ok_or(SchedulerError::UnknownTask)?;
        if task.archived {
            return Err(SchedulerError::UnknownTask);
        }
        let now = self.clock.now_unix();
        let run = build_run(
            &task,
            TriggerSource::Manual,
            &format!("manual:{task_id}:{request_id}"),
            task.schedule_generation,
            task.schedule_generation_generated(task.schedule_generation, now),
            now,
            0,
        );
        self.dispatch_one(task, run)
    }

    /// One dispatcher tick. Walks every enabled task, materializes the next
    /// due occurrence into a `task_runs` row, and returns the resulting
    /// dispatch plan. The caller actually runs them. Re-running `tick` for
    /// the same `(task, after)` is safe — the `occurrence_key` UNIQUE
    /// constraint deduplicates, and the second tick treats the
    /// already-queued row as a no-op rather than an error.
    pub fn tick(&self) -> Result<Vec<DispatchedRun>, SchedulerError> {
        let now = self.clock.now_unix();
        let tasks = self.store.list_tasks(false)?;
        let mut out = Vec::new();
        for task in tasks {
            if !task.state.dispatcher_should_run() {
                continue;
            }
            match self.due_for(&task, now) {
                Ok(Some(plan)) => {
                    let run = build_run(
                        &task,
                        TriggerSource::Scheduled,
                        &plan.occurrence_key,
                        task.schedule_generation,
                        plan.scheduled_for,
                        now,
                        plan.skipped_count,
                    );
                    match self.dispatch_one(task.clone(), run) {
                        Ok(dispatched) => out.push(dispatched),
                        // `AlreadyFinished` here means the run row was
                        // already inserted (either by an earlier tick that
                        // succeeded, or by a manual trigger that won the
                        // race). The occurrence is the same, the work is
                        // already in flight — no need to alarm the caller.
                        Err(SchedulerError::AlreadyFinished) => {}
                        Err(SchedulerError::ResourceBusy) => {
                            // Drop the new run and let overlap-policy
                            // decide. We log the skip in the audit trail.
                            self.store.record_skipped(
                                &task.id,
                                &plan.occurrence_key,
                                plan.scheduled_for,
                                "resource_busy",
                                plan.skipped_count,
                            )?;
                        }
                        Err(error) => return Err(error),
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    eprintln!("kodo scheduler: due_for({}) failed: {error}", task.id);
                }
            }
        }
        Ok(out)
    }

    fn dispatch_one(
        &self,
        task: ScheduledTask,
        run: TaskRun,
    ) -> Result<DispatchedRun, SchedulerError> {
        // `insert_queued_run` returns false when the same `occurrence_key`
        // already exists. The dispatcher treats that as a no-op.
        let inserted = self.store.insert_queued_run(&run)?;
        if !inserted {
            return Err(SchedulerError::AlreadyFinished);
        }
        // Advance the cursor. For `Once`, the next fire is `None`. For
        // everything else, it is the second occurrence strictly after the
        // one we just consumed.
        let cursor = match &task.schedule {
            Schedule::Once { .. } => None,
            _ => next_occurrences(&task.schedule, run.scheduled_for, 1)
                .ok()
                .and_then(|v| v.into_iter().next()),
        };
        self.store.set_next_fire_at(&task.id, cursor)?;
        Ok(DispatchedRun { run, task })
    }

    fn due_for(
        &self,
        task: &ScheduledTask,
        now: UnixSeconds,
    ) -> Result<Option<DuePlan>, SchedulerError> {
        // Walk the next N occurrences strictly after `now - grace`. Split
        // into "passed" (at or before `now`) and "future" (after `now`).
        // This is the same calculator the GUI uses for the live preview,
        // so the two can never disagree about what is due.
        let grace = task.grace_seconds.max(self.config.max_grace_seconds);
        let after = now.saturating_sub(grace as i64);
        let upcoming = next_occurrences(&task.schedule, after, PREVIEW_COUNT + 1)
            .map_err(|_| SchedulerError::InvalidSchedule)?;
        let passed: Vec<UnixSeconds> = upcoming.iter().copied().filter(|ts| *ts <= now).collect();
        if passed.is_empty() {
            return Ok(None);
        }
        match task.misfire_policy {
            MisfirePolicy::Skip => {
                // Out-of-grace moments were filtered out by `after`; here
                // we record every remaining passed moment as `skipped`
                // because the user said "don't run on catch-up".
                for (count, fire_at) in passed.iter().rev().enumerate() {
                    let key = kodo_core::scheduler::occurrence_key(
                        &task.id,
                        task.schedule_generation,
                        *fire_at,
                    );
                    self.store.record_skipped(
                        &task.id,
                        &key,
                        *fire_at,
                        "outside_grace",
                        count as u32,
                    )?;
                }
                Ok(None)
            }
            MisfirePolicy::LatestOnce | MisfirePolicy::Coalesce => {
                // The latest moment we have not yet fired. Earlier
                // moments collapse into a single `skipped_count`.
                let fire_at = *passed.last().expect("non-empty");
                let key = kodo_core::scheduler::occurrence_key(
                    &task.id,
                    task.schedule_generation,
                    fire_at,
                );
                let skipped = (passed.len() as u32).saturating_sub(1);
                Ok(Some(DuePlan {
                    scheduled_for: fire_at,
                    occurrence_key: key,
                    skipped_count: skipped,
                }))
            }
        }
    }

    pub fn try_admit(
        &self,
        run_id: &str,
        resource_keys: &[String],
    ) -> Result<ResourceHandle, SchedulerError> {
        self.resources
            .try_admit(run_id, resource_keys, &self.config.limits)
    }

    pub fn mark_started(&self, run_id: &str) -> Result<(), SchedulerError> {
        let now = self.clock.now_unix();
        // `mark_running` returns the prior lifecycle for callers that want
        // to detect stale state; the public `mark_started` API discards it
        // because by construction we only ever mark a freshly-dispatched
        // `Queued` run.
        self.store.mark_running(run_id, now).map(|_| ())
    }

    /// Mark a run finished. Releases the resource handle the caller holds.
    pub fn mark_finished(
        &self,
        run_id: &str,
        outcome: Outcome,
        delivery: Option<&str>,
        verification: Option<&str>,
        session_id: Option<&str>,
        report_ref: Option<&str>,
        reason_code: Option<&str>,
        error_summary: Option<&str>,
        handle: ResourceHandle,
    ) -> Result<(), SchedulerError> {
        let now = self.clock.now_unix();
        self.store.mark_finished(
            run_id,
            now,
            outcome,
            delivery,
            verification,
            session_id,
            report_ref,
            reason_code,
            error_summary,
        )?;
        release(handle, &self.resources);
        Ok(())
    }

    pub fn request_cancel(&self, run_id: &str) -> Result<bool, SchedulerError> {
        self.store.request_cancel(run_id)
    }
}

#[derive(Debug, Clone)]
struct DuePlan {
    scheduled_for: UnixSeconds,
    occurrence_key: String,
    skipped_count: u32,
}

fn build_run(
    task: &ScheduledTask,
    trigger: TriggerSource,
    occurrence_key: &str,
    schedule_generation: u64,
    scheduled_for: UnixSeconds,
    queued_at: UnixSeconds,
    skipped_count: u32,
) -> TaskRun {
    TaskRun {
        id: format!("run-{occurrence_key}"),
        task_id: task.id.clone(),
        occurrence_key: occurrence_key.to_owned(),
        trigger_source: trigger,
        schedule_generation,
        scheduled_for,
        queued_at,
        started_at: None,
        finished_at: None,
        config_revision: task.revision,
        config_snapshot: task.task.clone(),
        lifecycle: Lifecycle::Queued,
        outcome: None,
        delivery: None,
        verification: None,
        session_id: None,
        report_ref: None,
        reason_code: None,
        error_summary: None,
        skipped_count,
        attempts: Vec::new(),
    }
}

// `ScheduledTask` keeps `next_fire_at` and `scheduled_for` separate; this
// helper avoids leaking that detail into every caller. Not used yet but
// kept here for future manual-trigger helpers.
trait ScheduledTaskExt {
    fn schedule_generation_generated(&self, generation: u64, now: UnixSeconds) -> UnixSeconds;
}
impl ScheduledTaskExt for ScheduledTask {
    fn schedule_generation_generated(&self, _generation: u64, now: UnixSeconds) -> UnixSeconds {
        now
    }
}

// ---------------------------------------------------------------------------
// Inventory helpers — used by the GUI and the recovery walk.
// ---------------------------------------------------------------------------

/// Map a task list to (id, title, next_fire_at) tuples. Pure projection —
/// kept here so the GUI and the dispatcher see the same shape.
pub fn task_summary(task: &ScheduledTask) -> TaskSummary {
    TaskSummary {
        id: task.id.clone(),
        title: task.title.clone(),
        state: task.state,
        next_fire_at: task.next_fire_at,
        revision: task.revision,
        schedule_generation: task.schedule_generation,
    }
}

#[derive(Debug, Clone)]
pub struct TaskSummary {
    pub id: String,
    pub title: String,
    pub state: TaskState,
    pub next_fire_at: Option<UnixSeconds>,
    pub revision: u64,
    pub schedule_generation: u64,
}

/// Walk in-flight runs and stamp a deterministic terminal state for any
/// that have no live executor. Used by the Tauri boot path: a process
/// restart cannot resume Agent runs it does not own.
pub fn recover_inflight(store: &SchedulerStore, now: UnixSeconds) -> Result<usize, SchedulerError> {
    let inflight = store.list_inflight()?;
    let mut count = 0;
    for run in inflight {
        let outcome = match run.lifecycle {
            Lifecycle::Queued => Outcome::Skipped,
            Lifecycle::Running | Lifecycle::Cancelling => Outcome::Interrupted,
            Lifecycle::Finished => continue,
        };
        store.mark_finished(
            &run.id,
            now,
            outcome,
            None,
            None,
            None,
            None,
            Some("interrupted_at_restart"),
            Some("scheduler restarted while run was in flight"),
        )?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kodo_core::scheduler::FixedClock;
    use tempfile::TempDir;

    fn open_store(tag: &str) -> (TempDir, SchedulerStore) {
        let dir = TempDir::new().unwrap();
        let store = SchedulerStore::open(dir.path()).unwrap();
        let _ = tag;
        (dir, store)
    }

    fn sample_task(now: UnixSeconds, schedule: Schedule) -> ScheduledTask {
        ScheduledTask {
            id: "t1".into(),
            title: "demo".into(),
            state: TaskState::Enabled,
            revision: 1,
            schedule_generation: 1,
            schedule,
            task: TaskKind::Job {
                job_type: "demo".into(),
                params: serde_json::json!({}),
                resources: vec![],
            },
            timezone: "UTC".into(),
            misfire_policy: MisfirePolicy::LatestOnce,
            grace_seconds: 3600,
            overlap_policy: OverlapPolicy::Skip,
            timeout_seconds: None,
            created_at: now,
            updated_at: now,
            next_fire_at: None,
            archived: false,
        }
    }

    fn scheduler(dir: &TempDir, clock: Arc<dyn Clock>) -> Scheduler {
        Scheduler::open_with(
            dir.path(),
            clock,
            JobRegistry::new(),
            DispatcherConfig::default(),
        )
        .unwrap()
    }

    #[test]
    fn store_round_trips_a_task() {
        let (_dir, store) = open_store("rt");
        let now = 1_700_000_000;
        let task = sample_task(
            now,
            Schedule::Interval {
                anchor: now + 60,
                interval_seconds: 120,
            },
        );
        store.upsert_task(&task).unwrap();
        let loaded = store.get_task("t1").unwrap().unwrap();
        assert_eq!(loaded.title, "demo");
        assert_eq!(loaded.schedule_generation, 1);
        match loaded.schedule {
            Schedule::Interval {
                anchor,
                interval_seconds,
            } => {
                assert_eq!(anchor, now + 60);
                assert_eq!(interval_seconds, 120);
            }
            _ => panic!("expected Interval"),
        }
    }

    #[test]
    fn dispatch_dedupes_via_occurrence_key() {
        let dir = TempDir::new().unwrap();
        let clock = Arc::new(FixedClock {
            unix: 1_700_000_000,
            mono: 0,
        });
        let sched = scheduler(&dir, clock.clone());
        let task = sample_task(
            clock.now_unix(),
            Schedule::Once {
                fire_at: clock.now_unix() + 60,
            },
        );
        sched.create_task(task).unwrap();

        // First tick at fire_at - 1: nothing due yet.
        let clock_one = Arc::new(FixedClock {
            unix: 1_700_000_059,
            mono: 0,
        });
        let sched2 = Scheduler::open_with(
            dir.path(),
            clock_one.clone(),
            JobRegistry::new(),
            DispatcherConfig::default(),
        )
        .unwrap();
        assert!(sched2.tick().unwrap().is_empty());

        // Second tick after fire_at: one run.
        let clock_two = Arc::new(FixedClock {
            unix: 1_700_000_061,
            mono: 0,
        });
        let sched3 = Scheduler::open_with(
            dir.path(),
            clock_two.clone(),
            JobRegistry::new(),
            DispatcherConfig::default(),
        )
        .unwrap();
        let plan = sched3.tick().unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].run.scheduled_for, 1_700_000_060);
        assert_eq!(
            plan[0].run.occurrence_key,
            "sched:t1:1:1700000060".to_string()
        );

        // Third tick: dedup — already queued, no new run.
        let plan = sched3.tick().unwrap();
        assert!(
            plan.is_empty(),
            "dedup should keep us from double-dispatching"
        );
    }

    #[test]
    fn update_task_rejects_stale_revision() {
        let dir = TempDir::new().unwrap();
        let clock = Arc::new(FixedClock {
            unix: 1_700_000_000,
            mono: 0,
        });
        let sched = scheduler(&dir, clock.clone());
        let mut task = sample_task(
            clock.now_unix(),
            Schedule::Interval {
                anchor: 1_700_000_000 + 60,
                interval_seconds: 60,
            },
        );
        sched.create_task(task.clone()).unwrap();

        // First update succeeds.
        task.title = "renamed".into();
        let updated = sched.update_task(1, task.clone()).unwrap();
        assert_eq!(updated.revision, 2);
        assert_eq!(updated.title, "renamed");

        // Second update with the stale revision is refused.
        let again = sched.update_task(1, task);
        assert!(matches!(again, Err(SchedulerError::RevisionConflict)));
    }

    #[test]
    fn update_bumps_generation_only_on_schedule_change() {
        let dir = TempDir::new().unwrap();
        let clock = Arc::new(FixedClock {
            unix: 1_700_000_000,
            mono: 0,
        });
        let sched = scheduler(&dir, clock.clone());
        let mut task = sample_task(
            clock.now_unix(),
            Schedule::Interval {
                anchor: 1_700_000_000 + 60,
                interval_seconds: 60,
            },
        );
        sched.create_task(task.clone()).unwrap();

        task.title = "renamed".into();
        let updated = sched.update_task(1, task.clone()).unwrap();
        assert_eq!(
            updated.schedule_generation, 1,
            "title edit keeps generation"
        );

        task.schedule = Schedule::Interval {
            anchor: 1_700_000_000 + 120,
            interval_seconds: 90,
        };
        let updated = sched.update_task(2, task).unwrap();
        assert_eq!(
            updated.schedule_generation, 2,
            "schedule edit bumps generation"
        );
    }

    #[test]
    fn resource_guard_admits_one_run_per_key() {
        let guard = ResourceGuard::new();
        let limits = AdmissionLimits {
            total: 4,
            per_agent: 1,
        };
        let h1 = guard
            .try_admit("a", &["agent:/p".into(), "job:ws".into()], &limits)
            .unwrap();
        // A second run on the same project is refused.
        let busy = guard.try_admit("b", &["agent:/p".into()], &limits);
        assert!(matches!(busy, Err(SchedulerError::ResourceBusy)));
        // Releasing h1 frees the keys.
        release(h1, &guard);
        let _h2 = guard.try_admit("c", &["agent:/p".into()], &limits).unwrap();
    }

    #[test]
    fn preview_matches_dispatcher_calculator() {
        let dir = TempDir::new().unwrap();
        let clock = Arc::new(FixedClock {
            unix: 1_700_000_000,
            mono: 0,
        });
        let sched = scheduler(&dir, clock.clone());
        let schedule = Schedule::Daily {
            hour: 9,
            minute: 0,
            tz_name: "Asia/Shanghai".into(),
        };
        let preview = sched.preview_schedule(&schedule, clock.now_unix()).unwrap();
        let task = sample_task(clock.now_unix(), schedule);
        sched.create_task(task).unwrap();
        // `next_fire_at` is the first preview slot.
        let stored = sched.store().get_task("t1").unwrap().unwrap();
        assert_eq!(stored.next_fire_at, preview.first().copied());
    }

    #[test]
    fn misfire_skip_records_skipped_rows() {
        let dir = TempDir::new().unwrap();
        let base = 1_700_000_000;
        let clock = Arc::new(FixedClock {
            unix: base + 7200, // 2 hours past anchor
            mono: 0,
        });
        let sched = scheduler(&dir, clock.clone());
        let task = sample_task(
            base,
            Schedule::Interval {
                anchor: base,
                interval_seconds: 60,
            },
        );
        let mut task = task;
        task.misfire_policy = MisfirePolicy::Skip;
        sched.create_task(task).unwrap();

        let plan = sched.tick().unwrap();
        assert!(plan.is_empty(), "skip misfire should not enqueue anything");
        let inflight = sched.store().list_inflight().unwrap();
        assert!(inflight.is_empty());
        let skipped: Vec<_> = sched
            .store()
            .list_runs("t1", 100)
            .unwrap()
            .into_iter()
            .filter(|r| r.outcome == Some(Outcome::Skipped))
            .collect();
        assert!(!skipped.is_empty(), "skip policy must record skipped rows");
    }

    #[test]
    fn recover_inflight_marks_running_as_interrupted() {
        let dir = TempDir::new().unwrap();
        let base = 1_700_000_000;
        let clock = Arc::new(FixedClock {
            unix: base,
            mono: 0,
        });
        let _sched = scheduler(&dir, clock.clone());
        // `fire_at` is 60s in the future relative to the clock at create
        // time. Validation requires `fire_at > now`, so we have to give
        // the future moment a slightly later now when we construct the
        // task.
        let create_clock = Arc::new(FixedClock {
            unix: base - 1,
            mono: 0,
        });
        let create_sched = Scheduler::open_with(
            dir.path(),
            create_clock.clone(),
            JobRegistry::new(),
            DispatcherConfig::default(),
        )
        .unwrap();
        let task = sample_task(
            create_clock.now_unix(),
            Schedule::Once { fire_at: base + 60 },
        );
        create_sched.create_task(task).unwrap();

        // Advance the clock past fire_at and tick — the dispatcher should
        // emit exactly one run.
        let later = Arc::new(FixedClock {
            unix: base + 120,
            mono: 0,
        });
        let later_sched = Scheduler::open_with(
            dir.path(),
            later.clone(),
            JobRegistry::new(),
            DispatcherConfig::default(),
        )
        .unwrap();
        let mut plan = later_sched.tick().unwrap();
        let run = plan.pop().unwrap().run;
        later_sched.try_admit(&run.id, &[]).unwrap();
        later_sched.mark_started(&run.id).unwrap();

        // Simulate a restart: a fresh scheduler on the same store, with a
        // clock that says "now is a bit later than when we marked it
        // running". `recover_inflight` should mark it interrupted.
        let recovered = recover_inflight(later_sched.store(), later.now_unix() + 5).unwrap();
        assert_eq!(recovered, 1);
        let stored = later_sched.store().get_run(&run.id).unwrap().unwrap();
        assert_eq!(stored.outcome, Some(Outcome::Interrupted));
        assert_eq!(stored.lifecycle, Lifecycle::Finished);
    }

    #[test]
    fn once_task_does_not_produce_a_second_dispatch() {
        let dir = TempDir::new().unwrap();
        let base = 1_700_000_000;
        let early = Arc::new(FixedClock {
            unix: base,
            mono: 0,
        });
        let early_sched = Scheduler::open_with(
            dir.path(),
            early.clone(),
            JobRegistry::new(),
            DispatcherConfig::default(),
        )
        .unwrap();
        let task = sample_task(early.now_unix(), Schedule::Once { fire_at: base + 60 });
        early_sched.create_task(task).unwrap();

        // First tick after fire_at: one run.
        let later = Arc::new(FixedClock {
            unix: base + 120,
            mono: 0,
        });
        let later_sched = Scheduler::open_with(
            dir.path(),
            later.clone(),
            JobRegistry::new(),
            DispatcherConfig::default(),
        )
        .unwrap();
        let first = later_sched.tick().unwrap();
        assert_eq!(first.len(), 1);

        // Second tick: nothing — `Once` consumes itself.
        let second = later_sched.tick().unwrap();
        assert!(second.is_empty());
        let stored = later_sched.store().get_task("t1").unwrap().unwrap();
        assert_eq!(stored.next_fire_at, None, "Once consumes itself");
    }

    #[test]
    fn archive_refuses_when_task_still_enabled() {
        let dir = TempDir::new().unwrap();
        let clock = Arc::new(FixedClock {
            unix: 1_700_000_000,
            mono: 0,
        });
        let sched = scheduler(&dir, clock.clone());
        let task = sample_task(
            clock.now_unix(),
            Schedule::Interval {
                anchor: 1_700_000_000 + 60,
                interval_seconds: 60,
            },
        );
        sched.create_task(task).unwrap();
        let err = sched.archive_task("t1").unwrap_err();
        assert!(matches!(err, SchedulerError::ResourceBusy));

        // Pause first, then archive.
        sched.set_enabled("t1", false).unwrap();
        sched.archive_task("t1").unwrap();
        let stored = sched.store().get_task("t1").unwrap().unwrap();
        assert!(stored.archived);
        assert_eq!(stored.state, TaskState::Archived);
    }

    // Helpers kept around for future ordering / admission tests.
    #[allow(dead_code)]
    fn _unused_helpers() {
        // Placeholder so the test module keeps its familiar shape.
    }
}
