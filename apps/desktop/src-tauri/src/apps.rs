//! Tauri command handlers that surface the workbench to the GUI.
//!
//! Every command is a thin wrapper over [`kodo_shell::AppStore`]: validation,
//! versioning and idempotency stay in the store, so the GUI and the scheduled
//! background path write through the same code. Commands return
//! `Result<T, String>` where the string is the stable [`AppError`] code
//! (`invalid_input`, `revision_conflict`, …) — the frontend keys on the code,
//! never on a message.
//!
//! The domain types already serialize as camelCase JSON, so they cross the
//! bridge as-is (same convention as `scheduler_cmds`).

use std::collections::BTreeMap;
use std::sync::Arc;

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use kodo_core::apps::{
    app_job_types, AppError, AppView, Artifact, ArtifactDraft, ArtifactView, CalendarEvent,
    CalendarEventDraft, Todo, TodoActivity, TodoAttachment, TodoAttachmentDraft, TodoBulkOp,
    TodoDraft, TodoPerson, TodoReminder, TodoReminderDraft, TodoStatusDef, TodoStatusDraft, TodoTag,
    TodoTagDraft,
};
use kodo_core::scheduler::TaskKind;
use kodo_shell::AppStore;

use crate::scheduler::SchedulerService;

/// Shared, cheap-to-clone handle to the workbench store.
///
/// The store is optional so a host that cannot open the database still boots
/// (the scheduler keeps running) and every workbench command answers with
/// `storage_unavailable` instead of panicking on missing state.
#[derive(Clone, Default)]
pub struct AppService {
    store: Option<Arc<AppStore>>,
}

impl AppService {
    /// Open the workbench database under the host's state directory.
    pub fn boot() -> Self {
        match AppStore::open_default() {
            Ok(store) => Self {
                store: Some(Arc::new(store)),
            },
            Err(error) => {
                eprintln!("kodo workbench: store unavailable: {error}");
                Self { store: None }
            }
        }
    }

    fn store(&self) -> Result<&Arc<AppStore>, String> {
        self.store
            .as_ref()
            .ok_or_else(|| AppError::StorageUnavailable.code().to_owned())
    }
}

fn app_err(error: AppError) -> String {
    error.code().to_owned()
}

// ---------------------------------------------------------------------------
// Apps
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_apps(service: State<'_, AppService>) -> Result<Vec<AppView>, String> {
    service.store()?.list_apps().map_err(app_err)
}

/// Enable or disable an app.
///
/// Disabling also pauses the scheduled tasks that drive it, so a rule cannot
/// keep writing after the user turned the app off. Re-enabling deliberately
/// does **not** switch those tasks back on: the user re-enables each rule
/// explicitly.
#[tauri::command]
pub fn set_app_enabled(
    service: State<'_, AppService>,
    scheduler: State<'_, SchedulerService>,
    app_id: String,
    enabled: bool,
) -> Result<AppView, String> {
    let view = service
        .store()?
        .set_app_enabled(&app_id, enabled)
        .map_err(app_err)?;
    if !enabled {
        pause_app_tasks(&scheduler, &app_id)?;
    }
    Ok(view)
}

#[tauri::command]
pub fn set_app_favorite(
    service: State<'_, AppService>,
    app_id: String,
    favorite: bool,
) -> Result<AppView, String> {
    service
        .store()?
        .set_app_favorite(&app_id, favorite)
        .map_err(app_err)
}

/// Reorder the workbench grid. The caller sends every app id it knows about,
/// in the wanted order.
#[tauri::command]
pub fn set_app_order(
    service: State<'_, AppService>,
    app_ids: Vec<String>,
) -> Result<Vec<AppView>, String> {
    service.store()?.set_app_order(&app_ids).map_err(app_err)
}

/// Turn the optional AI / scheduler links for one app on or off. An app that
/// does not declare a capability rejects the request rather than accepting a
/// switch that could never do anything.
#[tauri::command]
pub fn set_app_links(
    service: State<'_, AppService>,
    scheduler: State<'_, SchedulerService>,
    app_id: String,
    ai_enabled: bool,
    schedule_enabled: bool,
) -> Result<AppView, String> {
    let view = service
        .store()?
        .set_app_links(&app_id, ai_enabled, schedule_enabled)
        .map_err(app_err)?;
    if !schedule_enabled {
        pause_app_tasks(&scheduler, &app_id)?;
    }
    Ok(view)
}

