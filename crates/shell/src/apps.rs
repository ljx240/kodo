//! Workbench storage and lifecycle.
//!
//! One SQLite database (`<state_dir>/workbench/v1/workbench.sqlite`) holds
//! the per-install app state, the workbench UI preferences, and the business
//! data of the first-party apps. It is the single source of truth for todos,
//! calendar events and artifacts — no UI state is mirrored elsewhere.
//!
//! Layout mirrors the scheduler store: one [`parking_lot::Mutex`] around a
//! long-lived connection, every method a short transaction, and optimistic
//! concurrency via a `version` column. Creates take an optional client
//! `request_id` so a repeated request returns the original row instead of
//! inserting a duplicate.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, Row};

use kodo_core::apps::{
    builtin_apps, clean_body, clean_title, system_timezone, validate_date, validate_time,
    validate_timezone, AppError, AppManifest, AppPreference, AppView, Artifact, ArtifactDraft,
    ArtifactFileStatus, ArtifactKind, ArtifactView, CalendarEvent, CalendarEventDraft, Todo,
    TodoDraft, TodoPriority, TodoStatus,
};

use crate::scheduler::{Job, JobContext, JobOutcome, JobReport};

const MIGRATION_0001: &str = include_str!("../migrations/workbench_0001.sql");

/// Filename under `<state_dir>/workbench/v1/`.
const DB_FILE: &str = "workbench.sqlite";

