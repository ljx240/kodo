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
    CalendarEventDraft, Todo, TodoDraft, TodoStatus,
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

    pub(crate) fn store(&self) -> Result<&Arc<AppStore>, String> {
        self.store
            .as_ref()
            .ok_or_else(|| AppError::StorageUnavailable.code().to_owned())
    }

    #[cfg(test)]
    pub(crate) fn for_test(store: AppStore) -> Self {
        Self {
            store: Some(Arc::new(store)),
        }
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
/// restoring a todo.
#[tauri::command]
pub fn set_todo_status(
    service: State<'_, AppService>,
    id: String,
    expected_version: u64,
    status: TodoStatus,
) -> Result<Todo, String> {
    service
        .store()?
        .set_todo_status(&id, expected_version, status)
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
