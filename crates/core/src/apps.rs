//! Workbench applications.
//!
//! Pure data and validation for the workbench (应用工作台), the built-in app
//! manifests, and the first-party apps: 待办 (todos), 日历 (calendar) and
//! 成果中心 (artifacts). Storage, lifecycle and job handlers live in
//! `kodo-shell::apps`; the desktop shell wires the Tauri commands.
//!
//! Nothing here knows about Tauri, React or transport.

use serde::{Deserialize, Serialize};

/// Host API version the built-in apps are written against. Third-party
/// packages are a later roadmap item; the field exists now so the built-in
/// manifests are honest about what they target.
pub const APP_API_VERSION: &str = "1";

/// Longest accepted todo / event / artifact title, in `char`s.
pub const MAX_TITLE_LEN: usize = 500;
/// Longest accepted free-text body, in `char`s.
pub const MAX_BODY_LEN: usize = 200_000;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Stable error codes. The frontend keys on the code string, never on the
/// message text (same rule as [`crate::scheduler`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppError {
    /// Caller-supplied record failed validation (empty title, bad date,
    /// end before start, over-long field).
    InvalidInput,
    /// The referenced record does not exist (or was archived away).
    NotFound,
    /// `expected_version` did not match the stored row: the caller re-reads
    /// and retries rather than overwriting a newer edit.
    RevisionConflict,
    /// The manifest id is not a registered app.
    UnknownApp,
    /// The app is disabled, so it may not accept writes right now.
    AppDisabled,
    /// Underlying storage failed.
    StorageUnavailable,
}

impl AppError {
    /// Stable code string handed to the GUI.
    pub fn code(self) -> &'static str {
        match self {
            AppError::InvalidInput => "invalid_input",
            AppError::NotFound => "not_found",
            AppError::RevisionConflict => "revision_conflict",
            AppError::UnknownApp => "unknown_app",
            AppError::AppDisabled => "app_disabled",
            AppError::StorageUnavailable => "storage_unavailable",
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for AppError {}

// ---------------------------------------------------------------------------
// Manifests and per-install preferences
// ---------------------------------------------------------------------------

/// Optional capabilities an app declares. Absent capabilities mean the app
/// simply does not offer that surface — it still installs and runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCapabilities {
    /// The app can hand user-selected content to an authorized AI action.
    pub ai: bool,
    /// The app exposes non-interactive actions the scheduler may trigger.
    pub scheduler: bool,
    /// The app offers a workbench summary (today's todos, today's events…).
    pub summary: bool,
}

/// Identity and description of one app. Built-in apps ship in the binary;
/// the manifest is the stable contract the workbench renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppManifest {
    pub app_id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub api_version: String,
    /// lucide-react icon name; the frontend maps it to a component.
    pub icon: String,
    /// `true` for the first-party apps compiled into this build.
    pub builtin: bool,
    pub capabilities: AppCapabilities,
}

/// Per-install state: whether the app is enabled, pinned, where it sorts,
/// and which optional integrations the user turned on. Independent of the
/// manifest so a manifest update never resets the user's choices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPreference {
    pub app_id: String,
    pub enabled: bool,
    pub favorite: bool,
    pub sort_order: i64,
    /// User opted this app into AI-assisted actions. Declaring the link does
    /// not send anything; it only unlocks the entry points.
    pub ai_enabled: bool,
    /// User opted this app's background actions into the scheduler.
    pub schedule_enabled: bool,
    pub updated_at: i64,
}

impl AppPreference {
    /// Defaults for a manifest the store has not seen before.
    pub fn defaults(app_id: &str, sort_order: i64) -> Self {
        Self {
            app_id: app_id.to_owned(),
            enabled: true,
            favorite: false,
            sort_order,
            ai_enabled: false,
            schedule_enabled: false,
            updated_at: 0,
        }
    }
}

/// A manifest paired with its per-install state — what the workbench renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppView {
    pub manifest: AppManifest,
    pub preference: AppPreference,
}