/// Default sort slot for an app the store has never seen.
fn default_sort_order(app_id: &str) -> i64 {
    builtin_apps()
        .iter()
        .position(|a| a.app_id == app_id)
        .map(|index| index as i64)
        .unwrap_or(i64::MAX)
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Process-local id generator. Ids only need to be unique within one
/// workbench database, so a timestamp plus a counter is enough — and it
/// keeps the crate free of a uuid dependency.
fn new_id(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{nanos:x}{seq:x}")
}

fn file_fingerprint(path: &str) -> (Option<i64>, Option<i64>) {
    match std::fs::metadata(path) {
        Ok(meta) => {
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            (Some(meta.len() as i64), modified)
        }
        Err(_) => (None, None),
    }
}

/// Workbench database handle. Cheap to clone-free share behind an `Arc`;
/// the inner connection is serialized by a mutex.
pub struct AppStore {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl AppStore {
    /// Open (creating if needed) the workbench database under `state_dir`.
    pub fn open(state_dir: &Path) -> Result<Self, AppError> {
        let dir = state_dir.join("workbench").join("v1");
        std::fs::create_dir_all(&dir).map_err(|error| {
            eprintln!("kodo workbench: cannot create {}: {error}", dir.display());
            AppError::StorageUnavailable
        })?;
        let path = dir.join(DB_FILE);
        let conn = Connection::open(&path).map_err(store_err)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(store_err)?;
        conn.pragma_update(None, "busy_timeout", 5000_i64)
            .map_err(store_err)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(store_err)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(store_err)?;
        let mut store = Self {
            conn: Mutex::new(conn),
            path,
        };
        store.migrate()?;
        Ok(store)
    }

    /// Open using the host's state directory. `HOME` must be set.
    pub fn open_default() -> Result<Self, AppError> {
        let state_dir = kodo_core::settings::state_dir().ok_or(AppError::StorageUnavailable)?;
        Self::open(&state_dir)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn migrate(&mut self) -> Result<(), AppError> {
        let conn = self.conn.get_mut();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                 version     INTEGER PRIMARY KEY,
                 applied_at  INTEGER NOT NULL,
                 description TEXT    NOT NULL
             )",
        )
        .map_err(store_err)?;

        let applied: Vec<i64> = {
            let mut stmt = conn
                .prepare("SELECT version FROM schema_migrations")
                .map_err(store_err)?;
            let rows = stmt
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(store_err)?;
            rows.filter_map(Result::ok).collect()
        };

        let migrations: &[(i64, &str, &str)] = &[(1, MIGRATION_0001, "workbench v1")];
        for (version, sql, description) in migrations {
            if applied.contains(version) {
                continue;
            }
            let tx = conn.transaction().map_err(store_err)?;
            tx.execute_batch(sql).map_err(store_err)?;
            tx.execute(
                "INSERT OR REPLACE INTO schema_migrations (version, applied_at, description)
                 VALUES (?1, ?2, ?3)",
                params![version, now_unix(), description],
            )
            .map_err(store_err)?;
            tx.commit().map_err(store_err)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Apps
    // ------------------------------------------------------------------

    fn read_preferences(&self) -> Result<BTreeMap<String, AppPreference>, AppError> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT app_id, enabled, favorite, sort_order, ai_enabled, schedule_enabled,
                        updated_at
                 FROM app_preferences",
            )
            .map_err(store_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(AppPreference {
                    app_id: row.get(0)?,
                    enabled: row.get::<_, i64>(1)? != 0,
                    favorite: row.get::<_, i64>(2)? != 0,
                    sort_order: row.get(3)?,
                    ai_enabled: row.get::<_, i64>(4)? != 0,
                    schedule_enabled: row.get::<_, i64>(5)? != 0,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(store_err)?;
        let mut out = BTreeMap::new();
        for row in rows {
            let pref = row.map_err(store_err)?;
            out.insert(pref.app_id.clone(), pref);
        }
        Ok(out)
    }

    fn write_preference(&self, pref: &AppPreference) -> Result<(), AppError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO app_preferences
                 (app_id, enabled, favorite, sort_order, ai_enabled, schedule_enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(app_id) DO UPDATE SET
                 enabled          = excluded.enabled,
                 favorite         = excluded.favorite,
                 sort_order       = excluded.sort_order,
                 ai_enabled       = excluded.ai_enabled,
                 schedule_enabled = excluded.schedule_enabled,
                 updated_at       = excluded.updated_at",
            params![
                pref.app_id,
                pref.enabled as i64,
                pref.favorite as i64,
                pref.sort_order,
                pref.ai_enabled as i64,
                pref.schedule_enabled as i64,
                pref.updated_at,
            ],
        )
        .map_err(store_err)?;
        Ok(())
    }

    fn preference_for(&self, manifest: &AppManifest) -> Result<AppPreference, AppError> {
        let prefs = self.read_preferences()?;
        Ok(prefs.get(&manifest.app_id).cloned().unwrap_or_else(|| {
            AppPreference::defaults(&manifest.app_id, default_sort_order(&manifest.app_id))
        }))
    }

    /// Every built-in app, merged with its stored preference. Pinned apps
    /// first, then by the user's sort order, then by id for stability.
    pub fn list_apps(&self) -> Result<Vec<AppView>, AppError> {
        let prefs = self.read_preferences()?;
        let mut views: Vec<AppView> = builtin_apps()
            .into_iter()
            .map(|manifest| {
                let preference = prefs.get(&manifest.app_id).cloned().unwrap_or_else(|| {
                    AppPreference::defaults(&manifest.app_id, default_sort_order(&manifest.app_id))
                });
                AppView {
                    manifest,
                    preference,
                }
            })
            .collect();
        views.sort_by(|a, b| {
            b.preference
                .favorite
                .cmp(&a.preference.favorite)
                .then(a.preference.sort_order.cmp(&b.preference.sort_order))
                .then(a.manifest.app_id.cmp(&b.manifest.app_id))
        });
        Ok(views)
    }

    fn known_manifest(&self, app_id: &str) -> Result<AppManifest, AppError> {
        builtin_apps()
            .into_iter()
            .find(|a| a.app_id == app_id)
            .ok_or(AppError::UnknownApp)
    }

    pub fn set_app_enabled(&self, app_id: &str, enabled: bool) -> Result<AppView, AppError> {
        let manifest = self.known_manifest(app_id)?;
        let mut pref = self.preference_for(&manifest)?;
        pref.enabled = enabled;
        pref.updated_at = now_unix();
        self.write_preference(&pref)?;
        Ok(AppView {
            manifest,
            preference: pref,
        })
    }

    pub fn set_app_favorite(&self, app_id: &str, favorite: bool) -> Result<AppView, AppError> {
        let manifest = self.known_manifest(app_id)?;
        let mut pref = self.preference_for(&manifest)?;
        pref.favorite = favorite;
        pref.updated_at = now_unix();
        self.write_preference(&pref)?;
        Ok(AppView {
            manifest,
            preference: pref,
        })
    }

    pub fn set_app_links(
        &self,
        app_id: &str,
        ai_enabled: bool,
        schedule_enabled: bool,
    ) -> Result<AppView, AppError> {
        let manifest = self.known_manifest(app_id)?;
        if ai_enabled && !manifest.capabilities.ai {
            return Err(AppError::InvalidInput);
        }
        if schedule_enabled && !manifest.capabilities.scheduler {
            return Err(AppError::InvalidInput);
        }
        let mut pref = self.preference_for(&manifest)?;
        pref.ai_enabled = ai_enabled;
        pref.schedule_enabled = schedule_enabled;
        pref.updated_at = now_unix();
        self.write_preference(&pref)?;
        Ok(AppView {
            manifest,
            preference: pref,
        })
    }

    /// Reorder apps. `app_ids` lists the apps in the wanted order; a built-in
    /// app the caller omits keeps its manifest position after the listed
    /// ones, so a caller holding a partial list cannot drop an app out of the
    /// ordering.
    pub fn set_app_order(&self, app_ids: &[String]) -> Result<Vec<AppView>, AppError> {
        for app_id in app_ids {
            self.known_manifest(app_id)?;
        }
        let prefs = self.read_preferences()?;
        let now = now_unix();
        let mut next = app_ids.len() as i64;
        for manifest in builtin_apps() {
            let mut pref = prefs.get(&manifest.app_id).cloned().unwrap_or_else(|| {
                AppPreference::defaults(&manifest.app_id, default_sort_order(&manifest.app_id))
            });
            pref.sort_order = match app_ids.iter().position(|id| *id == manifest.app_id) {
                Some(index) => index as i64,
                None => {
                    let slot = next;
                    next += 1;
                    slot
                }
            };
            pref.updated_at = now;
            self.write_preference(&pref)?;
        }
        self.list_apps()
    }

    /// Whether the app accepts new writes right now.
    pub fn app_enabled(&self, app_id: &str) -> Result<bool, AppError> {
        let manifest = self.known_manifest(app_id)?;
        Ok(self.preference_for(&manifest)?.enabled)
    }

    /// Whether the user opted this app's background actions into the scheduler.
    pub fn app_schedule_enabled(&self, app_id: &str) -> Result<bool, AppError> {
        let manifest = self.known_manifest(app_id)?;
        let pref = self.preference_for(&manifest)?;
        Ok(pref.enabled && pref.schedule_enabled)
    }

    // ------------------------------------------------------------------
    // Workbench preferences (key/value)
    // ------------------------------------------------------------------

    pub fn list_prefs(&self) -> Result<BTreeMap<String, String>, AppError> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT key, value FROM workbench_prefs")
            .map_err(store_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(store_err)?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (key, value) = row.map_err(store_err)?;
            out.insert(key, value);
        }
        Ok(out)
    }

    pub fn set_pref(&self, key: &str, value: &str) -> Result<(), AppError> {
        if key.trim().is_empty() || key.chars().count() > 120 {
            return Err(AppError::InvalidInput);
        }
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO workbench_prefs (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key.trim(), value, now_unix()],
        )
        .map_err(store_err)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Todos
    // ------------------------------------------------------------------

    fn read_todo(row: &Row<'_>) -> rusqlite::Result<Todo> {
        let status: String = row.get("status")?;
        let priority: String = row.get("priority")?;
        Ok(Todo {
            id: row.get("id")?,
            title: row.get("title")?,
            notes: row.get("notes")?,
            status: TodoStatus::parse(&status).unwrap_or(TodoStatus::Todo),
            priority: TodoPriority::parse(&priority).unwrap_or(TodoPriority::Normal),
            due_date: row.get("due_date")?,
            parent_id: row.get("parent_id")?,
            archived: row.get::<_, i64>("archived")? != 0,
            completed_at: row.get("completed_at")?,
            sort_order: row.get("sort_order")?,
            source: row.get("source")?,
            version: row.get("version")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }

    const TODO_COLUMNS: &'static str = "id, title, notes, status, priority, due_date, parent_id, \
         archived, completed_at, sort_order, source, version, created_at, updated_at";

    pub fn list_todos(&self, include_archived: bool) -> Result<Vec<Todo>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM todos {} ORDER BY sort_order ASC, created_at ASC",
            Self::TODO_COLUMNS,
            if include_archived {
                ""
            } else {
                "WHERE archived = 0"
            }
        );
        let mut stmt = stmt(&conn, &sql)?;
        let rows = stmt.query_map([], Self::read_todo).map_err(store_err)?;
        collect(rows)
    }

    pub fn get_todo(&self, id: &str) -> Result<Option<Todo>, AppError> {
        let conn = self.conn.lock();
        let sql = format!("SELECT {} FROM todos WHERE id = ?1", Self::TODO_COLUMNS);
        conn.query_row(&sql, params![id], Self::read_todo)
            .optional()
            .map_err(store_err)
    }

    /// Create a todo. Only `title` is required. A repeated `request_id`
    /// returns the todo created the first time.
    pub fn create_todo(&self, draft: &TodoDraft) -> Result<Todo, AppError> {
        let title = clean_title(&draft.title)?;
        let notes = clean_body(draft.notes.as_deref())?;
        let status = draft.status.unwrap_or(TodoStatus::Todo);
        let priority = draft.priority.unwrap_or(TodoPriority::Normal);
        let due_date = match draft.due_date.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => Some(validate_date(value)?),
            _ => None,
        };
        let parent_id = match draft.parent_id.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => {
                if self.get_todo(value)?.is_none() {
                    return Err(AppError::NotFound);
                }
                Some(value.to_owned())
            }
            _ => None,
        };
        let source = match draft.source.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => value.to_owned(),
            _ => "manual".to_owned(),
        };
        let request_id = draft
            .request_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);

        if let Some(key) = request_id.as_deref() {
            if let Some(existing) = self.todo_by_request(key)? {
                return Ok(existing);
            }
        }

        let now = now_unix();
        let todo = Todo {
            id: new_id("todo"),
            title,
            notes,
            status,
            priority,
            due_date,
            parent_id,
            archived: false,
            completed_at: (status == TodoStatus::Done).then_some(now),
            sort_order: self.next_sort_order()?,
            source,
            version: 1,
            created_at: now,
            updated_at: now,
        };

        {
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO todos
                     (id, title, notes, status, priority, due_date, parent_id, archived,
                      completed_at, sort_order, source, version, created_at, updated_at, request_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?10, 1, ?11, ?11, ?12)
                 ON CONFLICT(request_id) WHERE request_id IS NOT NULL DO NOTHING",
                params![
                    todo.id,
                    todo.title,
                    todo.notes,
                    todo.status.as_str(),
                    todo.priority.as_str(),
                    todo.due_date,
                    todo.parent_id,
                    todo.completed_at,
                    todo.sort_order,
                    todo.source,
                    todo.created_at,
                    request_id,
                ],
            )
            .map_err(store_err)?;
        }

        // A concurrent create with the same request_id won the unique index:
        // return that row so both callers see the same todo.
        if let Some(key) = request_id.as_deref() {
            if let Some(existing) = self.todo_by_request(key)? {
                return Ok(existing);
            }
        }
        Ok(todo)
    }

    fn todo_by_request(&self, request_id: &str) -> Result<Option<Todo>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM todos WHERE request_id = ?1",
            Self::TODO_COLUMNS
        );
        conn.query_row(&sql, params![request_id], Self::read_todo)
            .optional()
            .map_err(store_err)
    }

    fn next_sort_order(&self) -> Result<i64, AppError> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM todos",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(store_err)
    }

    /// Replace the editable fields of an existing todo. `expected_version`
    /// guards against overwriting an edit made after the caller read the row.
    pub fn update_todo(&self, expected_version: u64, patch: &Todo) -> Result<Todo, AppError> {
        let existing = self.get_todo(&patch.id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let title = clean_title(&patch.title)?;
        let notes = clean_body(Some(&patch.notes))?;
        let due_date = match patch.due_date.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => Some(validate_date(value)?),
            _ => None,
        };
        let parent_id = match patch.parent_id.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => {
                if value == existing.id {
                    return Err(AppError::InvalidInput);
                }
                if self.get_todo(value)?.is_none() {
                    return Err(AppError::NotFound);
                }
                Some(value.to_owned())
            }
            _ => None,
        };
        let completed_at = match (patch.status == TodoStatus::Done, existing.status) {
            (true, TodoStatus::Done) => existing.completed_at,
            (true, _) => Some(now_unix()),
            (false, _) => None,
        };
        let now = now_unix();
        {
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE todos SET title = ?2, notes = ?3, status = ?4, priority = ?5,
                     due_date = ?6, parent_id = ?7, completed_at = ?8, version = version + 1,
                     updated_at = ?9
                 WHERE id = ?1 AND version = ?10",
                params![
                    existing.id,
                    title,
                    notes,
                    patch.status.as_str(),
                    patch.priority.as_str(),
                    due_date,
                    parent_id,
                    completed_at,
                    now,
                    expected_version as i64,
                ],
            )
            .map_err(store_err)?;
        }
        self.get_todo(&existing.id)?.ok_or(AppError::NotFound)
    }

    /// Move a todo between board columns. This is the same write path the
    /// drag, the status select and the edit form all use.
    pub fn set_todo_status(
        &self,
        id: &str,
        expected_version: u64,
        status: TodoStatus,
    ) -> Result<Todo, AppError> {
        let existing = self.get_todo(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let completed_at = match (status == TodoStatus::Done, existing.status) {
            (true, TodoStatus::Done) => existing.completed_at,
            (true, _) => Some(now_unix()),
            (false, _) => None,
        };
        {
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE todos SET status = ?2, completed_at = ?3, version = version + 1,
                     updated_at = ?4
                 WHERE id = ?1 AND version = ?5",
                params![
                    id,
                    status.as_str(),
                    completed_at,
                    now_unix(),
                    expected_version as i64
                ],
            )
            .map_err(store_err)?;
        }
        self.get_todo(id)?.ok_or(AppError::NotFound)
    }

    /// Archive or restore. Archived todos keep every field.
    pub fn set_todo_archived(
        &self,
        id: &str,
        expected_version: u64,
        archived: bool,
    ) -> Result<Todo, AppError> {
        let existing = self.get_todo(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        {
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE todos SET archived = ?2, version = version + 1, updated_at = ?3
                 WHERE id = ?1 AND version = ?4",
                params![id, archived as i64, now_unix(), expected_version as i64],
            )
            .map_err(store_err)?;
        }
        self.get_todo(id)?.ok_or(AppError::NotFound)
    }

    // ------------------------------------------------------------------
    // Calendar
    // ------------------------------------------------------------------

    fn read_event(row: &Row<'_>) -> rusqlite::Result<CalendarEvent> {
        Ok(CalendarEvent {
            id: row.get("id")?,
            title: row.get("title")?,
            notes: row.get("notes")?,
            date: row.get("date")?,
            start_time: row.get("start_time")?,
            end_time: row.get("end_time")?,
            all_day: row.get::<_, i64>("all_day")? != 0,
            timezone: row.get("timezone")?,
            version: row.get("version")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }

    const EVENT_COLUMNS: &'static str =
        "id, title, notes, date, start_time, end_time, all_day, timezone, version, created_at, \
         updated_at";

    pub fn list_events(&self) -> Result<Vec<CalendarEvent>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM calendar_events ORDER BY date ASC, start_time ASC, created_at ASC",
            Self::EVENT_COLUMNS
        );
        let mut stmt = stmt(&conn, &sql)?;
        let rows = stmt.query_map([], Self::read_event).map_err(store_err)?;
        collect(rows)
    }

    pub fn get_event(&self, id: &str) -> Result<Option<CalendarEvent>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM calendar_events WHERE id = ?1",
            Self::EVENT_COLUMNS
        );
        conn.query_row(&sql, params![id], Self::read_event)
            .optional()
            .map_err(store_err)
    }

    pub fn create_event(&self, draft: &CalendarEventDraft) -> Result<CalendarEvent, AppError> {
        let title = clean_title(&draft.title)?;
        let notes = clean_body(draft.notes.as_deref())?;
        let date = validate_date(&draft.date)?;
        let timezone = match draft.timezone.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => validate_timezone(value)?,
            _ => system_timezone(),
        };
        let all_day = draft.all_day.unwrap_or(false);
        let (start_time, end_time) = if all_day {
            (None, None)
        } else {
            let start = validate_time(draft.start_time.as_deref().unwrap_or(""))?;
            let end = validate_time(draft.end_time.as_deref().unwrap_or(""))?;
            if end <= start {
                return Err(AppError::InvalidInput);
            }
            (Some(start), Some(end))
        };
        let request_id = draft
            .request_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);

        if let Some(key) = request_id.as_deref() {
            if let Some(existing) = self.event_by_request(key)? {
                return Ok(existing);
            }
        }

        let now = now_unix();
        let event = CalendarEvent {
            id: new_id("evt"),
            title,
            notes,
            date,
            start_time,
            end_time,
            all_day,
            timezone,
            version: 1,
            created_at: now,
            updated_at: now,
        };
        {
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO calendar_events
                     (id, title, notes, date, start_time, end_time, all_day, timezone,
                      version, created_at, updated_at, request_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?9, ?10)
                 ON CONFLICT(request_id) WHERE request_id IS NOT NULL DO NOTHING",
                params![
                    event.id,
                    event.title,
                    event.notes,
                    event.date,
                    event.start_time,
                    event.end_time,
                    event.all_day as i64,
                    event.timezone,
                    event.created_at,
                    request_id,
                ],
            )
            .map_err(store_err)?;
        }
        if let Some(key) = request_id.as_deref() {
            if let Some(existing) = self.event_by_request(key)? {
                return Ok(existing);
            }
        }
        Ok(event)
    }

    fn event_by_request(&self, request_id: &str) -> Result<Option<CalendarEvent>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM calendar_events WHERE request_id = ?1",
            Self::EVENT_COLUMNS
        );
        conn.query_row(&sql, params![request_id], Self::read_event)
            .optional()
            .map_err(store_err)
    }

    pub fn update_event(
        &self,
        expected_version: u64,
        patch: &CalendarEvent,
    ) -> Result<CalendarEvent, AppError> {
        let existing = self.get_event(&patch.id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let title = clean_title(&patch.title)?;
        let notes = clean_body(Some(&patch.notes))?;
        let date = validate_date(&patch.date)?;
        let timezone = validate_timezone(&patch.timezone)?;
        let (start_time, end_time) = if patch.all_day {
            (None, None)
        } else {
            let start = validate_time(patch.start_time.as_deref().unwrap_or(""))?;
            let end = validate_time(patch.end_time.as_deref().unwrap_or(""))?;
            if end <= start {
                return Err(AppError::InvalidInput);
            }
            (Some(start), Some(end))
        };
        {
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE calendar_events SET title = ?2, notes = ?3, date = ?4, start_time = ?5,
                     end_time = ?6, all_day = ?7, timezone = ?8, version = version + 1,
                     updated_at = ?9
                 WHERE id = ?1 AND version = ?10",
                params![
                    existing.id,
                    title,
                    notes,
                    date,
                    start_time,
                    end_time,
                    patch.all_day as i64,
                    timezone,
                    now_unix(),
                    expected_version as i64,
                ],
            )
            .map_err(store_err)?;
        }
        self.get_event(&existing.id)?.ok_or(AppError::NotFound)
    }

    pub fn delete_event(&self, id: &str, expected_version: u64) -> Result<(), AppError> {
        let existing = self.get_event(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let conn = self.conn.lock();
        conn.execute("DELETE FROM calendar_events WHERE id = ?1", params![id])
            .map_err(store_err)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Artifacts
    // ------------------------------------------------------------------

    fn read_artifact(row: &Row<'_>) -> rusqlite::Result<Artifact> {
        let kind: String = row.get("kind")?;
        Ok(Artifact {
            id: row.get("id")?,
            title: row.get("title")?,
            kind: ArtifactKind::parse(&kind).unwrap_or(ArtifactKind::Text),
            body: row.get("body")?,
            file_path: row.get("file_path")?,
            file_size: row.get("file_size")?,
            file_modified_at: row.get("file_modified_at")?,
            source: row.get("source")?,
            category: row.get("category")?,
            favorite: row.get::<_, i64>("favorite")? != 0,
            archived: row.get::<_, i64>("archived")? != 0,
            version: row.get("version")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }

    const ARTIFACT_COLUMNS: &'static str =
        "id, title, kind, body, file_path, file_size, file_modified_at, source, category, \
         favorite, archived, version, created_at, updated_at";

    /// Artifacts with the current on-disk state of any referenced file.
    pub fn list_artifacts(&self, include_archived: bool) -> Result<Vec<ArtifactView>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM artifacts {} ORDER BY favorite DESC, updated_at DESC",
            Self::ARTIFACT_COLUMNS,
            if include_archived {
                ""
            } else {
                "WHERE archived = 0"
            }
        );
        let mut stmt = stmt(&conn, &sql)?;
        let rows = stmt.query_map([], Self::read_artifact).map_err(store_err)?;
        let artifacts = collect(rows)?;
        Ok(artifacts.into_iter().map(resolve_file_status).collect())
    }

    pub fn get_artifact(&self, id: &str) -> Result<Option<ArtifactView>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM artifacts WHERE id = ?1",
            Self::ARTIFACT_COLUMNS
        );
        let artifact = conn
            .query_row(&sql, params![id], Self::read_artifact)
            .optional()
            .map_err(store_err)?;
        Ok(artifact.map(resolve_file_status))
    }

    /// The stored file reference behind one artifact, for the host to hand to
    /// the OS.
    ///
    /// The record is the allow-list: the GUI names a record id and never a
    /// path, so the shell can only ever open a file the user already
    /// registered. A text artifact (or a deleted record) has nothing to open
    /// and is refused.
    pub fn artifact_file_path(&self, id: &str) -> Result<String, AppError> {
        let conn = self.conn.lock();
        let stored: Option<Option<String>> = conn
            .query_row(
                "SELECT file_path FROM artifacts WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_err)?;
        match stored {
            None => Err(AppError::NotFound),
            Some(None) => Err(AppError::InvalidInput),
            Some(Some(path)) => Ok(path),
        }
    }

    pub fn create_artifact(&self, draft: &ArtifactDraft) -> Result<Artifact, AppError> {
        let title = clean_title(&draft.title)?;
        let (body, file_path, file_size, file_modified_at) = match draft.kind {
            ArtifactKind::Text => {
                let body = clean_body(draft.body.as_deref())?;
                if body.is_empty() {
                    return Err(AppError::InvalidInput);
                }
                (Some(body), None, None, None)
            }
            ArtifactKind::File => {
                let path = draft
                    .file_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .ok_or(AppError::InvalidInput)?
                    .to_owned();
                let (size, modified) = file_fingerprint(&path);
                (None, Some(path), size, modified)
            }
        };
        let source = match draft.source.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => value.to_owned(),
            _ => "manual".to_owned(),
        };
        let category = draft
            .category
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_owned();
        let request_id = draft
            .request_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);

        if let Some(key) = request_id.as_deref() {
            if let Some(existing) = self.artifact_by_request(key)? {
                return Ok(existing);
            }
        }

        let now = now_unix();
        let artifact = Artifact {
            id: new_id("art"),
            title,
            kind: draft.kind,
            body,
            file_path,
            file_size,
            file_modified_at,
            source,
            category,
            favorite: false,
            archived: false,
            version: 1,
            created_at: now,
            updated_at: now,
        };
        {
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO artifacts
                     (id, title, kind, body, file_path, file_size, file_modified_at, source,
                      category, favorite, archived, version, created_at, updated_at, request_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0, 1, ?10, ?10, ?11)
                 ON CONFLICT(request_id) WHERE request_id IS NOT NULL DO NOTHING",
                params![
                    artifact.id,
                    artifact.title,
                    artifact.kind.as_str(),
                    artifact.body,
                    artifact.file_path,
                    artifact.file_size,
                    artifact.file_modified_at,
                    artifact.source,
                    artifact.category,
                    artifact.created_at,
                    request_id,
                ],
            )
            .map_err(store_err)?;
        }
        if let Some(key) = request_id.as_deref() {
            if let Some(existing) = self.artifact_by_request(key)? {
                return Ok(existing);
            }
        }
        Ok(artifact)
    }

    fn artifact_by_request(&self, request_id: &str) -> Result<Option<Artifact>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM artifacts WHERE request_id = ?1",
            Self::ARTIFACT_COLUMNS
        );
        conn.query_row(&sql, params![request_id], Self::read_artifact)
            .optional()
            .map_err(store_err)
    }

    /// Edit title / body / category. Re-captures the identity of a referenced
    /// file but never touches the file itself.
    pub fn update_artifact(
        &self,
        expected_version: u64,
        patch: &Artifact,
    ) -> Result<Artifact, AppError> {
        let existing = self
            .get_artifact(&patch.id)?
            .ok_or(AppError::NotFound)?
            .artifact;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let title = clean_title(&patch.title)?;
        let (body, file_path, file_size, file_modified_at) = match existing.kind {
            ArtifactKind::Text => (Some(clean_body(patch.body.as_deref())?), None, None, None),
            ArtifactKind::File => {
                let path = patch
                    .file_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .ok_or(AppError::InvalidInput)?
                    .to_owned();
                let (size, modified) = file_fingerprint(&path);
                (None, Some(path), size, modified)
            }
        };
        let category = patch.category.trim().to_owned();
        {
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE artifacts SET title = ?2, body = ?3, file_path = ?4, file_size = ?5,
                     file_modified_at = ?6, category = ?7, version = version + 1, updated_at = ?8
                 WHERE id = ?1 AND version = ?9",
                params![
                    existing.id,
                    title,
                    body,
                    file_path,
                    file_size,
                    file_modified_at,
                    category,
                    now_unix(),
                    expected_version as i64,
                ],
            )
            .map_err(store_err)?;
        }
        self.get_artifact(&existing.id)?
            .map(|view| view.artifact)
            .ok_or(AppError::NotFound)
    }

    pub fn set_artifact_flags(
        &self,
        id: &str,
        expected_version: u64,
        favorite: Option<bool>,
        archived: Option<bool>,
    ) -> Result<Artifact, AppError> {
        let existing = self.get_artifact(id)?.ok_or(AppError::NotFound)?.artifact;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let favorite = favorite.unwrap_or(existing.favorite);
        let archived = archived.unwrap_or(existing.archived);
        {
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE artifacts SET favorite = ?2, archived = ?3, version = version + 1,
                     updated_at = ?4
                 WHERE id = ?1 AND version = ?5",
                params![
                    id,
                    favorite as i64,
                    archived as i64,
                    now_unix(),
                    expected_version as i64
                ],
            )
            .map_err(store_err)?;
        }
        self.get_artifact(id)?
            .map(|view| view.artifact)
            .ok_or(AppError::NotFound)
    }

    /// Remove the record. The referenced file is never deleted.
    pub fn delete_artifact(&self, id: &str, expected_version: u64) -> Result<(), AppError> {
        let existing = self.get_artifact(id)?.ok_or(AppError::NotFound)?.artifact;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let conn = self.conn.lock();
        conn.execute("DELETE FROM artifacts WHERE id = ?1", params![id])
            .map_err(store_err)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Job idempotency
    // ------------------------------------------------------------------

    /// Claim a one-shot receipt for a background action. Returns `true` when
    /// this caller is the first to claim `key`; `false` means the action
    /// already ran and must not be applied again.
    pub fn claim_receipt(&self, key: &str, job_type: &str) -> Result<bool, AppError> {
        let conn = self.conn.lock();
        let changed = conn
            .execute(
                "INSERT OR IGNORE INTO job_receipts (receipt_key, job_type, created_at)
                 VALUES (?1, ?2, ?3)",
                params![key, job_type, now_unix()],
            )
            .map_err(store_err)?;
        Ok(changed == 1)
    }
}