/// Suspend the scheduled tasks that drive `app_id`. Tasks stay on disk with
/// their schedule intact — they are paused, not archived.
fn pause_app_tasks(scheduler: &SchedulerService, app_id: &str) -> Result<(), String> {
    let job_types = app_job_types(app_id);
    if job_types.is_empty() {
        return Ok(());
    }
    for task in scheduler.list_tasks(false)? {
        let TaskKind::Job { job_type, .. } = &task.task else {
            continue;
        };
        if !job_types.contains(&job_type.as_str()) || !task.state.dispatcher_should_run() {
            continue;
        }
        scheduler.set_enabled(&task.id, false)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Workbench preferences
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_workbench_prefs(
    service: State<'_, AppService>,
) -> Result<BTreeMap<String, String>, String> {
    service.store()?.list_prefs().map_err(app_err)
}

#[tauri::command]
pub fn set_workbench_pref(
    service: State<'_, AppService>,
    key: String,
    value: String,
) -> Result<(), String> {
    service.store()?.set_pref(&key, &value).map_err(app_err)
}

// ---------------------------------------------------------------------------
// 待办
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_todos(
    service: State<'_, AppService>,
    include_archived: Option<bool>,
) -> Result<Vec<Todo>, String> {
    service
        .store()?
        .list_todos(include_archived.unwrap_or(false))
        .map_err(app_err)
}

#[tauri::command]
pub fn create_todo(service: State<'_, AppService>, draft: TodoDraft) -> Result<Todo, String> {
    service.store()?.create_todo(&draft).map_err(app_err)
}

#[tauri::command]
pub fn update_todo(
    service: State<'_, AppService>,
    expected_version: u64,
    todo: Todo,
) -> Result<Todo, String> {
    service
        .store()?
        .update_todo(expected_version, &todo)
        .map_err(app_err)
}

/// The one write path behind drag, the status select and completing or
/// restoring a todo. `status` is an id into the status catalog.
#[tauri::command]
pub fn set_todo_status(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
    status: String,
) -> Result<Todo, String> {
    service
        .store()?
        .set_todo_status(&id, expected_version, &status)
        .map_err(app_err)
}

#[tauri::command]
pub fn set_todo_archived(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
    archived: bool,
) -> Result<Todo, String> {
    service
        .store()?
        .set_todo_archived(&id, expected_version, archived)
        .map_err(app_err)
}

/// Delete a todo and its sub-items.
#[tauri::command]
pub fn delete_todo(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
) -> Result<(), String> {
    service
        .store()?
        .delete_todo(&id, expected_version)
        .map_err(app_err)
}

/// The same-column half of a drag: drop `id` into `status` ahead of
/// `before_id`, or at the end when it is `None`.
#[tauri::command]
pub fn reorder_todo(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
    status: String,
    before_id: Option<String>,
) -> Result<Todo, String> {
    service
        .store()?
        .reorder_todo(&id, expected_version, &status, before_id.as_deref())
        .map_err(app_err)
}

#[tauri::command]
pub fn set_todo_tags(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
    tag_ids: Vec<String>,
) -> Result<Todo, String> {
    service
        .store()?
        .set_todo_tags(&id, expected_version, &tag_ids)
        .map_err(app_err)
}

#[tauri::command]
pub fn set_todo_links(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
    link_ids: Vec<String>,
) -> Result<Todo, String> {
    service
        .store()?
        .set_todo_links(&id, expected_version, &link_ids)
        .map_err(app_err)
}

/// One action over many todos — the list view's selection bar.
#[tauri::command]
pub fn bulk_update_todos(
    service: State<'_, AppService>,
    ids: Vec<String>,
    op: TodoBulkOp,
) -> Result<Vec<Todo>, String> {
    service.store()?.bulk_update_todos(&ids, &op).map_err(app_err)
}

// ---------------------------------------------------------------------------
// 待办 · 状态目录
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_todo_statuses(service: State<'_, AppService>) -> Result<Vec<TodoStatusDef>, String> {
    service.store()?.list_todo_statuses().map_err(app_err)
}

#[tauri::command]
pub fn create_todo_status(
    service: State<'_, AppService>,
    draft: TodoStatusDraft,
) -> Result<TodoStatusDef, String> {
    service.store()?.create_todo_status(&draft).map_err(app_err)
}

#[tauri::command]
pub fn update_todo_status(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
    status: TodoStatusDef,
) -> Result<TodoStatusDef, String> {
    service
        .store()?
        .update_todo_status(&id, expected_version, &status)
        .map_err(app_err)
}

/// `reassign_to` moves the status's todos somewhere else first; without it a
/// status that still holds todos answers `status_in_use`.
#[tauri::command]
pub fn delete_todo_status(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
    reassign_to: Option<String>,
) -> Result<(), String> {
    service
        .store()?
        .delete_todo_status(&id, expected_version, reassign_to.as_deref())
        .map_err(app_err)
}

#[tauri::command]
pub fn reorder_todo_statuses(
    service: State<'_, AppService>,
    ids: Vec<String>,
) -> Result<Vec<TodoStatusDef>, String> {
    service.store()?.reorder_todo_statuses(&ids).map_err(app_err)
}

// ---------------------------------------------------------------------------
// 待办 · 标签与成员
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_todo_tags(service: State<'_, AppService>) -> Result<Vec<TodoTag>, String> {
    service.store()?.list_todo_tags().map_err(app_err)
}

#[tauri::command]
pub fn create_todo_tag(
    service: State<'_, AppService>,
    draft: TodoTagDraft,
) -> Result<TodoTag, String> {
    service.store()?.create_todo_tag(&draft).map_err(app_err)
}

#[tauri::command]
pub fn update_todo_tag(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
    name: String,
    color: String,
) -> Result<TodoTag, String> {
    service
        .store()?
        .update_todo_tag(&id, expected_version, &name, &color)
        .map_err(app_err)
}

#[tauri::command]
pub fn delete_todo_tag(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
) -> Result<(), String> {
    service
        .store()?
        .delete_todo_tag(&id, expected_version)
        .map_err(app_err)
}

#[tauri::command]
pub fn list_todo_people(service: State<'_, AppService>) -> Result<Vec<TodoPerson>, String> {
    service.store()?.list_todo_people().map_err(app_err)
}

#[tauri::command]
pub fn create_todo_person(
    service: State<'_, AppService>,
    name: String,
    color: String,
) -> Result<TodoPerson, String> {
    service.store()?.create_todo_person(&name, &color).map_err(app_err)
}

// ---------------------------------------------------------------------------
// 待办 · 附件、提醒与活动记录
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_todo_attachments(
    service: State<'_, AppService>,
    todo_id: String,
) -> Result<Vec<TodoAttachment>, String> {
    service
        .store()?
        .list_todo_attachments(&todo_id)
        .map_err(app_err)
}

/// Name a file the user picked and record it against the todo. The path comes
/// from [`pick_todo_attachment`]; nothing here reads or copies the file.
#[tauri::command]
pub fn create_todo_attachment(
    service: State<'_, AppService>,
    draft: TodoAttachmentDraft,
) -> Result<TodoAttachment, String> {
    service
        .store()?
        .create_todo_attachment(&draft)
        .map_err(app_err)
}

#[tauri::command]
pub fn update_todo_attachment(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
    name: String,
) -> Result<TodoAttachment, String> {
    service
        .store()?
        .update_todo_attachment(&id, expected_version, &name)
        .map_err(app_err)
}

/// Forget the attachment. The file is left on disk.
#[tauri::command]
pub fn delete_todo_attachment(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
) -> Result<(), String> {
    service
        .store()?
        .delete_todo_attachment(&id, expected_version)
        .map_err(app_err)
}

/// The native file picker. It runs in Rust because `tauri-plugin-dialog` is
/// deliberately not exposed to the webview (see `src-tauri/Cargo.toml`): the
/// GUI hands back a path, never a filesystem capability.
#[tauri::command]
pub fn pick_todo_attachment(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app.dialog().file().blocking_pick_file();
    Ok(picked.map(|file| file.to_string()))
}

#[tauri::command]
pub fn list_todo_reminders(
    service: State<'_, AppService>,
    todo_id: String,
) -> Result<Vec<TodoReminder>, String> {
    service
        .store()?
        .list_todo_reminders(&todo_id)
        .map_err(app_err)
}

#[tauri::command]
pub fn create_todo_reminder(
    service: State<'_, AppService>,
    draft: TodoReminderDraft,
) -> Result<TodoReminder, String> {
    service.store()?.create_todo_reminder(&draft).map_err(app_err)
}

#[tauri::command]
pub fn set_todo_reminder_enabled(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
    enabled: bool,
) -> Result<TodoReminder, String> {
    service
        .store()?
        .set_todo_reminder_enabled(&id, expected_version, enabled)
        .map_err(app_err)
}

#[tauri::command]
pub fn delete_todo_reminder(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
) -> Result<(), String> {
    service
        .store()?
        .delete_todo_reminder(&id, expected_version)
        .map_err(app_err)
}

#[tauri::command]
pub fn list_todo_activity(
    service: State<'_, AppService>,
    todo_id: String,
) -> Result<Vec<TodoActivity>, String> {
    service.store()?.list_todo_activity(&todo_id).map_err(app_err)
}

#[tauri::command]
pub fn add_todo_comment(
    service: State<'_, AppService>,
    todo_id: String,
    author_id: Option<String>,
    body: String,
) -> Result<TodoActivity, String> {
    service
        .store()?
        .add_todo_comment(&todo_id, author_id.as_deref(), &body)
        .map_err(app_err)
}

// ---------------------------------------------------------------------------
// 日历
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_calendar_events(service: State<'_, AppService>) -> Result<Vec<CalendarEvent>, String> {
    service.store()?.list_events().map_err(app_err)
}

#[tauri::command]
pub fn create_calendar_event(
    service: State<'_, AppService>,
    draft: CalendarEventDraft,
) -> Result<CalendarEvent, String> {
    service.store()?.create_event(&draft).map_err(app_err)
}

#[tauri::command]
pub fn update_calendar_event(
    service: State<'_, AppService>,
    expected_version: u64,
    event: CalendarEvent,
) -> Result<CalendarEvent, String> {
    service
        .store()?
        .update_event(expected_version, &event)
        .map_err(app_err)
}

#[tauri::command]
pub fn delete_calendar_event(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
) -> Result<(), String> {
    service
        .store()?
        .delete_event(&id, expected_version)
        .map_err(app_err)
}

// ---------------------------------------------------------------------------
// 成果中心
// ---------------------------------------------------------------------------
//
// Named `*_result*` rather than `*_artifact*`: the desktop shell already has a
// `list_artifacts` command for the files a *turn* produced, and the two must
// not be confused.

#[tauri::command]
pub fn list_results(
    service: State<'_, AppService>,
    include_archived: Option<bool>,
) -> Result<Vec<ArtifactView>, String> {
    service
        .store()?
        .list_artifacts(include_archived.unwrap_or(false))
        .map_err(app_err)
}

#[tauri::command]
pub fn create_result(
    service: State<'_, AppService>,
    draft: ArtifactDraft,
) -> Result<Artifact, String> {
    service.store()?.create_artifact(&draft).map_err(app_err)
}

#[tauri::command]
pub fn update_result(
    service: State<'_, AppService>,
    expected_version: u64,
    artifact: Artifact,
) -> Result<Artifact, String> {
    service
        .store()?
        .update_artifact(expected_version, &artifact)
        .map_err(app_err)
}

#[tauri::command]
pub fn set_result_flags(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
    favorite: Option<bool>,
    archived: Option<bool>,
) -> Result<Artifact, String> {
    service
        .store()?
        .set_artifact_flags(&id, expected_version, favorite, archived)
        .map_err(app_err)
}

/// Remove a record. The referenced file is never deleted.
#[tauri::command]
pub fn delete_result(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
) -> Result<(), String> {
    service
        .store()?
        .delete_artifact(&id, expected_version)
        .map_err(app_err)
}

/// Hand one artifact's stored file to the system default application.
///
/// The GUI names a record, never a path: [`AppStore::artifact_file_path`]
/// resolves the reference from the row, so the webview cannot ask the OS to
/// open a file the user never registered.
#[tauri::command]
pub fn open_result_file(
    app: AppHandle,
    service: State<'_, AppService>,
    id: String,
) -> Result<(), String> {
    let path = service.store()?.artifact_file_path(&id).map_err(app_err)?;
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|error| error.to_string())
}

/// Reveal the same stored file in the system file manager. Same guard as
/// [`open_result_file`]; nothing is read or executed.
#[tauri::command]
pub fn reveal_result_file(
    app: AppHandle,
    service: State<'_, AppService>,
    id: String,
) -> Result<(), String> {
    let path = service.store()?.artifact_file_path(&id).map_err(app_err)?;
    app.opener()
        .reveal_item_in_dir(std::path::Path::new(&path))
        .map_err(|error| error.to_string())
}