/// The first-party apps compiled into this build, in default workbench order.
pub fn builtin_apps() -> Vec<AppManifest> {
    vec![
        AppManifest {
            app_id: "todo".into(),
            name: "待办".into(),
            description: "记录事项，管理优先级与截止时间".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            api_version: APP_API_VERSION.into(),
            icon: "list-todo".into(),
            builtin: true,
            capabilities: AppCapabilities {
                ai: true,
                scheduler: true,
                summary: true,
            },
        },
        AppManifest {
            app_id: "calendar".into(),
            name: "日历".into(),
            description: "安排日程，查看工作时段与截止信息".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            api_version: APP_API_VERSION.into(),
            icon: "calendar-days".into(),
            builtin: true,
            capabilities: AppCapabilities {
                ai: false,
                scheduler: true,
                summary: true,
            },
        },
        AppManifest {
            app_id: "artifacts".into(),
            name: "成果中心".into(),
            description: "保存与管理文本、文件和工作成果".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            api_version: APP_API_VERSION.into(),
            icon: "files".into(),
            builtin: true,
            capabilities: AppCapabilities {
                ai: false,
                scheduler: false,
                summary: false,
            },
        },
        AppManifest {
            app_id: "knowledge".into(),
            name: "知识库".into(),
            description: "管理本地资料，检索并引用可追溯的出处".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            api_version: APP_API_VERSION.into(),
            icon: "book-open".into(),
            builtin: true,
            capabilities: AppCapabilities {
                ai: false,
                scheduler: false,
                summary: false,
            },
        },
    ]
}

/// Look up one built-in manifest by id.
pub fn builtin_app(app_id: &str) -> Option<AppManifest> {
    builtin_apps().into_iter().find(|a| a.app_id == app_id)
}

/// Job types a scheduled task may use to drive this app. One source of truth:
/// the shell registers handlers for exactly these, and turning the app off
/// pauses the tasks that name them.
pub fn app_job_types(app_id: &str) -> &'static [&'static str] {
    match app_id {
        "todo" => &["app.todo.create"],
        "calendar" => &["app.calendar.create_event"],
        // 成果中心 has no background action yet; the list stays empty rather
        // than shipping a job type nothing implements.
        _ => &[],
    }
}

// ---------------------------------------------------------------------------
// Todos (待办)
// ---------------------------------------------------------------------------

/// The single source of truth for a todo's business state. "Board column",
/// "completed" and "today overview" are all derived from this field — there
/// is no second completion flag to keep in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TodoStatus {
    Todo,
    Doing,
    Done,
}

impl TodoStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TodoStatus::Todo => "todo",
            TodoStatus::Doing => "doing",
            TodoStatus::Done => "done",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "todo" => Some(TodoStatus::Todo),
            "doing" => Some(TodoStatus::Doing),
            "done" => Some(TodoStatus::Done),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TodoPriority {
    Low,
    Normal,
    High,
}

impl TodoPriority {
    pub fn as_str(self) -> &'static str {
        match self {
            TodoPriority::Low => "low",
            TodoPriority::Normal => "normal",
            TodoPriority::High => "high",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(TodoPriority::Low),
            "normal" => Some(TodoPriority::Normal),
            "high" => Some(TodoPriority::High),
            _ => None,
        }
    }
}