// ---------------------------------------------------------------------------
// Scheduler-driven app actions
// ---------------------------------------------------------------------------
//
// These reuse the existing job channel: a scheduled task of kind
// `TaskKind::Job` names one of these `job_type`s and passes the app action's
// parameters. Nothing here touches the scheduling engine or the agent
// execution contract, and none of it calls a model.
//
// Two rules hold for every handler:
//
//   * the app must be enabled *and* its scheduler link must be on, otherwise
//     the run is reported `Blocked` and writes nothing;
//   * the write goes through the same `AppStore` method the UI uses, so
//     validation, versioning and idempotency are shared rather than
//     reimplemented for the background path.

/// Params may carry an explicit idempotency key; otherwise the run id is the
/// key, so a retried delivery of the same run does not apply twice. A manual
/// run with neither has no key and is simply applied once.
fn receipt_key(job_type: &str, params: &serde_json::Value, ctx: &JobContext) -> Option<String> {
    if let Some(value) = params.get("requestId").and_then(|v| v.as_str()) {
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_owned());
        }
    }
    if ctx.run_id.is_empty() {
        return None;
    }
    Some(format!("{job_type}:{}", ctx.run_id))
}

fn blocked(reason: &str, summary: String) -> JobReport {
    JobReport {
        outcome: JobOutcome::Blocked,
        processed: 0,
        succeeded: 0,
        failed: 0,
        reason_code: Some(reason.into()),
        summary: Some(summary),
        report_ref: None,
    }
}

