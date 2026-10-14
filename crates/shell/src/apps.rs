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
    builtin_apps, clean_body, clean_color, clean_id, clean_id_list, clean_item_type, clean_title,
    system_timezone, validate_date, validate_hours, validate_time, validate_timezone, AppError,
    AppManifest, AppPreference, AppView, Artifact, ArtifactDraft, ArtifactFileStatus, ArtifactKind,
    ArtifactView, CalendarEvent, CalendarEventDraft, Todo, TodoActivity, TodoAttachment,
    TodoAttachmentDraft, TodoBulkOp, TodoDraft, TodoPerson, TodoPriority, TodoReminder,
    TodoReminderDraft, TodoReminderKind, TodoStatusDef, TodoStatusDraft, TodoTag, TodoTagDraft,
    DEFAULT_TODO_STATUS, MAX_ID_LIST, MAX_TODO_TAGS,
};

use crate::scheduler::{Job, JobContext, JobOutcome, JobReport};

const MIGRATION_0001: &str = include_str!("../migrations/workbench_0001.sql");
const MIGRATION_0002: &str = include_str!("../migrations/workbench_0002.sql");

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

/// Wall-clock milliseconds since the Unix epoch. The frontend reads this as
/// `epochMs` (`new Date(n)`, `Date.parse(...)` are all in milliseconds) and
/// `DEMO_TODOS`' `at(...)` helper follows the same unit, so the round-trip
/// across the IPC stays in one scale. The scheduler is the one place that
/// genuinely wants whole seconds, and it goes through `Clock::now_unix`
/// instead of this helper.
pub fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
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

        let migrations: &[(i64, &str, &str)] = &[
            (1, MIGRATION_0001, "workbench v1"),
            (2, MIGRATION_0002, "workbench v2"),
        ];
        for (version, sql, description) in migrations {
            if applied.contains(version) {
                continue;
            }
            let tx = conn.transaction().map_err(store_err)?;
            tx.execute_batch(sql).map_err(store_err)?;
            tx.execute(
                "INSERT OR REPLACE INTO schema_migrations (version, applied_at, description)
                 VALUES (?1, ?2, ?3)",
                params![version, now_unix_ms(), description],
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
        pref.updated_at = now_unix_ms();
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
        pref.updated_at = now_unix_ms();
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
        pref.updated_at = now_unix_ms();
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
        let now = now_unix_ms();
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
            params![key.trim(), value, now_unix_ms()],
        )
        .map_err(store_err)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Todos
    // ------------------------------------------------------------------

    fn read_todo(row: &Row<'_>) -> rusqlite::Result<Todo> {
        let priority: String = row.get("priority")?;
        Ok(Todo {
            id: row.get("id")?,
            title: row.get("title")?,
            notes: row.get("notes")?,
            status: row.get("status")?,
            priority: TodoPriority::parse(&priority).unwrap_or(TodoPriority::Normal),
            due_date: row.get("due_date")?,
            due_time: row.get("due_time")?,
            parent_id: row.get("parent_id")?,
            assignee_id: row.get("assignee_id")?,
            item_type: row.get("item_type")?,
            module: row.get("module")?,
            estimate_hours: row.get("estimate_hours")?,
            actual_hours: row.get("actual_hours")?,
            archived: row.get::<_, i64>("archived")? != 0,
            completed_at: row.get("completed_at")?,
            sort_order: row.get("sort_order")?,
            source: row.get("source")?,
            version: row.get("version")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            // Filled by `attach_todo_links`; a row read on its own carries the
            // honest empties rather than a half-populated answer.
            tag_ids: Vec::new(),
            link_ids: Vec::new(),
            is_done: false,
        })
    }

    fn read_status(row: &Row<'_>) -> rusqlite::Result<TodoStatusDef> {
        Ok(TodoStatusDef {
            id: row.get("id")?,
            name: row.get("name")?,
            color: row.get("color")?,
            is_done: row.get::<_, i64>("is_done")? != 0,
            builtin: row.get::<_, i64>("builtin")? != 0,
            sort_order: row.get("sort_order")?,
            version: row.get("version")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }

    fn read_tag(row: &Row<'_>) -> rusqlite::Result<TodoTag> {
        Ok(TodoTag {
            id: row.get("id")?,
            name: row.get("name")?,
            color: row.get("color")?,
            sort_order: row.get("sort_order")?,
            version: row.get("version")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }

    fn read_person(row: &Row<'_>) -> rusqlite::Result<TodoPerson> {
        Ok(TodoPerson {
            id: row.get("id")?,
            name: row.get("name")?,
            color: row.get("color")?,
            created_at: row.get("created_at")?,
        })
    }

    fn read_attachment(row: &Row<'_>) -> rusqlite::Result<TodoAttachment> {
        Ok(TodoAttachment {
            id: row.get("id")?,
            todo_id: row.get("todo_id")?,
            name: row.get("name")?,
            path: row.get("path")?,
            size: row.get("size")?,
            mime: row.get("mime")?,
            version: row.get("version")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }

    fn read_reminder(row: &Row<'_>) -> rusqlite::Result<TodoReminder> {
        let kind: String = row.get("kind")?;
        Ok(TodoReminder {
            id: row.get("id")?,
            todo_id: row.get("todo_id")?,
            kind: TodoReminderKind::parse(&kind).unwrap_or_default(),
            offset_minutes: row.get("offset_minutes")?,
            message: row.get("message")?,
            enabled: row.get::<_, i64>("enabled")? != 0,
            version: row.get("version")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }

    fn read_activity(row: &Row<'_>) -> rusqlite::Result<TodoActivity> {
        Ok(TodoActivity {
            id: row.get("id")?,
            todo_id: row.get("todo_id")?,
            kind: row.get("kind")?,
            author_id: row.get("author_id")?,
            body: row.get("body")?,
            meta: row.get("meta")?,
            created_at: row.get("created_at")?,
        })
    }

    const TODO_COLUMNS: &'static str = "id, title, notes, status, priority, due_date, due_time, \
         parent_id, assignee_id, item_type, module, estimate_hours, actual_hours, archived, \
         completed_at, sort_order, source, version, created_at, updated_at";

    const STATUS_COLUMNS: &'static str = "id, name, color, is_done, builtin, sort_order, version, \
         created_at, updated_at";

    const TAG_COLUMNS: &'static str =
        "id, name, color, sort_order, version, created_at, updated_at";

    const ATTACHMENT_COLUMNS: &'static str = "id, todo_id, name, path, size, mime, version, \
         created_at, updated_at";

    const REMINDER_COLUMNS: &'static str = "id, todo_id, kind, offset_minutes, message, enabled, \
         version, created_at, updated_at";

    const ACTIVITY_COLUMNS: &'static str = "id, todo_id, kind, author_id, body, meta, created_at";

    /// The status catalog keyed by id. The one place `is_done` is resolved, and
    /// the thing every status write validates against.
    fn status_map(conn: &Connection) -> Result<BTreeMap<String, TodoStatusDef>, AppError> {
        let sql = format!("SELECT {} FROM todo_statuses", Self::STATUS_COLUMNS);
        let mut stmt = stmt(conn, &sql)?;
        let rows = stmt.query_map([], Self::read_status).map_err(store_err)?;
        let mut map = BTreeMap::new();
        for status in collect(rows)? {
            map.insert(status.id.clone(), status);
        }
        Ok(map)
    }

    /// Reject a status id the catalog does not have. `todos.status` carries no
    /// foreign key — SQLite cannot add one to an existing table without the
    /// 12-step rebuild — so this check is the only thing between a stale client
    /// and a row pointing at a status that no longer exists.
    fn check_status(
        statuses: &BTreeMap<String, TodoStatusDef>,
        id: &str,
    ) -> Result<String, AppError> {
        let id = clean_id(id)?;
        if !statuses.contains_key(&id) {
            return Err(AppError::NotFound);
        }
        Ok(id)
    }

    fn status_is_done(statuses: &BTreeMap<String, TodoStatusDef>, id: &str) -> bool {
        statuses.get(id).is_some_and(|status| status.is_done)
    }

    /// The `completed_at` a write should leave behind. Callers pass the target
    /// status's `is_done` and the row's previous one: entering done stamps the
    /// moment, staying done keeps the original, and leaving done clears it.
    fn completed_at_for(
        now: i64,
        done: bool,
        was_done: bool,
        previous: Option<i64>,
    ) -> Option<i64> {
        match (done, was_done) {
            (false, _) => None,
            (true, true) => previous.or(Some(now)),
            (true, false) => Some(now),
        }
    }

    /// Fill in the three fields that live outside `todos` — `is_done` from the
    /// status catalog, `tag_ids` and `link_ids` — in a constant number of
    /// queries rather than one per row.
    fn attach_todo_links(conn: &Connection, rows: &mut [Todo]) -> Result<(), AppError> {
        if rows.is_empty() {
            return Ok(());
        }
        let statuses = Self::status_map(conn)?;
        for row in rows.iter_mut() {
            row.is_done = Self::status_is_done(&statuses, &row.status);
        }

        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        let list = placeholders(1, ids.len());

        let mut tags: BTreeMap<String, Vec<String>> = BTreeMap::new();
        {
            // In the catalog's own order, so the chips render stably.
            let sql = format!(
                "SELECT l.todo_id, l.tag_id FROM todo_tag_links l
                   JOIN todo_tags t ON t.id = l.tag_id
                  WHERE l.todo_id IN ({list})
                  ORDER BY t.sort_order ASC, t.name ASC"
            );
            let mut stmt = stmt(conn, &sql)?;
            let found = stmt
                .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(store_err)?;
            for entry in found {
                let (todo_id, tag_id) = entry.map_err(store_err)?;
                tags.entry(todo_id).or_default().push(tag_id);
            }
        }

        let mut links: BTreeMap<String, Vec<String>> = BTreeMap::new();
        {
            // Both stored directions, folded into one list per todo: a link
            // written from either side shows on both.
            let tail = placeholders(1 + ids.len(), ids.len());
            let sql = format!(
                "SELECT todo_id, related_id FROM todo_links
                  WHERE todo_id IN ({list}) OR related_id IN ({tail})"
            );
            let mut stmt = stmt(conn, &sql)?;
            let both = ids.iter().chain(ids.iter());
            let found = stmt
                .query_map(rusqlite::params_from_iter(both), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(store_err)?;
            for entry in found {
                let (todo_id, related_id) = entry.map_err(store_err)?;
                links
                    .entry(todo_id.clone())
                    .or_default()
                    .push(related_id.clone());
                links.entry(related_id).or_default().push(todo_id);
            }
        }

        for row in rows.iter_mut() {
            row.tag_ids = tags.remove(&row.id).unwrap_or_default();
            row.link_ids = links.remove(&row.id).unwrap_or_default();
        }
        Ok(())
    }

    /// A todo has to exist before a sub-item or a link may name it.
    fn todo_exists(&self, id: &str) -> Result<(), AppError> {
        if self.get_todo(id)?.is_none() {
            return Err(AppError::NotFound);
        }
        Ok(())
    }

    fn todo_tag_ids_exist(&self, ids: &[String]) -> Result<(), AppError> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT COUNT(*) FROM todo_tags WHERE id IN ({})",
            placeholders(1, ids.len())
        );
        let found: i64 = conn
            .query_row(&sql, rusqlite::params_from_iter(ids.iter()), |row| {
                row.get(0)
            })
            .map_err(store_err)?;
        if found as usize != ids.len() {
            return Err(AppError::NotFound);
        }
        Ok(())
    }

    fn todo_person_exists(&self, id: &str) -> Result<(), AppError> {
        let conn = self.conn.lock();
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM todo_people WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(store_err)?;
        if found == 0 {
            return Err(AppError::NotFound);
        }
        Ok(())
    }

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
        let mut todos = collect(rows)?;
        Self::attach_todo_links(&conn, &mut todos)?;
        Ok(todos)
    }

    pub fn get_todo(&self, id: &str) -> Result<Option<Todo>, AppError> {
        let conn = self.conn.lock();
        let sql = format!("SELECT {} FROM todos WHERE id = ?1", Self::TODO_COLUMNS);
        let mut todo = conn
            .query_row(&sql, params![id], Self::read_todo)
            .optional()
            .map_err(store_err)?;
        if let Some(row) = todo.as_mut() {
            Self::attach_todo_links(&conn, std::slice::from_mut(row))?;
        }
        Ok(todo)
    }

    /// Create a todo. Only `title` is required. A repeated `request_id`
    /// returns the todo created the first time.
    pub fn create_todo(&self, draft: &TodoDraft) -> Result<Todo, AppError> {
        let title = clean_title(&draft.title)?;
        let notes = clean_body(draft.notes.as_deref())?;
        let priority = draft.priority.unwrap_or(TodoPriority::Normal);
        let due_date = match draft.due_date.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => Some(validate_date(value)?),
            _ => None,
        };
        let due_time = match draft.due_time.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => Some(validate_time(value)?),
            _ => None,
        };
        let parent_id = match draft.parent_id.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => {
                self.todo_exists(value)?;
                Some(value.to_owned())
            }
            _ => None,
        };
        let assignee_id = match draft.assignee_id.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => {
                self.todo_person_exists(value)?;
                Some(value.to_owned())
            }
            _ => None,
        };
        let item_type = clean_item_type(draft.item_type.as_deref())?;
        let module = draft.module.as_deref().unwrap_or("").trim().to_owned();
        let estimate_hours = validate_hours(draft.estimate_hours)?;
        let actual_hours = validate_hours(draft.actual_hours)?;
        let tag_ids = clean_id_list(draft.tag_ids.as_deref().unwrap_or(&[]), MAX_TODO_TAGS)?;
        self.todo_tag_ids_exist(&tag_ids)?;
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

        let now = now_unix_ms();
        let sort_order = self.next_sort_order()?;
        let id = new_id("todo");
        let inserted = {
            let mut conn = self.conn.lock();
            let statuses = Self::status_map(&conn)?;
            let status = match draft
                .status
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                Some(value) => Self::check_status(&statuses, value)?,
                None => DEFAULT_TODO_STATUS.to_owned(),
            };
            let completed_at =
                Self::completed_at_for(now, Self::status_is_done(&statuses, &status), false, None);
            let tx = conn.transaction().map_err(store_err)?;
            tx.execute(
                "INSERT INTO todos
                     (id, title, notes, status, priority, due_date, due_time, parent_id,
                      assignee_id, item_type, module, estimate_hours, actual_hours, archived,
                      completed_at, sort_order, source, version, created_at, updated_at, request_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0, ?14, ?15, ?16,
                         1, ?17, ?17, ?18)
                 ON CONFLICT(request_id) WHERE request_id IS NOT NULL DO NOTHING",
                params![
                    id,
                    title,
                    notes,
                    status,
                    priority.as_str(),
                    due_date,
                    due_time,
                    parent_id,
                    assignee_id,
                    item_type,
                    module,
                    estimate_hours,
                    actual_hours,
                    completed_at,
                    sort_order,
                    source,
                    now,
                    request_id,
                ],
            )
            .map_err(store_err)?;
            // A lost race inserted nothing, so it must not link tags onto the
            // row that won.
            let inserted = tx.changes() > 0;
            if inserted {
                for tag_id in &tag_ids {
                    tx.execute(
                        "INSERT OR IGNORE INTO todo_tag_links (todo_id, tag_id) VALUES (?1, ?2)",
                        params![id, tag_id],
                    )
                    .map_err(store_err)?;
                }
            }
            tx.commit().map_err(store_err)?;
            inserted
        };

        if inserted {
            return self.get_todo(&id)?.ok_or(AppError::NotFound);
        }
        // A concurrent create with the same request_id won the unique index:
        // return that row so both callers see the same todo.
        match request_id {
            Some(key) => self.todo_by_request(&key)?.ok_or(AppError::NotFound),
            None => Err(AppError::NotFound),
        }
    }

    fn todo_by_request(&self, request_id: &str) -> Result<Option<Todo>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM todos WHERE request_id = ?1",
            Self::TODO_COLUMNS
        );
        let mut todo = conn
            .query_row(&sql, params![request_id], Self::read_todo)
            .optional()
            .map_err(store_err)?;
        if let Some(row) = todo.as_mut() {
            Self::attach_todo_links(&conn, std::slice::from_mut(row))?;
        }
        Ok(todo)
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
        let due_time = match patch.due_time.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => Some(validate_time(value)?),
            _ => None,
        };
        let parent_id = match patch.parent_id.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => {
                if value == existing.id {
                    return Err(AppError::InvalidInput);
                }
                self.todo_exists(value)?;
                Some(value.to_owned())
            }
            _ => None,
        };
        let assignee_id = match patch.assignee_id.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => {
                self.todo_person_exists(value)?;
                Some(value.to_owned())
            }
            _ => None,
        };
        let item_type = clean_item_type(Some(&patch.item_type))?;
        let module = patch.module.trim().to_owned();
        let estimate_hours = validate_hours(patch.estimate_hours)?;
        let actual_hours = validate_hours(patch.actual_hours)?;
        let tag_ids = clean_id_list(&patch.tag_ids, MAX_TODO_TAGS)?;
        self.todo_tag_ids_exist(&tag_ids)?;
        let tags_changed = tag_ids != existing.tag_ids;
        let now = now_unix_ms();
        {
            let mut conn = self.conn.lock();
            let statuses = Self::status_map(&conn)?;
            let status = Self::check_status(&statuses, &patch.status)?;
            let completed_at = Self::completed_at_for(
                now,
                Self::status_is_done(&statuses, &status),
                existing.is_done,
                existing.completed_at,
            );
            let tx = conn.transaction().map_err(store_err)?;
            tx.execute(
                "UPDATE todos SET title = ?2, notes = ?3, status = ?4, priority = ?5,
                     due_date = ?6, due_time = ?7, parent_id = ?8, assignee_id = ?9,
                     item_type = ?10, module = ?11, estimate_hours = ?12, actual_hours = ?13,
                     completed_at = ?14, version = version + 1, updated_at = ?15
                 WHERE id = ?1 AND version = ?16",
                params![
                    existing.id,
                    title,
                    notes,
                    status,
                    patch.priority.as_str(),
                    due_date,
                    due_time,
                    parent_id,
                    assignee_id,
                    item_type,
                    module,
                    estimate_hours,
                    actual_hours,
                    completed_at,
                    now,
                    expected_version as i64,
                ],
            )
            .map_err(store_err)?;
            if tags_changed {
                tx.execute(
                    "DELETE FROM todo_tag_links WHERE todo_id = ?1",
                    params![existing.id],
                )
                .map_err(store_err)?;
                for tag_id in &tag_ids {
                    tx.execute(
                        "INSERT OR IGNORE INTO todo_tag_links (todo_id, tag_id) VALUES (?1, ?2)",
                        params![existing.id, tag_id],
                    )
                    .map_err(store_err)?;
                }
            }
            tx.commit().map_err(store_err)?;
        }
        self.get_todo(&existing.id)?.ok_or(AppError::NotFound)
    }

    /// Move a todo between board columns. This is the same write path the
    /// drag, the status select and the edit form all use.
    pub fn set_todo_status(
        &self,
        id: &str,
        expected_version: u64,
        status: &str,
    ) -> Result<Todo, AppError> {
        let existing = self.get_todo(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let now = now_unix_ms();
        {
            let conn = self.conn.lock();
            let statuses = Self::status_map(&conn)?;
            let status = Self::check_status(&statuses, status)?;
            let completed_at = Self::completed_at_for(
                now,
                Self::status_is_done(&statuses, &status),
                existing.is_done,
                existing.completed_at,
            );
            conn.execute(
                "UPDATE todos SET status = ?2, completed_at = ?3, version = version + 1,
                     updated_at = ?4
                 WHERE id = ?1 AND version = ?5",
                params![id, status, completed_at, now, expected_version as i64],
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
                params![id, archived as i64, now_unix_ms(), expected_version as i64],
            )
            .map_err(store_err)?;
        }
        self.get_todo(id)?.ok_or(AppError::NotFound)
    }

    /// Delete a todo together with its sub-items.
    ///
    /// `todos.parent_id` is `ON DELETE SET NULL` (0001), so a plain DELETE
    /// would *promote* the children to top-level rows instead of removing them.
    /// The subtree is walked explicitly for that reason — one level, which is
    /// as deep as the UI nests.
    pub fn delete_todo(&self, id: &str, expected_version: u64) -> Result<(), AppError> {
        let existing = self.get_todo(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let conn = self.conn.lock();
        conn.execute("DELETE FROM todos WHERE parent_id = ?1", params![id])
            .map_err(store_err)?;
        conn.execute(
            "DELETE FROM todos WHERE id = ?1 AND version = ?2",
            params![id, expected_version as i64],
        )
        .map_err(store_err)?;
        Ok(())
    }

    /// Drop a todo at a position inside a column — the same-column half of a
    /// drag. `before_id` names the row it should land ahead of, or `None` to
    /// drop it at the end.
    ///
    /// Siblings whose slot merely shifts are written without a version bump:
    /// the change is invisible to them, and bumping would fail an unrelated
    /// editor's save for no reason.
    pub fn reorder_todo(
        &self,
        id: &str,
        expected_version: u64,
        status: &str,
        before_id: Option<&str>,
    ) -> Result<Todo, AppError> {
        let existing = self.get_todo(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let now = now_unix_ms();
        {
            let mut conn = self.conn.lock();
            let statuses = Self::status_map(&conn)?;
            let status = Self::check_status(&statuses, status)?;
            let completed_at = Self::completed_at_for(
                now,
                Self::status_is_done(&statuses, &status),
                existing.is_done,
                existing.completed_at,
            );
            let tx = conn.transaction().map_err(store_err)?;
            // Archived rows are off the board, so they keep their slot.
            let mut ordered: Vec<String> = {
                let mut stmt = stmt(
                    &tx,
                    "SELECT id FROM todos
                      WHERE status = ?1 AND archived = 0 AND id <> ?2
                      ORDER BY sort_order ASC, created_at ASC",
                )?;
                let rows = stmt
                    .query_map(params![status, id], |row| row.get::<_, String>(0))
                    .map_err(store_err)?;
                collect(rows)?
            };
            let at = match before_id {
                Some(before) => ordered
                    .iter()
                    .position(|other| other == before)
                    .unwrap_or(ordered.len()),
                None => ordered.len(),
            };
            ordered.insert(at, id.to_owned());
            for (slot, row_id) in ordered.iter().enumerate() {
                if row_id == id {
                    tx.execute(
                        "UPDATE todos SET status = ?2, completed_at = ?3, sort_order = ?4,
                             version = version + 1, updated_at = ?5
                         WHERE id = ?1 AND version = ?6",
                        params![
                            id,
                            status,
                            completed_at,
                            slot as i64,
                            now,
                            expected_version as i64
                        ],
                    )
                    .map_err(store_err)?;
                } else {
                    tx.execute(
                        "UPDATE todos SET sort_order = ?2 WHERE id = ?1",
                        params![row_id, slot as i64],
                    )
                    .map_err(store_err)?;
                }
            }
            tx.commit().map_err(store_err)?;
        }
        self.get_todo(id)?.ok_or(AppError::NotFound)
    }

    /// Replace a todo's tags wholesale — the editor sends the set it shows.
    /// The links are not part of the row, so the row's version still moves: a
    /// caller holding an older copy is out of date either way.
    pub fn set_todo_tags(
        &self,
        id: &str,
        expected_version: u64,
        tag_ids: &[String],
    ) -> Result<Todo, AppError> {
        let existing = self.get_todo(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let tag_ids = clean_id_list(tag_ids, MAX_TODO_TAGS)?;
        self.todo_tag_ids_exist(&tag_ids)?;
        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction().map_err(store_err)?;
            tx.execute("DELETE FROM todo_tag_links WHERE todo_id = ?1", params![id])
                .map_err(store_err)?;
            for tag_id in &tag_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO todo_tag_links (todo_id, tag_id) VALUES (?1, ?2)",
                    params![id, tag_id],
                )
                .map_err(store_err)?;
            }
            tx.execute(
                "UPDATE todos SET version = version + 1, updated_at = ?2
                 WHERE id = ?1 AND version = ?3",
                params![id, now_unix_ms(), expected_version as i64],
            )
            .map_err(store_err)?;
            tx.commit().map_err(store_err)?;
        }
        self.get_todo(id)?.ok_or(AppError::NotFound)
    }

    /// Replace a todo's related items wholesale. Both directions are stored, so
    /// the link shows on either todo; a self-link is refused.
    pub fn set_todo_links(
        &self,
        id: &str,
        expected_version: u64,
        link_ids: &[String],
    ) -> Result<Todo, AppError> {
        let existing = self.get_todo(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let link_ids = clean_id_list(link_ids, MAX_TODO_TAGS)?;
        for other in &link_ids {
            if other == &existing.id {
                return Err(AppError::InvalidInput);
            }
            self.todo_exists(other)?;
        }
        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction().map_err(store_err)?;
            // Either direction: a link the *other* todo created is this todo's
            // to remove too.
            tx.execute(
                "DELETE FROM todo_links WHERE todo_id = ?1 OR related_id = ?1",
                params![id],
            )
            .map_err(store_err)?;
            for other in &link_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO todo_links (todo_id, related_id) VALUES (?1, ?2)",
                    params![id, other],
                )
                .map_err(store_err)?;
            }
            tx.execute(
                "UPDATE todos SET version = version + 1, updated_at = ?2
                 WHERE id = ?1 AND version = ?3",
                params![id, now_unix_ms(), expected_version as i64],
            )
            .map_err(store_err)?;
            tx.commit().map_err(store_err)?;
        }
        self.get_todo(id)?.ok_or(AppError::NotFound)
    }

    /// Apply one action to many todos in a single transaction.
    ///
    /// `expected_version` is deliberately **not** checked per row: this is an
    /// explicit multi-row action the user just confirmed, and one stale copy
    /// among twenty rows must not fail the whole batch. Every touched row still
    /// bumps its own `version`, so a client holding an older copy of a row
    /// still loses its next single-row write to `RevisionConflict`. Do not
    /// "fix" this into per-row version checks.
    ///
    /// Everything that can fail — the op's own status id, every id — is
    /// validated before anything is written, so a bad id cannot leave half the
    /// batch applied.
    pub fn bulk_update_todos(
        &self,
        ids: &[String],
        op: &TodoBulkOp,
    ) -> Result<Vec<Todo>, AppError> {
        let ids = clean_id_list(ids, MAX_ID_LIST)?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        for id in &ids {
            self.todo_exists(id)?;
        }
        let now = now_unix_ms();
        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction().map_err(store_err)?;
            match op {
                TodoBulkOp::SetStatus { status } => {
                    let statuses = Self::status_map(&tx)?;
                    let status = Self::check_status(&statuses, status)?;
                    // Already-done rows keep the moment they became done.
                    let sql = if Self::status_is_done(&statuses, &status) {
                        "UPDATE todos SET status = ?2, completed_at = COALESCE(completed_at, ?3),
                             version = version + 1, updated_at = ?3
                         WHERE id = ?1"
                    } else {
                        "UPDATE todos SET status = ?2, completed_at = NULL,
                             version = version + 1, updated_at = ?3
                         WHERE id = ?1"
                    };
                    for id in &ids {
                        tx.execute(sql, params![id, status, now])
                            .map_err(store_err)?;
                    }
                }
                TodoBulkOp::SetPriority { priority } => {
                    for id in &ids {
                        tx.execute(
                            "UPDATE todos SET priority = ?2, version = version + 1, updated_at = ?3
                             WHERE id = ?1",
                            params![id, priority.as_str(), now],
                        )
                        .map_err(store_err)?;
                    }
                }
                TodoBulkOp::Archive { archived } => {
                    for id in &ids {
                        tx.execute(
                            "UPDATE todos SET archived = ?2, version = version + 1, updated_at = ?3
                             WHERE id = ?1",
                            params![id, *archived as i64, now],
                        )
                        .map_err(store_err)?;
                    }
                }
                TodoBulkOp::Delete => {
                    for id in &ids {
                        // Subtree first, exactly as `delete_todo` does.
                        tx.execute("DELETE FROM todos WHERE parent_id = ?1", params![id])
                            .map_err(store_err)?;
                        tx.execute("DELETE FROM todos WHERE id = ?1", params![id])
                            .map_err(store_err)?;
                    }
                }
            }
            tx.commit().map_err(store_err)?;
        }
        if matches!(op, TodoBulkOp::Delete) {
            return Ok(Vec::new());
        }
        ids.iter()
            .map(|id| self.get_todo(id)?.ok_or(AppError::NotFound))
            .collect()
    }

    // ------------------------------------------------------------------
    // Todo statuses
    // ------------------------------------------------------------------

    pub fn list_todo_statuses(&self) -> Result<Vec<TodoStatusDef>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM todo_statuses ORDER BY sort_order ASC, created_at ASC",
            Self::STATUS_COLUMNS
        );
        let mut stmt = stmt(&conn, &sql)?;
        let rows = stmt.query_map([], Self::read_status).map_err(store_err)?;
        collect(rows)
    }

    pub fn create_todo_status(&self, draft: &TodoStatusDraft) -> Result<TodoStatusDef, AppError> {
        let name = clean_title(&draft.name)?;
        let color = clean_color(draft.color.as_deref(), "gray")?;
        let is_done = draft.is_done.unwrap_or(false);
        let now = now_unix_ms();
        let id = new_id("st");
        {
            let conn = self.conn.lock();
            let next: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM todo_statuses",
                    [],
                    |row| row.get(0),
                )
                .map_err(store_err)?;
            conn.execute(
                "INSERT INTO todo_statuses
                     (id, name, color, is_done, builtin, sort_order, version, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, ?5, 1, ?6, ?6)",
                params![id, name, color, is_done as i64, next, now],
            )
            .map_err(store_err)?;
        }
        self.todo_status(&id)?.ok_or(AppError::NotFound)
    }

    pub fn update_todo_status(
        &self,
        id: &str,
        expected_version: u64,
        patch: &TodoStatusDef,
    ) -> Result<TodoStatusDef, AppError> {
        let existing = self.todo_status(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let name = clean_title(&patch.name)?;
        let color = clean_color(Some(&patch.color), "gray")?;
        let now = now_unix_ms();
        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction().map_err(store_err)?;
            tx.execute(
                "UPDATE todo_statuses SET name = ?2, color = ?3, is_done = ?4,
                     version = version + 1, updated_at = ?5
                 WHERE id = ?1 AND version = ?6",
                params![
                    id,
                    name,
                    color,
                    patch.is_done as i64,
                    now,
                    expected_version as i64
                ],
            )
            .map_err(store_err)?;
            // Flipping `is_done` restates every todo in this status, so
            // `completed_at` is reconciled to match rather than left stale.
            if patch.is_done != existing.is_done {
                if patch.is_done {
                    tx.execute(
                        "UPDATE todos SET completed_at = COALESCE(completed_at, ?2)
                         WHERE status = ?1",
                        params![id, now],
                    )
                    .map_err(store_err)?;
                } else {
                    tx.execute(
                        "UPDATE todos SET completed_at = NULL WHERE status = ?1",
                        params![id],
                    )
                    .map_err(store_err)?;
                }
            }
            tx.commit().map_err(store_err)?;
        }
        self.todo_status(id)?.ok_or(AppError::NotFound)
    }

    /// Remove a status.
    ///
    /// A built-in one is refused. One that still holds todos is refused with
    /// `StatusInUse` unless the caller names a `reassign_to` status, in which
    /// case those todos move there first — that is what turns the GUI's dead
    /// end into a "该状态仍有 n 个事项" picker.
    ///
    /// `todos.status` carries no foreign key, so the move has to happen here,
    /// in the same transaction as the delete: nothing else would stop the rows
    /// from pointing at a status that no longer exists.
    pub fn delete_todo_status(
        &self,
        id: &str,
        expected_version: u64,
        reassign_to: Option<&str>,
    ) -> Result<(), AppError> {
        let existing = self.todo_status(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        if existing.builtin {
            return Err(AppError::InvalidInput);
        }
        let now = now_unix_ms();
        {
            let mut conn = self.conn.lock();
            let statuses = Self::status_map(&conn)?;
            let target = match reassign_to.map(str::trim).filter(|v| !v.is_empty()) {
                Some(value) => {
                    let value = Self::check_status(&statuses, value)?;
                    if value == id {
                        return Err(AppError::InvalidInput);
                    }
                    Some(value)
                }
                None => None,
            };
            let held: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM todos WHERE status = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .map_err(store_err)?;
            if held > 0 && target.is_none() {
                return Err(AppError::StatusInUse);
            }
            let tx = conn.transaction().map_err(store_err)?;
            if let Some(target) = target.as_deref() {
                if Self::status_is_done(&statuses, target) {
                    tx.execute(
                        "UPDATE todos SET status = ?2, completed_at = COALESCE(completed_at, ?3),
                             version = version + 1, updated_at = ?3
                         WHERE status = ?1",
                        params![id, target, now],
                    )
                    .map_err(store_err)?;
                } else {
                    tx.execute(
                        "UPDATE todos SET status = ?2, completed_at = NULL,
                             version = version + 1, updated_at = ?3
                         WHERE status = ?1",
                        params![id, target, now],
                    )
                    .map_err(store_err)?;
                }
            }
            tx.execute("DELETE FROM todo_statuses WHERE id = ?1", params![id])
                .map_err(store_err)?;
            tx.commit().map_err(store_err)?;
        }
        Ok(())
    }

    /// Write a new column order. `ids` must name every status exactly once — a
    /// partial list would leave the rest at stale sort keys. Sort keys are not
    /// versions, so nothing is bumped here.
    pub fn reorder_todo_statuses(&self, ids: &[String]) -> Result<Vec<TodoStatusDef>, AppError> {
        let ids = clean_id_list(ids, MAX_ID_LIST)?;
        {
            let mut conn = self.conn.lock();
            let known: Vec<String> = {
                let mut stmt = stmt(&conn, "SELECT id FROM todo_statuses")?;
                let rows = stmt
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(store_err)?;
                collect(rows)?
            };
            if known.len() != ids.len() || !ids.iter().all(|id| known.contains(id)) {
                return Err(AppError::InvalidInput);
            }
            let tx = conn.transaction().map_err(store_err)?;
            for (slot, id) in ids.iter().enumerate() {
                tx.execute(
                    "UPDATE todo_statuses SET sort_order = ?2 WHERE id = ?1",
                    params![id, slot as i64],
                )
                .map_err(store_err)?;
            }
            tx.commit().map_err(store_err)?;
        }
        self.list_todo_statuses()
    }

    fn todo_status(&self, id: &str) -> Result<Option<TodoStatusDef>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM todo_statuses WHERE id = ?1",
            Self::STATUS_COLUMNS
        );
        conn.query_row(&sql, params![id], Self::read_status)
            .optional()
            .map_err(store_err)
    }

    // ------------------------------------------------------------------
    // Todo tags and people
    // ------------------------------------------------------------------

    pub fn list_todo_tags(&self) -> Result<Vec<TodoTag>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM todo_tags ORDER BY sort_order ASC, name ASC",
            Self::TAG_COLUMNS
        );
        let mut stmt = stmt(&conn, &sql)?;
        let rows = stmt.query_map([], Self::read_tag).map_err(store_err)?;
        collect(rows)
    }

    pub fn create_todo_tag(&self, draft: &TodoTagDraft) -> Result<TodoTag, AppError> {
        let name = clean_title(&draft.name)?;
        let color = clean_color(draft.color.as_deref(), "blue")?;
        let now = now_unix_ms();
        let id = new_id("tag");
        {
            let conn = self.conn.lock();
            let next: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM todo_tags",
                    [],
                    |row| row.get(0),
                )
                .map_err(store_err)?;
            conn.execute(
                "INSERT INTO todo_tags (id, name, color, sort_order, version, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)",
                params![id, name, color, next, now],
            )
            .map_err(store_err)?;
        }
        self.todo_tag(&id)?.ok_or(AppError::NotFound)
    }

    pub fn update_todo_tag(
        &self,
        id: &str,
        expected_version: u64,
        name: &str,
        color: &str,
    ) -> Result<TodoTag, AppError> {
        let existing = self.todo_tag(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let name = clean_title(name)?;
        let color = clean_color(Some(color), "blue")?;
        {
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE todo_tags SET name = ?2, color = ?3, version = version + 1, updated_at = ?4
                 WHERE id = ?1 AND version = ?5",
                params![id, name, color, now_unix_ms(), expected_version as i64],
            )
            .map_err(store_err)?;
        }
        self.todo_tag(id)?.ok_or(AppError::NotFound)
    }

    /// Remove a tag. Its links cascade, and every todo that carried it moves
    /// its version — from a reader's point of view those todos just changed,
    /// and a copy holding the tag must lose its next write rather than try to
    /// re-link an id that no longer exists.
    pub fn delete_todo_tag(&self, id: &str, expected_version: u64) -> Result<(), AppError> {
        let existing = self.todo_tag(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let mut conn = self.conn.lock();
        let tx = conn.transaction().map_err(store_err)?;
        // Before the delete: the links are what names the todos to bump.
        tx.execute(
            "UPDATE todos SET version = version + 1, updated_at = ?2
             WHERE id IN (SELECT todo_id FROM todo_tag_links WHERE tag_id = ?1)",
            params![id, now_unix_ms()],
        )
        .map_err(store_err)?;
        tx.execute("DELETE FROM todo_tags WHERE id = ?1", params![id])
            .map_err(store_err)?;
        tx.commit().map_err(store_err)?;
        Ok(())
    }

    fn todo_tag(&self, id: &str) -> Result<Option<TodoTag>, AppError> {
        let conn = self.conn.lock();
        let sql = format!("SELECT {} FROM todo_tags WHERE id = ?1", Self::TAG_COLUMNS);
        conn.query_row(&sql, params![id], Self::read_tag)
            .optional()
            .map_err(store_err)
    }

    pub fn list_todo_people(&self) -> Result<Vec<TodoPerson>, AppError> {
        let conn = self.conn.lock();
        let mut stmt = stmt(
            &conn,
            "SELECT id, name, color, created_at FROM todo_people ORDER BY created_at ASC, name ASC",
        )?;
        let rows = stmt.query_map([], Self::read_person).map_err(store_err)?;
        collect(rows)
    }

    pub fn create_todo_person(&self, name: &str, color: &str) -> Result<TodoPerson, AppError> {
        let name = clean_title(name)?;
        let color = clean_color(Some(color), "blue")?;
        let now = now_unix_ms();
        let id = new_id("person");
        {
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO todo_people (id, name, color, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![id, name, color, now],
            )
            .map_err(store_err)?;
        }
        Ok(TodoPerson {
            id,
            name,
            color,
            created_at: now,
        })
    }

    // ------------------------------------------------------------------
    // Todo attachments, reminders and activity
    // ------------------------------------------------------------------

    pub fn list_todo_attachments(&self, todo_id: &str) -> Result<Vec<TodoAttachment>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM todo_attachments WHERE todo_id = ?1 ORDER BY created_at ASC, id ASC",
            Self::ATTACHMENT_COLUMNS
        );
        let mut stmt = stmt(&conn, &sql)?;
        let rows = stmt
            .query_map(params![todo_id], Self::read_attachment)
            .map_err(store_err)?;
        collect(rows)
    }

    /// Record a file as attached. The file itself is never read, copied or
    /// deleted — this is a reference. A repeated `request_id` returns the
    /// record created the first time.
    pub fn create_todo_attachment(
        &self,
        draft: &TodoAttachmentDraft,
    ) -> Result<TodoAttachment, AppError> {
        let name = clean_title(&draft.name)?;
        let path = draft.path.trim();
        if path.is_empty() {
            return Err(AppError::InvalidInput);
        }
        self.todo_exists(&draft.todo_id)?;
        let mime = draft.mime.as_deref().unwrap_or("").trim().to_owned();
        let request_id = draft
            .request_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        if let Some(key) = request_id.as_deref() {
            if let Some(existing) = self.attachment_by_request(key)? {
                return Ok(existing);
            }
        }
        let now = now_unix_ms();
        let id = new_id("att");
        {
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO todo_attachments
                     (id, todo_id, name, path, size, mime, version, created_at, updated_at,
                      request_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?7, ?8)
                 ON CONFLICT(request_id) WHERE request_id IS NOT NULL DO NOTHING",
                params![
                    id,
                    draft.todo_id,
                    name,
                    path,
                    draft.size,
                    mime,
                    now,
                    request_id
                ],
            )
            .map_err(store_err)?;
        }
        match request_id {
            Some(key) => self.attachment_by_request(&key)?.ok_or(AppError::NotFound),
            None => self.attachment(&id)?.ok_or(AppError::NotFound),
        }
    }

    /// Rename an attachment. The path, size and mime are facts about the file
    /// on disk and are not editable.
    pub fn update_todo_attachment(
        &self,
        id: &str,
        expected_version: u64,
        name: &str,
    ) -> Result<TodoAttachment, AppError> {
        let name = clean_title(name)?;
        {
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE todo_attachments SET name = ?2, version = version + 1, updated_at = ?3
                 WHERE id = ?1 AND version = ?4",
                params![id, name, now_unix_ms(), expected_version as i64],
            )
            .map_err(store_err)?;
        }
        self.attachment(id)?.ok_or(AppError::NotFound)
    }

    /// Forget an attachment. The file on disk is left alone — Kodo never
    /// deletes a file it did not create.
    pub fn delete_todo_attachment(&self, id: &str, expected_version: u64) -> Result<(), AppError> {
        let existing = self.attachment(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM todo_attachments WHERE id = ?1 AND version = ?2",
            params![id, expected_version as i64],
        )
        .map_err(store_err)?;
        Ok(())
    }

    fn attachment(&self, id: &str) -> Result<Option<TodoAttachment>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM todo_attachments WHERE id = ?1",
            Self::ATTACHMENT_COLUMNS
        );
        conn.query_row(&sql, params![id], Self::read_attachment)
            .optional()
            .map_err(store_err)
    }

    fn attachment_by_request(&self, request_id: &str) -> Result<Option<TodoAttachment>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM todo_attachments WHERE request_id = ?1",
            Self::ATTACHMENT_COLUMNS
        );
        conn.query_row(&sql, params![request_id], Self::read_attachment)
            .optional()
            .map_err(store_err)
    }

    pub fn list_todo_reminders(&self, todo_id: &str) -> Result<Vec<TodoReminder>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM todo_reminders WHERE todo_id = ?1 ORDER BY created_at ASC, id ASC",
            Self::REMINDER_COLUMNS
        );
        let mut stmt = stmt(&conn, &sql)?;
        let rows = stmt
            .query_map(params![todo_id], Self::read_reminder)
            .map_err(store_err)?;
        collect(rows)
    }

    pub fn create_todo_reminder(
        &self,
        draft: &TodoReminderDraft,
    ) -> Result<TodoReminder, AppError> {
        if draft.kind == TodoReminderKind::DueBefore {
            // A due-before reminder without an offset would never fire.
            match draft.offset_minutes {
                Some(minutes) if (0..=525_600).contains(&minutes) => {}
                _ => return Err(AppError::InvalidInput),
            }
        }
        self.todo_exists(&draft.todo_id)?;
        let message = clean_body(draft.message.as_deref())?;
        let enabled = draft.enabled.unwrap_or(true);
        let now = now_unix_ms();
        let id = new_id("rem");
        {
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO todo_reminders
                     (id, todo_id, kind, offset_minutes, message, enabled, version, created_at,
                      updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?7)",
                params![
                    id,
                    draft.todo_id,
                    draft.kind.as_str(),
                    draft.offset_minutes,
                    message,
                    enabled as i64,
                    now
                ],
            )
            .map_err(store_err)?;
        }
        self.reminder(&id)?.ok_or(AppError::NotFound)
    }

    pub fn set_todo_reminder_enabled(
        &self,
        id: &str,
        expected_version: u64,
        enabled: bool,
    ) -> Result<TodoReminder, AppError> {
        {
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE todo_reminders SET enabled = ?2, version = version + 1, updated_at = ?3
                 WHERE id = ?1 AND version = ?4",
                params![id, enabled as i64, now_unix_ms(), expected_version as i64],
            )
            .map_err(store_err)?;
        }
        self.reminder(id)?.ok_or(AppError::NotFound)
    }

    pub fn delete_todo_reminder(&self, id: &str, expected_version: u64) -> Result<(), AppError> {
        let existing = self.reminder(id)?.ok_or(AppError::NotFound)?;
        if existing.version != expected_version {
            return Err(AppError::RevisionConflict);
        }
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM todo_reminders WHERE id = ?1 AND version = ?2",
            params![id, expected_version as i64],
        )
        .map_err(store_err)?;
        Ok(())
    }

    fn reminder(&self, id: &str) -> Result<Option<TodoReminder>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM todo_reminders WHERE id = ?1",
            Self::REMINDER_COLUMNS
        );
        conn.query_row(&sql, params![id], Self::read_reminder)
            .optional()
            .map_err(store_err)
    }

    pub fn list_todo_activity(&self, todo_id: &str) -> Result<Vec<TodoActivity>, AppError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {} FROM todo_activity WHERE todo_id = ?1 ORDER BY created_at ASC, id ASC",
            Self::ACTIVITY_COLUMNS
        );
        let mut stmt = stmt(&conn, &sql)?;
        let rows = stmt
            .query_map(params![todo_id], Self::read_activity)
            .map_err(store_err)?;
        collect(rows)
    }

    /// Append a comment. Entries are never edited or removed — the log's value
    /// is that it is a log.
    pub fn add_todo_comment(
        &self,
        todo_id: &str,
        author_id: Option<&str>,
        body: &str,
    ) -> Result<TodoActivity, AppError> {
        let body = clean_body(Some(body))?;
        if body.is_empty() {
            return Err(AppError::InvalidInput);
        }
        self.todo_exists(todo_id)?;
        if let Some(author) = author_id {
            self.todo_person_exists(author)?;
        }
        let now = now_unix_ms();
        let id = new_id("act");
        {
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO todo_activity (id, todo_id, kind, author_id, body, meta, created_at)
                 VALUES (?1, ?2, 'comment', ?3, ?4, '', ?5)",
                params![id, todo_id, author_id, body, now],
            )
            .map_err(store_err)?;
        }
        Ok(TodoActivity {
            id,
            todo_id: todo_id.to_owned(),
            kind: "comment".to_owned(),
            author_id: author_id.map(str::to_owned),
            body,
            meta: String::new(),
            created_at: now,
        })
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

        let now = now_unix_ms();
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
                    now_unix_ms(),
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

        let now = now_unix_ms();
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
                    now_unix_ms(),
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
                    now_unix_ms(),
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
                params![key, job_type, now_unix_ms()],
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