/// One todo. Only `title` is required; everything else can be filled in
/// later, and the board renders whatever is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    pub id: String,
    pub title: String,
    pub notes: String,
    pub status: TodoStatus,
    pub priority: TodoPriority,
    /// Local due date, `YYYY-MM-DD`. `None` puts the todo in the inbox.
    pub due_date: Option<String>,
    /// Parent todo id for a sub-item. One level of nesting is what the UI
    /// shows; the column does not enforce depth beyond that.
    pub parent_id: Option<String>,
    /// Archived todos are hidden from the board and list but keep their data.
    pub archived: bool,
    /// Set when the todo entered `Done`; cleared when it leaves. Purely
    /// informational — `status` decides what the UI shows.
    pub completed_at: Option<i64>,
    /// Manual ordering inside a column, ascending. Board columns sort by
    /// this, then by `created_at`.
    pub sort_order: i64,
    /// How the todo was created: `manual` or `ai`.
    pub source: String,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Caller-supplied fields for a new todo. `title` is the only required one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoDraft {
    pub title: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub status: Option<TodoStatus>,
    #[serde(default)]
    pub priority: Option<TodoPriority>,
    #[serde(default)]
    pub due_date: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    /// Client-generated idempotency key: repeating the same create returns
    /// the original todo instead of adding a second one.
    #[serde(default)]
    pub request_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Calendar (日历)
// ---------------------------------------------------------------------------

/// One manually managed calendar event.
///
/// `date` / `start_time` / `end_time` are wall-clock values expressed in
/// `timezone`, which is stored explicitly so a later system-timezone change
/// never silently reinterprets an existing event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    pub notes: String,
    /// Local calendar date, `YYYY-MM-DD`.
    pub date: String,
    /// `HH:MM` in `timezone`; `None` for an all-day event.
    pub start_time: Option<String>,
    /// `HH:MM` in `timezone`; `None` for an all-day event.
    pub end_time: Option<String>,
    pub all_day: bool,
    /// IANA timezone name the wall-clock fields are expressed in.
    pub timezone: String,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEventDraft {
    pub title: String,
    #[serde(default)]
    pub notes: Option<String>,
    pub date: String,
    #[serde(default)]
    pub start_time: Option<String>,
    #[serde(default)]
    pub end_time: Option<String>,
    #[serde(default)]
    pub all_day: Option<bool>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Artifacts (成果中心)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactKind {
    /// A text snapshot the user saved into the app.
    Text,
    /// A reference to a file on disk. Removing the record never deletes
    /// the file.
    File,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::Text => "text",
            ArtifactKind::File => "file",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "text" => Some(ArtifactKind::Text),
            "file" => Some(ArtifactKind::File),
            _ => None,
        }
    }
}

/// How the referenced file looks right now, recomputed on every list. The
/// recorded size/mtime are the fingerprint captured when the reference was
/// registered or last refreshed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactFileStatus {
    /// Text artifacts have no file to check.
    NotApplicable,
    /// The file exists and matches the recorded fingerprint.
    Ok,
    /// The path no longer resolves.
    Missing,
    /// The file exists but its size or mtime moved since registration.
    Changed,
}

/// One saved artifact: a text snapshot or a file reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: String,
    pub title: String,
    pub kind: ArtifactKind,
    /// Present for text artifacts.
    pub body: Option<String>,
    /// Present for file artifacts.
    pub file_path: Option<String>,
    /// Size in bytes recorded when the reference was last captured.
    pub file_size: Option<i64>,
    /// Modification time (unix seconds) recorded at the same moment.
    pub file_modified_at: Option<i64>,
    /// Free-form provenance, e.g. `manual`, `ai`, `session`.
    pub source: String,
    /// User-chosen bucket; empty means uncategorized.
    pub category: String,
    pub favorite: bool,
    pub archived: bool,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// An artifact plus the file state resolved at read time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactView {
    pub artifact: Artifact,
    pub file_status: ArtifactFileStatus,
    /// Size on disk right now, when the file resolves.
    pub current_size: Option<i64>,
    pub current_modified_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactDraft {
    pub title: String,
    pub kind: ArtifactKind,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Trim and bound a required title.
pub fn clean_title(raw: &str) -> Result<String, AppError> {
    let title = raw.trim();
    if title.is_empty() {
        return Err(AppError::InvalidInput);
    }
    if title.chars().count() > MAX_TITLE_LEN {
        return Err(AppError::InvalidInput);
    }
    Ok(title.to_owned())
}

/// Bound an optional free-text body.
pub fn clean_body(raw: Option<&str>) -> Result<String, AppError> {
    let body = raw.unwrap_or("").trim();
    if body.chars().count() > MAX_BODY_LEN {
        return Err(AppError::InvalidInput);
    }
    Ok(body.to_owned())
}

/// Accept `YYYY-MM-DD` and reject calendar-invalid dates (2026-02-30).
pub fn validate_date(raw: &str) -> Result<String, AppError> {
    let value = raw.trim();
    if value.len() != 10 {
        return Err(AppError::InvalidInput);
    }
    jiff::civil::Date::strptime("%Y-%m-%d", value).map_err(|_| AppError::InvalidInput)?;
    Ok(value.to_owned())
}

/// Accept `HH:MM` (24h, zero-padded).
pub fn validate_time(raw: &str) -> Result<String, AppError> {
    let value = raw.trim();
    if value.len() != 5 || value.as_bytes()[2] != b':' {
        return Err(AppError::InvalidInput);
    }
    jiff::civil::Time::strptime("%H:%M", value).map_err(|_| AppError::InvalidInput)?;
    Ok(value.to_owned())
}

/// Accept an IANA timezone name the bundled tzdb knows.
pub fn validate_timezone(raw: &str) -> Result<String, AppError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(AppError::InvalidInput);
    }
    jiff::tz::TimeZone::get(value).map_err(|_| AppError::InvalidInput)?;
    Ok(value.to_owned())
}