fn failed(reason: &str, summary: String) -> JobReport {
    JobReport {
        outcome: JobOutcome::Failed,
        processed: 0,
        succeeded: 0,
        failed: 1,
        reason_code: Some(reason.into()),
        summary: Some(summary),
        report_ref: None,
    }
}

fn applied(job_type: &str, app_id: &str, id: &str, first_delivery: bool) -> JobReport {
    JobReport {
        outcome: JobOutcome::Succeeded,
        processed: 1,
        succeeded: 1,
        failed: 0,
        reason_code: None,
        summary: Some(if first_delivery {
            format!("{app_id} {job_type} applied ({id})")
        } else {
            format!("{app_id} {job_type} already applied ({id})")
        }),
        report_ref: Some(id.to_owned()),
    }
}

/// Resolve the store the handler writes to: an injected one in tests, the
/// host's own workbench database in production.
fn resolve_store(injected: &Option<Arc<AppStore>>) -> Result<Arc<AppStore>, JobReport> {
    if let Some(store) = injected {
        return Ok(Arc::clone(store));
    }
    match AppStore::open_default() {
        Ok(store) => Ok(Arc::new(store)),
        Err(error) => Err(failed(
            "storage_unavailable",
            format!("workbench store unavailable: {error}"),
        )),
    }
}

