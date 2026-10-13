//! Tauri command handlers that surface the scheduler to the GUI.
//!
//! Each command is a thin wrapper around [`SchedulerService`] that the
//! `invoke_handler!` macro picks up. The commands return
//! `Result<T, String>` so the frontend sees a stable `error: string`
//! shape, where the string is the [`kodo_shell::SchedulerError`] code.
//!
//! All commands are synchronous; the heavy work (the tick thread, the
//! executor threads) lives behind the service.

use tauri::State;

use kodo_core::scheduler::{Schedule, ScheduledTask, TaskRun, UnixSeconds};

use crate::scheduler::SchedulerService;

#[tauri::command]
pub fn list_scheduled_tasks(
    service: State<'_, SchedulerService>,
    include_archived: Option<bool>,
) -> Result<Vec<ScheduledTask>, String> {
    service.list_tasks(include_archived.unwrap_or(false))
}

#[tauri::command]
pub fn get_scheduled_task(
    service: State<'_, SchedulerService>,
    id: String,
) -> Result<Option<ScheduledTask>, String> {
    service.get_task(&id)
}

#[tauri::command]
pub fn create_scheduled_task(
    service: State<'_, SchedulerService>,
    task: ScheduledTask,
) -> Result<ScheduledTask, String> {
    service.create_task(task)
}

#[tauri::command]
pub fn update_scheduled_task(
    service: State<'_, SchedulerService>,
    expected_revision: u64,
    task: ScheduledTask,
) -> Result<ScheduledTask, String> {
    service.update_task(expected_revision, task)
}

#[tauri::command]
pub fn set_scheduled_task_enabled(
    service: State<'_, SchedulerService>,
    id: String,
    enabled: bool,
) -> Result<ScheduledTask, String> {
    service.set_enabled(&id, enabled)
}

#[tauri::command]
pub fn archive_scheduled_task(
    service: State<'_, SchedulerService>,
    id: String,
) -> Result<(), String> {
    service.archive_task(&id)
}

#[tauri::command]
pub fn preview_schedule(
    service: State<'_, SchedulerService>,
    schedule: Schedule,
    after: Option<UnixSeconds>,
) -> Result<Vec<UnixSeconds>, String> {
    service.preview_schedule(&schedule, after.unwrap_or(0))
}

#[tauri::command]
pub fn run_scheduled_task_now(
    service: State<'_, SchedulerService>,
    id: String,
) -> Result<TaskRun, String> {
    service.run_now(&id)
}

#[tauri::command]
pub fn list_task_runs(
    service: State<'_, SchedulerService>,
    task_id: String,
    limit: Option<usize>,
) -> Result<Vec<TaskRun>, String> {
    service.list_runs(&task_id, limit.unwrap_or(50))
}

#[tauri::command]
pub fn cancel_task_run(service: State<'_, SchedulerService>, id: String) -> Result<bool, String> {
    service.cancel_run(&id)
}

#[tauri::command]
pub fn list_job_types(service: State<'_, SchedulerService>) -> Vec<String> {
    service.list_job_types()
}