/// The host's own IANA timezone, falling back to UTC when the platform does
/// not report one. Used as the default for new events.
pub fn system_timezone() -> String {
    jiff::tz::TimeZone::system()
        .iana_name()
        .map(str::to_owned)
        .unwrap_or_else(|| "UTC".to_owned())
}

/// Compare two `HH:MM` values. Wall-clock comparison is exact because both
/// sides are zero-padded 24-hour strings — and both are in the same stored
/// timezone, so no DST conversion is involved.
pub fn time_le(start: &str, end: &str) -> bool {
    start <= end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_apps_have_unique_ids() {
        let apps = builtin_apps();
        assert_eq!(apps.len(), 4);
        let mut ids: Vec<&str> = apps.iter().map(|a| a.app_id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 4);
    }

    #[test]
    fn builtin_manifest_round_trips_as_camel_case() {
        let app = builtin_app("todo").expect("todo app exists");
        let json = serde_json::to_value(&app).unwrap();
        assert_eq!(json["appId"], "todo");
        assert_eq!(json["apiVersion"], APP_API_VERSION);
        assert_eq!(json["capabilities"]["scheduler"], true);
        assert!(json.get("app_id").is_none());
    }

    #[test]
    fn todo_status_and_priority_use_stable_codes() {
        assert_eq!(serde_json::to_value(TodoStatus::Doing).unwrap(), "doing");
        assert_eq!(serde_json::to_value(TodoPriority::High).unwrap(), "high");
        assert_eq!(TodoStatus::parse("done"), Some(TodoStatus::Done));
        assert_eq!(TodoPriority::parse("urgent"), None);
    }

    #[test]
    fn titles_are_trimmed_and_bounded() {
        assert_eq!(clean_title("  买牛奶 ").unwrap(), "买牛奶");
        assert_eq!(clean_title("   "), Err(AppError::InvalidInput));
        assert_eq!(
            clean_title(&"x".repeat(MAX_TITLE_LEN + 1)),
            Err(AppError::InvalidInput)
        );
    }

    #[test]
    fn dates_reject_impossible_values() {
        assert_eq!(validate_date("2026-10-13").unwrap(), "2026-10-13");
        assert_eq!(validate_date("2026-02-30"), Err(AppError::InvalidInput));
        assert_eq!(
            validate_date("2026-10-13T00:00"),
            Err(AppError::InvalidInput)
        );
    }

    #[test]
    fn times_are_zero_padded_24h() {
        assert_eq!(validate_time("09:05").unwrap(), "09:05");
        assert_eq!(validate_time("24:00"), Err(AppError::InvalidInput));
        assert_eq!(validate_time("9:05"), Err(AppError::InvalidInput));
    }

    #[test]
    fn timezones_must_resolve() {
        assert_eq!(validate_timezone("Asia/Shanghai").unwrap(), "Asia/Shanghai");
        assert_eq!(
            validate_timezone("Mars/Olympus"),
            Err(AppError::InvalidInput)
        );
    }

    #[test]
    fn job_types_are_declared_per_app() {
        assert_eq!(app_job_types("todo"), &["app.todo.create"]);
        assert_eq!(app_job_types("calendar"), &["app.calendar.create_event"]);
        // An app may declare scheduler capability and still have no
        // background action implemented; that is not an error.
        assert!(app_job_types("artifacts").is_empty());
        assert!(app_job_types("nope").is_empty());
    }

    #[test]
    fn error_codes_are_stable_strings() {
        assert_eq!(AppError::RevisionConflict.code(), "revision_conflict");
        assert_eq!(AppError::AppDisabled.to_string(), "app_disabled");
    }
}