/// Is the app allowed to act from the background right now?
fn background_guard(store: &AppStore, app_id: &str) -> Result<(), JobReport> {
    match store.app_enabled(app_id) {
        Ok(false) => {
            return Err(blocked(
                "app_disabled",
                format!("应用 {app_id} 已停用，跳过本次运行"),
            ))
        }
        Err(error) => {
            return Err(failed(
                "unknown_app",
                format!("应用 {app_id} 未注册: {error}"),
            ))
        }
        Ok(true) => {}
    }
    match store.app_schedule_enabled(app_id) {
        Ok(true) => Ok(()),
        Ok(false) => Err(blocked(
            "scheduler_link_off",
            format!("应用 {app_id} 未开启调度关联，跳过本次运行"),
        )),
        Err(error) => Err(failed(
            "unknown_app",
            format!("应用 {app_id} 未注册: {error}"),
        )),
    }
}

/// `app.todo.create` — create a todo from the scheduled task's params.
///
/// Params: `{ "title": "...", "notes"?, "priority"?, "dueDate"?,
/// "requestId"? }`. Only `title` is required, matching the in-app form.
pub struct AppTodoCreate {
    store: Option<Arc<AppStore>>,
}

impl AppTodoCreate {
    pub fn new() -> Self {
        Self { store: None }
    }

    pub fn with_store(store: Arc<AppStore>) -> Self {
        Self { store: Some(store) }
    }
}

impl Default for AppTodoCreate {
    fn default() -> Self {
        Self::new()
    }
}

impl Job for AppTodoCreate {
    fn job_type(&self) -> &str {
        "app.todo.create"
    }

    fn execute(&self, params: &serde_json::Value, ctx: &JobContext) -> JobReport {
        if ctx.cancel.is_cancelled() {
            return JobReport {
                outcome: JobOutcome::Cancelled,
                processed: 0,
                succeeded: 0,
                failed: 0,
                reason_code: Some("cancelled".into()),
                summary: Some("cancelled before start".into()),
                report_ref: None,
            };
        }
        let store = match resolve_store(&self.store) {
            Ok(store) => store,
            Err(report) => return report,
        };
        if let Err(report) = background_guard(&store, "todo") {
            return report;
        }

        let key = receipt_key(self.job_type(), params, ctx);
        let mut draft: TodoDraft = match serde_json::from_value(params.clone()) {
            Ok(draft) => draft,
            Err(error) => return failed("invalid_params", format!("参数无效: {error}")),
        };
        draft.source = Some("scheduled".into());
        draft.request_id = key.clone();

        let todo = match store.create_todo(&draft) {
            Ok(todo) => todo,
            Err(error) => {
                // A rejected write is reported, never silently dropped: an
                // invalid title must not look like a successful run.
                return failed("create_rejected", format!("{}", error));
            }
        };

        let first_delivery = match key {
            Some(key) => match store.claim_receipt(&key, self.job_type()) {
                Ok(first) => first,
                Err(error) => {
                    return failed("storage_unavailable", format!("{error}"));
                }
            },
            None => true,
        };
        applied(self.job_type(), "todo", &todo.id, first_delivery)
    }
}

/// `app.calendar.create_event` — add an event from the scheduled task's
/// params. Params mirror [`CalendarEventDraft`] plus an optional
/// `requestId`.
pub struct AppCalendarCreateEvent {
    store: Option<Arc<AppStore>>,
}

impl AppCalendarCreateEvent {
    pub fn new() -> Self {
        Self { store: None }
    }

    pub fn with_store(store: Arc<AppStore>) -> Self {
        Self { store: Some(store) }
    }
}