/// `?start, ?start+1, …` — an `IN` clause with `count` bindings, numbered from
/// `start` so it can follow earlier parameters in the same statement.
fn placeholders(start: usize, count: usize) -> String {
    (0..count)
        .map(|offset| format!("?{}", start + offset))
        .collect::<Vec<_>>()
        .join(", ")
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
        assert_eq!(apps.len(), 4);
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
        assert_eq!(order.len(), 4);
        assert_eq!(order[1].manifest.app_id, "todo");
        assert_eq!(order[2].manifest.app_id, "calendar");
        // A full reorder is honoured exactly.
        let order = store
            .set_app_order(&["calendar".into(), "todo".into(), "artifacts".into()])
            .unwrap();
        let ids: Vec<&str> = order.iter().map(|a| a.manifest.app_id.as_str()).collect();
        assert_eq!(ids, vec!["calendar", "todo", "artifacts", "knowledge"]);
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
        assert_eq!(todo.status, DEFAULT_TODO_STATUS);
        assert_eq!(todo.priority, TodoPriority::Normal);
        assert_eq!(todo.item_type, "task");
        assert!(todo.module.is_empty());
        assert!(todo.due_date.is_none());
        assert!(todo.due_time.is_none());
        assert!(todo.completed_at.is_none());
        assert!(todo.tag_ids.is_empty() && todo.link_ids.is_empty());
        assert!(!todo.is_done);
        assert!(!todo.archived);
    }

    /// Upgrade path: a database written by v1 keeps every row. The three ids
    /// the old enum stored (`todo` / `doing` / `done`) are the ids migration
    /// 0002 seeds, so the status column is *reinterpreted* as a catalog id
    /// rather than rewritten — and a done row still reads as done.
    #[test]
    fn migration_0002_leaves_existing_rows_readable() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("workbench").join("v1");
        std::fs::create_dir_all(&home).unwrap();
        {
            // A v1 install: migration 0001 applied, two todos, no catalog yet.
            let conn = Connection::open(home.join(DB_FILE)).unwrap();
            conn.execute_batch(MIGRATION_0001).unwrap();
            conn.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER
                     NOT NULL, description TEXT NOT NULL);
                 INSERT INTO schema_migrations VALUES (1, 0, 'workbench v1');",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO todos
                     (id, title, notes, status, priority, archived, sort_order, source, version,
                      created_at, updated_at)
                 VALUES ('old-1', '旧事项', '', 'done', 'high', 0, 1, 'manual', 3, 0, 0),
                        ('old-2', '另一条', '', 'doing', 'normal', 0, 2, 'manual', 1, 0, 0)",
                [],
            )
            .unwrap();
        }

        let store = AppStore::open(dir.path()).unwrap();
        let todos = store.list_todos(true).unwrap();
        assert_eq!(todos.len(), 2);
        let old = todos.iter().find(|t| t.id == "old-1").unwrap();
        assert_eq!(old.status, "done");
        assert!(old.is_done, "a v1 done row still reads as done");
        assert_eq!(old.priority, TodoPriority::High);
        assert_eq!(old.version, 3, "no row was rewritten");
        let other = todos.iter().find(|t| t.id == "old-2").unwrap();
        assert!(!other.is_done);
        // The catalog the new columns reference is in place.
        assert_eq!(store.list_todo_statuses().unwrap().len(), 3);
        assert_eq!(store.list_todo_people().unwrap().len(), 1);
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
            .set_todo_status(&todo.id, todo.version, "done")
            .unwrap();
        assert_eq!(done.status, "done");
        assert!(done.is_done);
        assert!(done.completed_at.is_some());
        // Restoring clears the completion stamp and returns to 待处理.
        let restored = store
            .set_todo_status(&todo.id, done.version, "todo")
            .unwrap();
        assert_eq!(restored.status, "todo");
        assert!(!restored.is_done);
        assert!(restored.completed_at.is_none());
    }

    #[test]
    fn todo_status_write_needs_the_current_version() {
        let (_dir, store) = store();
        let todo = store.create_todo(&draft("a")).unwrap();
        store
            .set_todo_status(&todo.id, todo.version, "doing")
            .unwrap();
        assert_eq!(
            store.set_todo_status(&todo.id, todo.version, "done"),
            Err(AppError::RevisionConflict)
        );
    }

    #[test]
    fn an_unknown_status_is_refused() {
        let (_dir, store) = store();
        let todo = store.create_todo(&draft("a")).unwrap();
        assert_eq!(
            store.set_todo_status(&todo.id, todo.version, "nope"),
            Err(AppError::NotFound)
        );
        let mut d = draft("b");
        d.status = Some("nope".into());
        assert_eq!(store.create_todo(&d), Err(AppError::NotFound));
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

    /// `todos.parent_id` is `ON DELETE SET NULL`, so a delete that did not walk
    /// the subtree would quietly promote the children to top-level rows.
    #[test]
    fn deleting_a_todo_takes_its_sub_items_with_it() {
        let (_dir, store) = store();
        let parent = store.create_todo(&draft("父事项")).unwrap();
        let mut child = draft("子项");
        child.parent_id = Some(parent.id.clone());
        let child = store.create_todo(&child).unwrap();
        assert_eq!(store.list_todos(true).unwrap().len(), 2);

        store.delete_todo(&parent.id, parent.version).unwrap();
        assert!(store.list_todos(true).unwrap().is_empty());
        assert!(store.get_todo(&child.id).unwrap().is_none());
        // A stale delete is refused like every other write.
        assert_eq!(
            store.delete_todo(&parent.id, parent.version),
            Err(AppError::NotFound)
        );
    }

    /// The whole editor round-trips a row, so `update_todo` must keep every
    /// field it is handed — a drop here would silently reset a column.
    #[test]
    fn a_whole_row_update_keeps_every_field() {
        let (_dir, store) = store();
        let person = store.create_todo_person("张三", "teal").unwrap();
        let tag = store
            .create_todo_tag(&TodoTagDraft {
                name: "后端".into(),
                color: Some("purple".into()),
            })
            .unwrap();
        let other = store.create_todo(&draft("关联事项")).unwrap();

        let mut d = draft("整行往返");
        d.due_date = Some("2026-10-20".into());
        d.due_time = Some("09:30".into());
        d.assignee_id = Some(person.id.clone());
        d.item_type = Some("bug".into());
        d.module = Some("实时数仓".into());
        d.estimate_hours = Some(8.0);
        d.actual_hours = Some(3.5);
        d.tag_ids = Some(vec![tag.id.clone()]);
        let created = store.create_todo(&d).unwrap();
        assert_eq!(created.estimate_hours, Some(8.0));
        assert_eq!(created.tag_ids, vec![tag.id.clone()]);

        store
            .set_todo_links(&created.id, created.version, &[other.id.clone()])
            .unwrap();
        let current = store.get_todo(&created.id).unwrap().unwrap();
        assert_eq!(current.link_ids, vec![other.id.clone()]);

        let mut patch = current.clone();
        patch.notes = "改过的备注".into();
        let updated = store.update_todo(current.version, &patch).unwrap();
        assert_eq!(updated.notes, "改过的备注");
        assert_eq!(updated.due_time.as_deref(), Some("09:30"));
        assert_eq!(updated.assignee_id.as_deref(), Some(person.id.as_str()));
        assert_eq!(updated.item_type, "bug");
        assert_eq!(updated.module, "实时数仓");
        assert_eq!(updated.estimate_hours, Some(8.0));
        assert_eq!(updated.actual_hours, Some(3.5));
        assert_eq!(updated.tag_ids, vec![tag.id.clone()]);
        assert_eq!(updated.link_ids, vec![other.id.clone()]);
    }

    #[test]
    fn tags_and_links_are_replaced_wholesale() {
        let (_dir, store) = store();
        let first = store.create_todo(&draft("一")).unwrap();
        let second = store.create_todo(&draft("二")).unwrap();
        let tag = store
            .create_todo_tag(&TodoTagDraft {
                name: "紧急".into(),
                color: None,
            })
            .unwrap();
        assert_eq!(tag.color, "blue", "an unnamed colour falls back");

        let tagged = store
            .set_todo_tags(&first.id, first.version, &["missing".into()])
            .unwrap_err();
        assert_eq!(tagged, AppError::NotFound);
        let tagged = store
            .set_todo_tags(&first.id, first.version, &[tag.id.clone()])
            .unwrap();
        assert_eq!(tagged.tag_ids, vec![tag.id.clone()]);
        // Clearing takes them all off again.
        let cleared = store.set_todo_tags(&first.id, tagged.version, &[]).unwrap();
        assert!(cleared.tag_ids.is_empty());

        // A link shows on both sides, and a self-link is refused.
        let linked = store
            .set_todo_links(&first.id, cleared.version, &[second.id.clone()])
            .unwrap();
        assert_eq!(linked.link_ids, vec![second.id.clone()]);
        let back = store.get_todo(&second.id).unwrap().unwrap();
        assert_eq!(back.link_ids, vec![first.id.clone()]);
        assert_eq!(
            store.set_todo_links(&first.id, linked.version, &[first.id.clone()]),
            Err(AppError::InvalidInput)
        );
        // Deleting a tag takes its links with it, leaving no dangling id.
        let linked = store
            .set_todo_tags(&first.id, linked.version, &[tag.id.clone()])
            .unwrap();
        store.delete_todo_tag(&tag.id, tag.version).unwrap();
        let after = store.get_todo(&first.id).unwrap().unwrap();
        assert_eq!(after.version, linked.version + 1);
        assert!(after.tag_ids.is_empty());
    }

    /// `todos.status` has no foreign key, so deleting a status has to move its
    /// todos — or refuse.
    #[test]
    fn a_status_in_use_needs_a_target_before_it_can_go() {
        let (_dir, store) = store();
        let review = store
            .create_todo_status(&TodoStatusDraft {
                name: "待确认".into(),
                color: Some("amber".into()),
                is_done: None,
            })
            .unwrap();
        assert!(!review.is_done && !review.builtin);
        let mut d = draft("等待评审");
        d.status = Some(review.id.clone());
        let todo = store.create_todo(&d).unwrap();

        assert_eq!(
            store.delete_todo_status(&review.id, review.version, None),
            Err(AppError::StatusInUse)
        );
        // A built-in status is never removable, even with a target.
        let builtin = store
            .list_todo_statuses()
            .unwrap()
            .into_iter()
            .find(|s| s.id == "todo")
            .unwrap();
        assert_eq!(
            store.delete_todo_status(&builtin.id, builtin.version, Some("doing")),
            Err(AppError::InvalidInput)
        );

        store
            .delete_todo_status(&review.id, review.version, Some("doing"))
            .unwrap();
        let moved = store.get_todo(&todo.id).unwrap().unwrap();
        assert_eq!(moved.status, "doing");
        assert_eq!(moved.version, todo.version + 1, "the move is a real edit");
        assert_eq!(store.list_todo_statuses().unwrap().len(), 3);
    }

    #[test]
    fn bulk_actions_touch_many_rows_in_one_call() {
        let (_dir, store) = store();
        let a = store.create_todo(&draft("甲")).unwrap();
        let b = store.create_todo(&draft("乙")).unwrap();
        let ids = vec![a.id.clone(), b.id.clone()];

        let done = store
            .bulk_update_todos(
                &ids,
                &TodoBulkOp::SetStatus {
                    status: "done".into(),
                },
            )
            .unwrap();
        assert!(done.iter().all(|t| t.is_done));
        assert!(done.iter().all(|t| t.completed_at.is_some()));
        // Deliberately not version-checked: a stale copy still applies, and the
        // row's own version moves so the *next* single-row write is refused.
        let done = store
            .bulk_update_todos(
                &ids,
                &TodoBulkOp::SetPriority {
                    priority: TodoPriority::High,
                },
            )
            .unwrap();
        assert!(done.iter().all(|t| t.priority == TodoPriority::High));
        assert!(done.iter().all(|t| t.version == a.version + 2));

        let archived = store
            .bulk_update_todos(&ids, &TodoBulkOp::Archive { archived: true })
            .unwrap();
        assert!(archived.iter().all(|t| t.archived));
        // A bad id fails the whole call rather than half-applying it.
        let mut bad = ids.clone();
        bad.push("missing".into());
        assert_eq!(
            store.bulk_update_todos(&bad, &TodoBulkOp::Delete),
            Err(AppError::NotFound)
        );
        assert_eq!(store.list_todos(true).unwrap().len(), 2);
        store.bulk_update_todos(&ids, &TodoBulkOp::Delete).unwrap();
        assert!(store.list_todos(true).unwrap().is_empty());
    }

    #[test]
    fn reordering_a_column_renumbers_only_the_slots() {
        let (_dir, store) = store();
        let first = store.create_todo(&draft("一")).unwrap();
        let second = store.create_todo(&draft("二")).unwrap();
        let third = store.create_todo(&draft("三")).unwrap();

        // Drop the last todo at the top of 待处理.
        store
            .reorder_todo(&third.id, third.version, "todo", Some(&first.id))
            .unwrap();
        let titles: Vec<String> = store
            .list_todos(false)
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect();
        assert_eq!(titles, vec!["三", "一", "二"]);
        // Moving into a new column completes the todo when that column is done.
        let moved = store
            .reorder_todo(&second.id, second.version, "done", None)
            .unwrap();
        assert!(moved.is_done && moved.completed_at.is_some());
    }

    #[test]
    fn comments_append_and_need_a_body() {
        let (_dir, store) = store();
        let todo = store.create_todo(&draft("评论我")).unwrap();
        store
            .add_todo_comment(&todo.id, None, "已联系供应商")
            .unwrap();
        store.add_todo_comment(&todo.id, None, "明天跟进").unwrap();
        let log = store.list_todo_activity(&todo.id).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].kind, "comment");
        assert_eq!(log[0].body, "已联系供应商");
        assert_eq!(
            store.add_todo_comment(&todo.id, None, "   "),
            Err(AppError::InvalidInput)
        );
        // Deleting the todo takes its log with it.
        store.delete_todo(&todo.id, todo.version).unwrap();
        assert!(store.list_todo_activity(&todo.id).unwrap().is_empty());
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
        assert_eq!(todos[0].status, DEFAULT_TODO_STATUS);
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