impl Default for AppCalendarCreateEvent {
    fn default() -> Self {
        Self::new()
    }
}

impl Job for AppCalendarCreateEvent {
    fn job_type(&self) -> &str {
        "app.calendar.create_event"
    }

    fn execute(&self, params: &serde_json::Value, ctx: &JobContext) -> JobReport {
        if ctx.cancel.is_cancelled() {
            return JobReport {
                outcome: JobOutcome::Cancelled,
                processed: 0,
                succeeded: 0,
                failed: 0,
                reason_code: Some("cancelled".into()),
                summary: Some("cancelled before start".into()),
                report_ref: None,
            };
        }
        let store = match resolve_store(&self.store) {
            Ok(store) => store,
            Err(report) => return report,
        };
        if let Err(report) = background_guard(&store, "calendar") {
            return report;
        }

        let key = receipt_key(self.job_type(), params, ctx);
        let mut draft: CalendarEventDraft = match serde_json::from_value(params.clone()) {
            Ok(draft) => draft,
            Err(error) => return failed("invalid_params", format!("参数无效: {error}")),
        };
        draft.request_id = key.clone();

        let event = match store.create_event(&draft) {
            Ok(event) => event,
            Err(error) => return failed("create_rejected", format!("{}", error)),
        };

        let first_delivery = match key {
            Some(key) => match store.claim_receipt(&key, self.job_type()) {
                Ok(first) => first,
                Err(error) => return failed("storage_unavailable", format!("{error}")),
            },
            None => true,
        };
        applied(self.job_type(), "calendar", &event.id, first_delivery)
    }
}

/// The app actions a scheduled task may trigger, in registration order.
pub fn app_job_handlers() -> Vec<Arc<dyn Job>> {
    vec![
        Arc::new(AppTodoCreate::new()),
        Arc::new(AppCalendarCreateEvent::new()),
    ]
}

fn resolve_file_status(artifact: Artifact) -> ArtifactView {
    if artifact.kind != ArtifactKind::File {
        return ArtifactView {
            artifact,
            file_status: ArtifactFileStatus::NotApplicable,
            current_size: None,
            current_modified_at: None,
        };
    }
    let Some(path) = artifact.file_path.as_deref() else {
        return ArtifactView {
            artifact,
            file_status: ArtifactFileStatus::Missing,
            current_size: None,
            current_modified_at: None,
        };
    };
    let (size, modified) = file_fingerprint(path);
    let status = match (size, artifact.file_size) {
        (None, _) => ArtifactFileStatus::Missing,
        (Some(current), Some(recorded))
            if current == recorded && modified == artifact.file_modified_at =>
        {
            ArtifactFileStatus::Ok
        }
        (Some(_), _) => ArtifactFileStatus::Changed,
    };
    ArtifactView {
        artifact,
        file_status: status,
        current_size: size,
        current_modified_at: modified,
    }
}

/// Every storage failure maps to one stable code; the driver's message goes
/// to stderr so a broken query is diagnosable from the session log.
fn store_err(error: rusqlite::Error) -> AppError {
    eprintln!("kodo workbench: storage error: {error}");
    AppError::StorageUnavailable
}

fn stmt<'c>(conn: &'c Connection, sql: &str) -> Result<rusqlite::Statement<'c>, AppError> {
    conn.prepare(sql).map_err(store_err)
}

fn collect<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>, AppError> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(store_err)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use tempfile::TempDir;

    fn store() -> (TempDir, AppStore) {
        let dir = TempDir::new().unwrap();
        let store = AppStore::open(dir.path()).expect("open store");
        (dir, store)
    }

    fn draft(title: &str) -> TodoDraft {
        TodoDraft {
            title: title.into(),
            ..Default::default()
        }
    }

    fn text_draft(title: &str, body: &str) -> ArtifactDraft {
        ArtifactDraft {
            title: title.into(),
            kind: ArtifactKind::Text,
            body: Some(body.into()),
            file_path: None,
            source: None,
            category: None,
            request_id: None,
        }
    }

    fn file_draft(title: &str, path: &Path) -> ArtifactDraft {
        ArtifactDraft {
            title: title.into(),
            kind: ArtifactKind::File,
            body: None,
            file_path: Some(path.to_string_lossy().into_owned()),
            source: None,
            category: None,
            request_id: None,
        }
    }

    #[test]
    fn builtin_apps_start_enabled_with_defaults() {
        let (_dir, store) = store();
        let apps = store.list_apps().unwrap();
        assert_eq!(apps.len(), 3);
        assert!(apps.iter().all(|a| a.preference.enabled));
        assert!(apps.iter().all(|a| !a.preference.favorite));
        // Default order follows the manifest order.
        assert_eq!(apps[0].manifest.app_id, "todo");
        assert_eq!(apps[1].manifest.app_id, "calendar");
        assert_eq!(apps[2].manifest.app_id, "artifacts");
    }

    #[test]
    fn favorites_sort_first_and_survive_reopen() {
        let dir = TempDir::new().unwrap();
        {
            let store = AppStore::open(dir.path()).unwrap();
            store.set_app_favorite("artifacts", true).unwrap();
        }
        let store = AppStore::open(dir.path()).unwrap();
        let apps = store.list_apps().unwrap();
        assert_eq!(apps[0].manifest.app_id, "artifacts");
        assert!(apps[0].preference.favorite);
    }

    #[test]
    fn capability_links_reject_unsupported_capabilities() {
        let (_dir, store) = store();
        // The artifacts app declares neither AI nor scheduler support.
        assert_eq!(
            store.set_app_links("artifacts", true, false),
            Err(AppError::InvalidInput)
        );
        assert_eq!(
            store.set_app_links("artifacts", false, true),
            Err(AppError::InvalidInput)
        );
        // The todo app supports both.
        let view = store.set_app_links("todo", true, true).unwrap();
        assert!(view.preference.ai_enabled && view.preference.schedule_enabled);
    }

    #[test]
    fn reordering_keeps_omitted_apps_in_the_list() {
        let (_dir, store) = store();
        let order = store.set_app_order(&["artifacts".to_string()]).unwrap();
        assert_eq!(order[0].manifest.app_id, "artifacts");
        // The apps the caller did not mention are still listed, after it.
        assert_eq!(order.len(), 3);
        assert_eq!(order[1].manifest.app_id, "todo");
        assert_eq!(order[2].manifest.app_id, "calendar");
        // A full reorder is honoured exactly.
        let order = store
            .set_app_order(&["calendar".into(), "todo".into(), "artifacts".into()])
            .unwrap();
        let ids: Vec<&str> = order.iter().map(|a| a.manifest.app_id.as_str()).collect();
        assert_eq!(ids, vec!["calendar", "todo", "artifacts"]);
        assert_eq!(
            store.set_app_order(&["nope".into()]),
            Err(AppError::UnknownApp)
        );
    }

    #[test]
    fn unknown_apps_are_rejected() {
        let (_dir, store) = store();
        assert_eq!(
            store.set_app_enabled("nope", true),
            Err(AppError::UnknownApp)
        );
    }

    #[test]
    fn prefs_round_trip_and_survive_reopen() {
        let dir = TempDir::new().unwrap();
        {
            let store = AppStore::open(dir.path()).unwrap();
            store.set_pref("showOverview", "false").unwrap();
        }
        let store = AppStore::open(dir.path()).unwrap();
        assert_eq!(
            store
                .list_prefs()
                .unwrap()
                .get("showOverview")
                .map(String::as_str),
            Some("false")
        );
        assert_eq!(store.set_pref("  ", "x"), Err(AppError::InvalidInput));
    }

    #[test]
    fn todo_requires_only_a_title() {
        let (_dir, store) = store();
        let todo = store.create_todo(&draft("买牛奶")).unwrap();
        assert_eq!(todo.title, "买牛奶");
        assert_eq!(todo.status, TodoStatus::Todo);
        assert_eq!(todo.priority, TodoPriority::Normal);
        assert!(todo.due_date.is_none());
        assert!(todo.completed_at.is_none());
        assert!(!todo.archived);
    }

    #[test]
    fn todo_create_is_idempotent_per_request_id() {
        let (_dir, store) = store();
        let mut d = draft("整理会议纪要");
        d.request_id = Some("req-1".into());
        let first = store.create_todo(&d).unwrap();
        let second = store.create_todo(&d).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(store.list_todos(false).unwrap().len(), 1);
    }

    #[test]
    fn todo_edits_check_the_version() {
        let (_dir, store) = store();
        let todo = store.create_todo(&draft("写周报")).unwrap();
        let mut patch = todo.clone();
        patch.title = "写周报（终稿）".into();
        let updated = store.update_todo(todo.version, &patch).unwrap();
        assert_eq!(updated.version, todo.version + 1);
        // A stale write is refused rather than overwriting the newer edit.
        assert_eq!(
            store.update_todo(todo.version, &patch),
            Err(AppError::RevisionConflict)
        );
    }

    #[test]
    fn todo_status_is_the_single_source_of_truth() {
        let (_dir, store) = store();
        let todo = store.create_todo(&draft("确认排期")).unwrap();
        let done = store
            .set_todo_status(&todo.id, todo.version, TodoStatus::Done)
            .unwrap();
        assert_eq!(done.status, TodoStatus::Done);
        assert!(done.completed_at.is_some());
        // Restoring clears the completion stamp and returns to 待处理.
        let restored = store
            .set_todo_status(&todo.id, done.version, TodoStatus::Todo)
            .unwrap();
        assert_eq!(restored.status, TodoStatus::Todo);
        assert!(restored.completed_at.is_none());
    }

    #[test]
    fn todo_status_write_needs_the_current_version() {
        let (_dir, store) = store();
        let todo = store.create_todo(&draft("a")).unwrap();
        store
            .set_todo_status(&todo.id, todo.version, TodoStatus::Doing)
            .unwrap();
        assert_eq!(
            store.set_todo_status(&todo.id, todo.version, TodoStatus::Done),
            Err(AppError::RevisionConflict)
        );
    }

    #[test]
    fn archived_todos_are_hidden_but_kept() {
        let (_dir, store) = store();
        let todo = store.create_todo(&draft("归档我")).unwrap();
        store
            .set_todo_archived(&todo.id, todo.version, true)
            .unwrap();
        assert!(store.list_todos(false).unwrap().is_empty());
        assert_eq!(store.list_todos(true).unwrap().len(), 1);
        assert_eq!(store.get_todo(&todo.id).unwrap().unwrap().title, "归档我");
    }

    #[test]
    fn invalid_due_dates_are_rejected() {
        let (_dir, store) = store();
        let mut d = draft("x");
        d.due_date = Some("2026-02-30".into());
        assert_eq!(store.create_todo(&d), Err(AppError::InvalidInput));
    }

    #[test]
    fn events_need_an_end_after_the_start() {
        let (_dir, store) = store();
        let base = CalendarEventDraft {
            title: "产品方案讨论".into(),
            date: "2026-10-13".into(),
            start_time: Some("10:00".into()),
            end_time: Some("11:00".into()),
            all_day: Some(false),
            timezone: Some("Asia/Shanghai".into()),
            ..Default::default()
        };
        let event = store.create_event(&base).unwrap();
        assert_eq!(event.timezone, "Asia/Shanghai");
        assert_eq!(event.start_time.as_deref(), Some("10:00"));

        let reversed = CalendarEventDraft {
            start_time: Some("11:00".into()),
            end_time: Some("10:00".into()),
            ..base.clone()
        };
        assert_eq!(store.create_event(&reversed), Err(AppError::InvalidInput));
    }

    #[test]
    fn all_day_events_drop_the_clock() {
        let (_dir, store) = store();
        let event = store
            .create_event(&CalendarEventDraft {
                title: "团队日".into(),
                date: "2026-10-15".into(),
                all_day: Some(true),
                timezone: Some("Asia/Shanghai".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(event.all_day);
        assert!(event.start_time.is_none() && event.end_time.is_none());
    }

    #[test]
    fn events_reject_an_unknown_timezone() {
        let (_dir, store) = store();
        assert_eq!(
            store.create_event(&CalendarEventDraft {
                title: "x".into(),
                date: "2026-10-13".into(),
                start_time: Some("10:00".into()),
                end_time: Some("11:00".into()),
                timezone: Some("Mars/Olympus".into()),
                ..Default::default()
            }),
            Err(AppError::InvalidInput)
        );
    }

    #[test]
    fn deleting_an_event_checks_the_version() {
        let (_dir, store) = store();
        let event = store
            .create_event(&CalendarEventDraft {
                title: "周会".into(),
                date: "2026-10-13".into(),
                start_time: Some("09:00".into()),
                end_time: Some("09:30".into()),
                ..Default::default()
            })
            .unwrap();
        let mut patch = event.clone();
        patch.title = "周会（改）".into();
        store.update_event(event.version, &patch).unwrap();
        assert_eq!(
            store.delete_event(&event.id, event.version),
            Err(AppError::RevisionConflict)
        );
    }

    #[test]
    fn text_artifacts_need_a_body() {
        let (_dir, store) = store();
        let err = store
            .create_artifact(&text_draft("摘录", "   "))
            .unwrap_err();
        assert_eq!(err, AppError::InvalidInput);

        let saved = store
            .create_artifact(&text_draft("摘录", "工作台提供统一入口。"))
            .unwrap();
        assert_eq!(saved.body.as_deref(), Some("工作台提供统一入口。"));
    }

    #[test]
    fn file_artifacts_report_missing_and_changed() {
        let dir = TempDir::new().unwrap();
        let store = AppStore::open(dir.path()).unwrap();
        let target = dir.path().join("成果.md");
        std::fs::write(&target, b"v1").unwrap();

        let artifact = store
            .create_artifact(&file_draft("成果文件", &target))
            .unwrap();

        let view = store.get_artifact(&artifact.id).unwrap().unwrap();
        assert_eq!(view.file_status, ArtifactFileStatus::Ok);

        // A size change is reported without touching the record.
        std::fs::write(&target, b"v2 with more bytes").unwrap();
        let view = store.get_artifact(&artifact.id).unwrap().unwrap();
        assert_eq!(view.file_status, ArtifactFileStatus::Changed);
        assert_eq!(view.current_size, Some(18));

        // Removing the file reports Missing; the record still exists.
        std::fs::remove_file(&target).unwrap();
        let view = store.get_artifact(&artifact.id).unwrap().unwrap();
        assert_eq!(view.file_status, ArtifactFileStatus::Missing);
    }

    #[test]
    fn removing_an_artifact_never_deletes_the_file() {
        let dir = TempDir::new().unwrap();
        let store = AppStore::open(dir.path()).unwrap();
        let target = dir.path().join("keep.md");
        std::fs::write(&target, b"keep me").unwrap();
        let artifact = store.create_artifact(&file_draft("保留", &target)).unwrap();
        store
            .delete_artifact(&artifact.id, artifact.version)
            .unwrap();
        assert!(target.exists());
        assert!(store.get_artifact(&artifact.id).unwrap().is_none());
    }

    #[test]
    fn only_a_stored_file_reference_can_be_handed_to_the_os() {
        let dir = TempDir::new().unwrap();
        let store = AppStore::open(dir.path()).unwrap();
        let target = dir.path().join("report.md");
        std::fs::write(&target, b"hello").unwrap();
        let file = store.create_artifact(&file_draft("报告", &target)).unwrap();
        assert_eq!(
            store.artifact_file_path(&file.id).unwrap(),
            target.to_string_lossy()
        );
        // A text artifact has no file behind it, and an unknown id has no
        // record: neither may ever reach the OS opener.
        let text = store.create_artifact(&text_draft("笔记", "内容")).unwrap();
        assert!(matches!(
            store.artifact_file_path(&text.id),
            Err(AppError::InvalidInput)
        ));
        assert!(matches!(
            store.artifact_file_path("missing"),
            Err(AppError::NotFound)
        ));
    }

    #[test]
    fn artifact_favorite_and_archive_flags_bump_the_version() {
        let (_dir, store) = store();
        let artifact = store.create_artifact(&text_draft("文本", "内容")).unwrap();
        let starred = store
            .set_artifact_flags(&artifact.id, artifact.version, Some(true), None)
            .unwrap();
        assert!(starred.favorite && starred.version == artifact.version + 1);
        let archived = store
            .set_artifact_flags(&artifact.id, starred.version, None, Some(true))
            .unwrap();
        assert!(archived.archived);
        assert!(store.list_artifacts(false).unwrap().is_empty());
        assert_eq!(store.list_artifacts(true).unwrap().len(), 1);
    }

    fn job_ctx(run_id: &str) -> JobContext {
        JobContext {
            run_id: run_id.into(),
            deadline: None,
            cancel: crate::scheduler::JobCancellation::new(),
        }
    }

    #[test]
    fn scheduled_actions_are_blocked_until_the_link_is_on() {
        let (_dir, store) = store();
        let store = Arc::new(store);
        let job = AppTodoCreate::with_store(Arc::clone(&store));
        let params = serde_json::json!({ "title": "定时创建的待办" });

        // Defaults: the app is enabled but its scheduler link is off.
        let report = job.execute(&params, &job_ctx("run-1"));
        assert_eq!(report.outcome, JobOutcome::Blocked);
        assert_eq!(report.reason_code.as_deref(), Some("scheduler_link_off"));
        assert!(store.list_todos(false).unwrap().is_empty());

        // Disabled app: still nothing written.
        store.set_app_links("todo", false, true).unwrap();
        store.set_app_enabled("todo", false).unwrap();
        let report = job.execute(&params, &job_ctx("run-2"));
        assert_eq!(report.outcome, JobOutcome::Blocked);
        assert_eq!(report.reason_code.as_deref(), Some("app_disabled"));
        assert!(store.list_todos(false).unwrap().is_empty());

        // Turned on: the action runs and writes through the normal path.
        store.set_app_enabled("todo", true).unwrap();
        let report = job.execute(&params, &job_ctx("run-3"));
        assert_eq!(report.outcome, JobOutcome::Succeeded);
        let todos = store.list_todos(false).unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].source, "scheduled");
        assert_eq!(todos[0].status, TodoStatus::Todo);
    }

    #[test]
    fn scheduled_todo_create_is_idempotent_per_run() {
        let (_dir, store) = store();
        let store = Arc::new(store);
        store.set_app_links("todo", false, true).unwrap();
        let job = AppTodoCreate::with_store(Arc::clone(&store));
        let params = serde_json::json!({ "title": "每日站会", "dueDate": "2026-10-14" });

        let first = job.execute(&params, &job_ctx("run-42"));
        assert_eq!(first.outcome, JobOutcome::Succeeded);
        assert!(first.summary.unwrap().contains("applied"));

        // A retried delivery of the same run must not add a second todo.
        let second = job.execute(&params, &job_ctx("run-42"));
        assert_eq!(second.outcome, JobOutcome::Succeeded);
        assert!(second.summary.unwrap().contains("already applied"));

        let todos = store.list_todos(false).unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].due_date.as_deref(), Some("2026-10-14"));
    }

    #[test]
    fn a_rejected_scheduled_action_is_reported_not_silently_dropped() {
        let (_dir, store) = store();
        let store = Arc::new(store);
        store.set_app_links("todo", false, true).unwrap();
        let job = AppTodoCreate::with_store(Arc::clone(&store));

        let report = job.execute(&serde_json::json!({ "title": "   " }), &job_ctx("run-9"));
        assert_eq!(report.outcome, JobOutcome::Failed);
        assert_eq!(report.reason_code.as_deref(), Some("create_rejected"));
        assert!(store.list_todos(false).unwrap().is_empty());
    }

    #[test]
    fn scheduled_calendar_event_is_created() {
        let (_dir, store) = store();
        let store = Arc::new(store);
        store.set_app_links("calendar", false, true).unwrap();
        let job = AppCalendarCreateEvent::with_store(Arc::clone(&store));

        let params = serde_json::json!({
            "title": "每周复盘",
            "date": "2026-10-16",
            "startTime": "16:00",
            "endTime": "17:00",
        });
        let report = job.execute(&params, &job_ctx("run-cal"));
        assert_eq!(report.outcome, JobOutcome::Succeeded);
        let events = store.list_events().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "每周复盘");
        assert_eq!(events[0].start_time.as_deref(), Some("16:00"));

        // The calendar app declares no AI link, so the scheduler is the only
        // integration it may switch on.
        assert_eq!(
            store.set_app_links("calendar", true, true),
            Err(AppError::InvalidInput)
        );
    }

    #[test]
    fn app_actions_are_the_last_registered_handlers() {
        let types: Vec<String> = crate::scheduler_jobs::default_job_registry_handlers()
            .iter()
            .map(|job| job.job_type().to_owned())
            .collect();
        assert_eq!(
            types,
            vec![
                "workspace_refresh",
                "memory_outbox_replay",
                "app.todo.create",
                "app.calendar.create_event",
            ]
        );
    }

    /// The registered handlers and the per-app job-type list must not drift:
    /// turning an app off pauses exactly the tasks that can drive it, and a
    /// paused task that still fires must find a handler.
    #[test]
    fn registered_app_handlers_match_the_declared_job_types() {
        let registered: Vec<String> = app_job_handlers()
            .iter()
            .map(|job| job.job_type().to_owned())
            .collect();
        let declared: Vec<String> = builtin_apps()
            .iter()
            .flat_map(|app| kodo_core::apps::app_job_types(&app.app_id))
            .map(|t| (*t).to_owned())
            .collect();
        assert_eq!(registered, declared);
    }

    #[test]
    fn receipts_are_claimed_once() {
        let (_dir, store) = store();
        assert!(store.claim_receipt("run:1", "app.todo.create").unwrap());
        assert!(!store.claim_receipt("run:1", "app.todo.create").unwrap());
        assert!(store.claim_receipt("run:2", "app.todo.create").unwrap());
    }

    #[test]
    fn reopening_the_store_keeps_every_app_dataset() {
        let dir = TempDir::new().unwrap();
        {
            let store = AppStore::open(dir.path()).unwrap();
            store.create_todo(&draft("持久化待办")).unwrap();
            store
                .create_event(&CalendarEventDraft {
                    title: "持久化日程".into(),
                    date: "2026-10-13".into(),
                    start_time: Some("09:00".into()),
                    end_time: Some("10:00".into()),
                    ..Default::default()
                })
                .unwrap();
            store
                .create_artifact(&text_draft("持久化成果", "内容"))
                .unwrap();
            store.set_app_enabled("calendar", false).unwrap();
        }
        let store = AppStore::open(dir.path()).unwrap();
        assert_eq!(store.list_todos(false).unwrap().len(), 1);
        assert_eq!(store.list_events().unwrap().len(), 1);
        assert_eq!(store.list_artifacts(false).unwrap().len(), 1);
        // Disabling an app keeps its data and stays disabled after reopen.
        assert!(!store.app_enabled("calendar").unwrap());
        let apps = store.list_apps().unwrap();
        let calendar = apps
            .iter()
            .find(|a| a.manifest.app_id == "calendar")
            .unwrap();
        assert!(!calendar.preference.enabled);
    }
}
