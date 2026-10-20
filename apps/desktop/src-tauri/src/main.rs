//! Tauri shell. Its only job is to expose the core to the GUI.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::{
    AppHandle, LogicalSize, Manager, State, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use kodo_agent as agent;
use kodo_core::{session, settings, traces, workspace};

use kodo_agent::mcp::{
    has_secrets, is_masked_value, merge_secrets, split_secrets, with_secret_values, McpClient,
    McpSecrets, McpServerConfig,
};

use run::{ApprovalChoice, Approvals, Runs, StartArgs};

mod knowledge;
mod platform;
mod run;
mod terminal;
mod view;
use knowledge::{
    knowledge_confirm_import, knowledge_create_base, knowledge_delete_document,
    knowledge_get_bindings, knowledge_import_text, knowledge_inspect_document,
    knowledge_list_bases, knowledge_list_documents, knowledge_pick_import, knowledge_prepare_turn,
    knowledge_search, knowledge_set_bindings, knowledge_set_enabled, knowledge_turn_citations,
    KnowledgeService,
};

mod memory;
use memory::{
    memory_disable, memory_doctor, memory_inspect, memory_list, memory_search, memory_status,
};

use terminal::{terminal_close, terminal_open, terminal_resize, terminal_write, Terminals};

mod apps;
use apps::{
    add_todo_comment, bulk_update_todos, create_calendar_event, create_result, create_todo,
    create_todo_attachment, create_todo_person, create_todo_reminder, create_todo_status,
    create_todo_tag, delete_calendar_event, delete_result, delete_todo, delete_todo_attachment,
    delete_todo_reminder, delete_todo_status, delete_todo_tag, get_workbench_prefs, list_apps,
    list_calendar_events, list_results, list_todo_activity, list_todo_attachments,
    list_todo_people, list_todo_reminders, list_todo_statuses, list_todo_tags, list_todos,
    open_result_file, pick_todo_attachment, reorder_todo, reorder_todo_statuses,
    reveal_result_file, set_app_enabled, set_app_favorite, set_app_links, set_app_order,
    set_result_flags, set_todo_archived, set_todo_links, set_todo_reminder_enabled,
    set_todo_status, set_todo_tags, set_workbench_pref, update_calendar_event, update_result,
    update_todo, update_todo_attachment, update_todo_status, update_todo_tag, AppService,
};

mod scheduler;
use scheduler::SchedulerService;

mod scheduler_cmds;
use scheduler_cmds::{
    archive_scheduled_task, cancel_task_run, create_scheduled_task, get_scheduled_task,
    list_job_types, list_scheduled_tasks, list_task_runs, preview_schedule, run_scheduled_task_now,
    set_scheduled_task_enabled, update_scheduled_task,
};

use view::{
    agent_view, skill_view, AgentView, ArchivedItemView, ModelOptionView, ProjectView,
    ProviderView, SessionRefView, SessionView, SkillView, Workspace,
};

/// The agent's mode, mapped onto the core's per-session tag.
fn core_mode(mode: kodo_agent::AgentMode) -> session::Mode {
    match mode {
        kodo_agent::AgentMode::Work => session::Mode::Work,
        kodo_agent::AgentMode::Code => session::Mode::Code,
    }
}

/// The reverse map, for feeding a session's tag back into a run.
fn agent_mode(mode: session::Mode) -> kodo_agent::AgentMode {
    match mode {
        session::Mode::Work => kodo_agent::AgentMode::Work,
        session::Mode::Code => kodo_agent::AgentMode::Code,
    }
}

#[derive(serde::Serialize)]
struct CoreInfo {
    name: &'static str,
    version: &'static str,
}

/// Embedded browser for the 浏览 inspector panel. macOS renders the child
/// webview via NSWindow.addChildWindow: so it tracks the parent content rect
/// natively; Windows/Linux get an owned/transient top-level window with
/// `skip_taskbar(true)` so the panel never appears as a separate taskbar entry.
/// The webview is reused across navigations — `WebviewWindow::navigate` updates
/// the URL without rebuilding the OS surface.
#[derive(Default)]
struct BrowseWebview(Mutex<Option<WebviewWindow>>);

const BROWSE_WEBVIEW_LABEL: &str = "browse";

/// Same scheme allowlist as `open_url` — javascript:/data:/file: never reach
/// the embedded webview or the OS browser. Returns the parsed `Url` on success
/// so the caller can hand it to `WebviewUrl::External` / `WebviewWindow::navigate`.
fn parse_safe_external_url(url: &str) -> Result<Url, String> {
    let trimmed = url.trim();
    let allowed = trimmed.starts_with("https://")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("mailto:");
    if !allowed {
        return Err(format!("refused url scheme: {trimmed}"));
    }
    Url::parse(trimmed).map_err(|error| format!("invalid url: {error}: {trimmed}"))
}

#[tauri::command]
fn core_info() -> CoreInfo {
    let info = kodo_core::info();
    CoreInfo {
        name: info.name,
        version: info.version,
    }
}

#[tauri::command]
fn workspace() -> Result<Workspace, String> {
    snapshot()
}

#[tauri::command]
fn add_project(path: String) -> Result<Workspace, String> {
    workspace::add(&log()?, Path::new(&path)).map_err(|error| error.to_string())?;
    snapshot()
}

#[tauri::command]
fn create_project(parent: String, name: String) -> Result<Workspace, String> {
    let made = workspace::create(Path::new(&parent), &name).map_err(|error| error.to_string())?;
    workspace::add(&log()?, &made).map_err(|error| error.to_string())?;
    snapshot()
}

#[tauri::command]
fn remove_project(path: String) -> Result<Workspace, String> {
    workspace::remove(&log()?, Path::new(&path)).map_err(|error| error.to_string())?;
    snapshot()
}

#[tauri::command]
fn rename_project(path: String, name: String) -> Result<Workspace, String> {
    workspace::rename(&log()?, Path::new(&path), &name).map_err(|error| error.to_string())?;
    snapshot()
}

#[tauri::command]
fn reorder_project(path: String, index: usize) -> Result<Workspace, String> {
    workspace::reorder(&log()?, Path::new(&path), index).map_err(|error| error.to_string())?;
    snapshot()
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .set_title("选择项目文件夹")
        .blocking_pick_folder()
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
}

/// OS file picker for "添加照片和文件" (photos and files from the filesystem).
#[tauri::command]
async fn pick_files(app: AppHandle) -> Option<Vec<String>> {
    let picked = app
        .dialog()
        .file()
        .set_title("添加照片和文件")
        .blocking_pick_files()?;
    let paths = picked
        .into_iter()
        .filter_map(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

#[tauri::command]
fn setting(key: String) -> Option<String> {
    settings::settings_path().and_then(|path| settings::read(&path, &key))
}

#[tauri::command]
fn set_setting(key: String, value: String) -> Result<(), String> {
    let path = settings::settings_path()
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep settings".to_owned())?;
    if value.contains('\n') || value.contains('\r') {
        return Err("setting values must be a single line".to_owned());
    }
    if key == "context-window-tokens" {
        kodo_shell::config::parse_context_window(&value).map_err(str::to_owned)?;
    }
    settings::append(&path, &key, &value).map_err(|error| error.to_string())
}

#[tauri::command]
fn git_branch(path: String) -> Option<String> {
    workspace::branch(Path::new(&path))
}

#[tauri::command]
fn reveal_project(app: AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(Path::new(&path))
        .map_err(|error| error.to_string())
}

/// Same idea as `reveal_project`, for the directory Kodo owns itself — the Help
/// menu's "打开状态目录" reaches it through `platform::reveal_state_dir`.
#[tauri::command]
fn reveal_state_dir(app: AppHandle) {
    platform::reveal_state_dir(&app);
}

/// The window's sidebar/inspector/theme state, pushed to the native menu so its
/// check marks mirror the app. The webview is the source of truth; this only
/// ever writes the menu.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MenuStateDto {
    sidebar: bool,
    inspector: bool,
    theme: String,
}

#[tauri::command]
fn set_menu_state(app: AppHandle, state: MenuStateDto) {
    platform::apply_menu_state(&app, state.sidebar, state.inspector, &state.theme);
}

/// Open a project file with the system default application — the escape
/// hatch for previews the viewer refuses (PDF, Office, oversized images,
/// binaries). Same path guards as `read_project_file`; content is never read.
#[tauri::command]
fn open_project_file(app: AppHandle, project: String, path: String) -> Result<(), String> {
    if path.trim().is_empty() || path.contains("..") {
        return Err(format!("invalid path: {path}"));
    }
    let candidate = Path::new(&path);
    if candidate.is_absolute() {
        return Err(format!("file outside project refused: {path}"));
    }
    let root = Path::new(&project);
    if !root.is_dir() {
        return Err(format!("project is not a directory: {project}"));
    }
    let full = root.join(candidate);
    if !full.is_file() {
        return Err(format!("file not found: {path}"));
    }
    app.opener()
        .open_path(full.to_string_lossy(), None::<&str>)
        .map_err(|error| error.to_string())
}

/// Open an http(s)/mailto URL with the system browser — links inside
/// assistant answers. The scheme allowlist is enforced here too, so the
/// webview can never hand an arbitrary URI to the OS.
#[tauri::command]
fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    let url = url.trim();
    let allowed =
        url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:");
    if !allowed {
        return Err(format!("refused url scheme: {url}"));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| error.to_string())
}

/// Create the embedded browser window (or navigate an existing one). The
/// webview is parented to the main window so it rides along on macOS via
/// `addChildWindow:`; on Win/Linux the owned window is `skip_taskbar(true)`
/// so it never appears as a separate OS window.
#[tauri::command]
fn browse_open(app: AppHandle, state: State<'_, BrowseWebview>, url: String) -> Result<(), String> {
    let parsed = parse_safe_external_url(&url)?;
    let mut guard = state.0.lock().map_err(|error| error.to_string())?;
    if let Some(existing) = guard.as_ref() {
        existing
            .navigate(parsed)
            .map_err(|error| error.to_string())?;
        return Ok(());
    }
    let main = app
        .get_webview_window("main")
        .ok_or_else(|| "main window missing".to_string())?;
    let webview =
        WebviewWindowBuilder::new(&app, BROWSE_WEBVIEW_LABEL, WebviewUrl::External(parsed))
            .parent(&main)
            .map_err(|error| error.to_string())?
            .skip_taskbar(true)
            .title("Browse")
            .inner_size(1.0, 1.0)
            .position(0.0, 0.0)
            .build()
            .map_err(|error| error.to_string())?;
    *guard = Some(webview);
    Ok(())
}

/// Navigate an existing browse webview to a new URL. The panel guarantees
/// `browse_open` ran first, so a missing webview here means the panel closed
/// mid-update — silently no-op rather than spawn a stray OS window.
#[tauri::command]
fn browse_set_url(state: State<'_, BrowseWebview>, url: String) -> Result<(), String> {
    let parsed = parse_safe_external_url(&url)?;
    let guard = state.0.lock().map_err(|error| error.to_string())?;
    match guard.as_ref() {
        Some(webview) => webview.navigate(parsed).map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

/// Destroy the embedded browser window (called when the inspector closes or
/// the panel switches away). Errors are ignored — the next `browse_open` will
/// recreate the webview from scratch.
#[tauri::command]
fn browse_close(state: State<'_, BrowseWebview>) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|error| error.to_string())?;
    if let Some(webview) = guard.take() {
        let _ = webview.destroy();
    }
    Ok(())
}

/// Resize and reposition the embedded browser window. The GUI measures the
/// `.ins-content--flush` rect (in window-content logical coords) and forwards
/// it here; the child webview sits inside the main window's content area.
#[tauri::command]
fn browse_set_position(
    state: State<'_, BrowseWebview>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let guard = state.0.lock().map_err(|error| error.to_string())?;
    let Some(webview) = guard.as_ref() else {
        return Ok(());
    };
    webview
        .set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)))
        .map_err(|error| error.to_string())?;
    webview
        .set_size(LogicalSize::new(width.max(1.0), height.max(1.0)))
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Completion ping from the webview. The OS attention request only fires while
/// the window sits in the background; the title marker is written here because
/// Tauri never syncs `document.title` back to the native window. Gaining focus
/// restores the plain title (see `on_window_event` on the builder).
#[tauri::command]
fn task_feedback(window: tauri::WebviewWindow) -> Result<(), String> {
    if !window.is_focused().unwrap_or(false) {
        let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
    window
        .set_title("Kodo — 任务完成")
        .map_err(|error| error.to_string())
}

/// Reveal a project file in the system file manager — same path guards as
/// `open_project_file`; nothing is read or executed.
#[tauri::command]
fn reveal_file(app: AppHandle, project: String, path: String) -> Result<(), String> {
    if path.trim().is_empty() || path.contains("..") {
        return Err(format!("invalid path: {path}"));
    }
    let candidate = Path::new(&path);
    if candidate.is_absolute() {
        return Err(format!("file outside project refused: {path}"));
    }
    let root = Path::new(&project);
    if !root.is_dir() {
        return Err(format!("project is not a directory: {project}"));
    }
    let full = root.join(candidate);
    if !full.is_file() {
        return Err(format!("file not found: {path}"));
    }
    app.opener()
        .reveal_item_in_dir(&full)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn open_session(project: String, title: String) -> Result<SessionView, String> {
    let dir = sessions()?;
    let config = kodo_shell::ShellConfig::load(kodo_shell::Overrides::default());
    let id = session::open(
        &dir,
        Path::new(&project),
        &title,
        session::now(),
        core_mode(config.mode),
    )
    .map_err(|error| error.to_string())?;
    load(&dir, &id)
}

#[tauri::command]
fn load_session(runs: State<'_, Runs>, id: String) -> Result<SessionView, String> {
    let dir = sessions()?;
    // A killed run must not read as Running forever (and never as Completed).
    // Recovery only stamps `interrupted` — items, changes, and verification stay.
    if !runs.has_run(&id) {
        let _ = session::recover_interrupted_session(&dir, &id, session::now());
    }
    load(&dir, &id)
}

/// Authoritative live state for a conversation the GUI is opening. The log is
/// the source of truth for finished steps; this fills the gap for a turn that
/// is still running (and for an approval that was requested while the user was
/// looking at another conversation).
#[tauri::command]
fn run_state(
    runs: State<'_, Runs>,
    approvals: State<'_, Approvals>,
    id: String,
) -> run::RunStateView {
    runs.state(&id, approvals.pending(&id))
}

#[tauri::command]
fn retitle_session(id: String, title: String) -> Result<SessionView, String> {
    let dir = sessions()?;
    session::retitle(&dir, &id, &title).map_err(|error| error.to_string())?;
    load(&dir, &id)
}

#[tauri::command]
fn archive_session(id: String) -> Result<Workspace, String> {
    let dir = sessions()?;
    session::archive(&dir, &id).map_err(|error| error.to_string())?;
    snapshot()
}

#[tauri::command]
fn restore_session(id: String) -> Result<Workspace, String> {
    let dir = sessions()?;
    session::restore(&dir, &id).map_err(|error| error.to_string())?;
    snapshot()
}

#[tauri::command]
fn delete_session(runs: State<'_, Runs>, id: String) -> Result<Workspace, String> {
    // A live run still owns the file — deleting under it would orphan the writer.
    if runs.has_run(&id) {
        return Err("session is running".to_string());
    }
    let dir = sessions()?;
    session::delete(&dir, &id).map_err(|error| error.to_string())?;
    snapshot()
}

/// Archived conversations with the summary fields the Archive table draws.
/// Summary is the last agent message (or ask), never a project name.
#[tauri::command]
fn list_archived() -> Result<Vec<ArchivedItemView>, String> {
    let dir = sessions()?;
    let projects = workspace::load(&log()?).map_err(|error| error.to_string())?;
    let known = session::list(&dir).map_err(|error| error.to_string())?;

    let mut out = Vec::new();
    for reference in known.into_iter().filter(|item| item.archived) {
        let full = session::load(&dir, &reference.id).map_err(|error| error.to_string())?;
        let mut model = String::new();
        let mut files_changed = 0u32;
        let mut added = 0u32;
        let mut removed = 0u32;
        let mut summary = String::new();
        let mut seen_paths = std::collections::HashSet::new();

        for turn in &full.turns {
            if !turn.ask.is_empty() {
                summary = turn.ask.clone();
            }
            for item in &turn.items {
                match &item.kind {
                    kodo_core::session::ItemKind::ModelCall { model: name, .. } => {
                        model = name.clone();
                    }
                    kodo_core::session::ItemKind::FileChange { changes, .. } => {
                        for change in changes {
                            // One path once; line deltas sum across steps/turns.
                            if seen_paths.insert(change.path.clone()) {
                                files_changed += 1;
                            }
                            added += change.added;
                            removed += change.removed;
                        }
                    }
                    kodo_core::session::ItemKind::AgentMessage { text, .. }
                        if !text.trim().is_empty() =>
                    {
                        summary = text.clone();
                    }
                    _ => {}
                }
            }
        }

        if summary.chars().count() > 80 {
            summary = summary.chars().take(80).collect::<String>() + "…";
        }

        let project_name = projects
            .iter()
            .find(|project| project.path == full.project)
            .map(|project| project.name.clone())
            .unwrap_or_else(|| {
                full.project
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| full.project.display().to_string())
            });

        out.push(ArchivedItemView {
            id: full.id,
            project: full.project.to_string_lossy().into_owned(),
            project_name,
            title: full.title,
            at: full.at,
            model,
            summary,
            files_changed,
            added,
            removed,
        });
    }
    Ok(out)
}

/// Provider metadata. Full API keys are never returned to the webview —
/// only a mask. Save with an empty `apiKey` to keep the stored secret.
#[tauri::command]
fn load_providers() -> Result<Vec<ProviderView>, String> {
    let settings_path = settings::settings_path()
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep providers".to_owned())?;
    let creds_path = settings::credentials_path()
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep credentials".to_owned())?;

    let raw = settings::read(&settings_path, "providers").unwrap_or_default();
    let mut list: Vec<ProviderView> = if raw.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&raw).map_err(|error| error.to_string())?
    };
    for provider in &mut list {
        let secret = settings::read_credential(&creds_path, &provider.id).unwrap_or_default();
        provider.api_key = mask_secret(&secret);
        provider.has_key = !secret.is_empty();
    }
    Ok(list)
}

fn mask_secret(secret: &str) -> String {
    if secret.is_empty() {
        return String::new();
    }
    if secret.len() <= 4 {
        return "••••".to_owned();
    }
    format!("••••{}", &secret[secret.len() - 4..])
}

#[tauri::command]
fn save_providers(providers: Vec<ProviderView>) -> Result<Vec<ProviderView>, String> {
    let settings_path = settings::settings_path()
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep providers".to_owned())?;
    let creds_path = settings::credentials_path()
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep credentials".to_owned())?;

    let mut secrets: Vec<(String, String)> = Vec::new();
    let mut metadata: Vec<ProviderView> = Vec::new();

    for provider in providers {
        let incoming = provider.api_key.trim().to_owned();
        let looks_masked = incoming.contains('•') || incoming.is_empty();
        let secret = if looks_masked {
            settings::read_credential(&creds_path, &provider.id).unwrap_or_default()
        } else {
            incoming
        };
        if !secret.is_empty() {
            secrets.push((provider.id.clone(), secret.clone()));
        }
        metadata.push(ProviderView {
            id: provider.id,
            name: provider.name,
            template: provider.template,
            api_key: String::new(),
            endpoint: provider.endpoint,
            model: provider.model.clone(),
            model_id: provider
                .model_id
                .clone()
                .or_else(|| Some(provider.model.clone())),
            display_name: provider.display_name.clone(),
            models: Some(provider.models.clone().unwrap_or_default()),
            has_key: !secret.is_empty(),
        });
    }

    settings::clear_credentials(&creds_path, &secrets).map_err(|error| error.to_string())?;

    let clean: Vec<serde_json::Value> = metadata
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "name": p.name,
                "template": p.template,
                "apiKey": "",
                "endpoint": p.endpoint,
                "model": p.model,
                "modelId": p.model_id,
                "displayName": p.display_name,
                "models": p.models,
            })
        })
        .collect();
    let json = serde_json::to_string(&clean).map_err(|error| error.to_string())?;
    settings::append(&settings_path, "providers", &json).map_err(|error| error.to_string())?;

    for provider in &mut metadata {
        let secret = settings::read_credential(&creds_path, &provider.id).unwrap_or_default();
        provider.api_key = mask_secret(&secret);
        provider.has_key = !secret.is_empty();
    }
    Ok(metadata)
}

/// One server's secret values from `credentials.log`, if stored and parsable.
fn read_mcp_secrets(creds_path: &Path, id: &str) -> Option<McpSecrets> {
    settings::read_secret(creds_path, "mcp", id).and_then(|raw| serde_json::from_str(&raw).ok())
}

/// Per-value mask of every secret half (env / header values).
fn mask_mcp_values(config: &mut McpServerConfig) {
    for map in [&mut config.env, &mut config.headers] {
        for value in map.values_mut() {
            *value = mask_secret(value);
        }
    }
}

/// MCP server configuration. Secret values (env / header values) are never
/// returned in cleartext — only per-value masks. Save with a masked value to
/// keep the stored secret, exactly like `save_providers` for API keys.
/// Runs async so the settings/credentials file I/O never touches the Tauri
/// main thread (same reasoning as `list_provider_models`).
#[tauri::command(async)]
fn load_mcp_servers() -> Result<Vec<McpServerConfig>, String> {
    let settings_path = settings::settings_path()
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep MCP servers".to_owned())?;
    let creds_path = settings::credentials_path()
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep credentials".to_owned())?;

    let raw = settings::read(&settings_path, "mcp-servers").unwrap_or_default();
    let mut list: Vec<McpServerConfig> = if raw.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&raw).map_err(|error| error.to_string())?
    };
    for config in &mut list {
        let stored = read_mcp_secrets(&creds_path, &config.id).unwrap_or_default();
        *config = merge_secrets(config, &stored);
        mask_mcp_values(config);
    }
    Ok(list)
}

/// Persist the full server list (settings copy holds empty secret values) and
/// answer with the re-masked view. Runs async so the credentials rewrite and
/// settings append never stall the Tauri main thread during the add flow.
#[tauri::command(async)]
fn save_mcp_servers(servers: Vec<McpServerConfig>) -> Result<Vec<McpServerConfig>, String> {
    let settings_path = settings::settings_path()
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep MCP servers".to_owned())?;
    let creds_path = settings::credentials_path()
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep credentials".to_owned())?;

    let mut keep: Vec<(String, String)> = Vec::new();
    let mut metadata: Vec<McpServerConfig> = Vec::new();

    for config in servers {
        let stored = read_mcp_secrets(&creds_path, &config.id).unwrap_or_default();
        let incoming = split_secrets(&config);
        // Each value resolves independently: a masked/blank placeholder keeps
        // the stored secret under the same key name, anything else is new.
        let mut resolved = McpSecrets::default();
        for (map_in, map_stored, map_out) in [
            (&incoming.env, &stored.env, &mut resolved.env),
            (&incoming.headers, &stored.headers, &mut resolved.headers),
        ] {
            for (key, value) in map_in {
                let secret = if is_masked_value(value) {
                    map_stored.get(key).cloned().unwrap_or_default()
                } else {
                    value.clone()
                };
                map_out.insert(key.clone(), secret);
            }
        }
        if has_secrets(&resolved) {
            keep.push((
                config.id.clone(),
                serde_json::to_string(&resolved).map_err(|e| e.to_string())?,
            ));
        }
        // The settings copy keeps the keys but drops every secret value.
        metadata.push(with_secret_values(&config, ""));
    }

    // Rebuild only the `mcp` namespace: provider secrets stay untouched.
    settings::clear_secrets(&creds_path, "mcp", &keep).map_err(|error| error.to_string())?;
    let json = serde_json::to_string(&metadata).map_err(|error| error.to_string())?;
    settings::append(&settings_path, "mcp-servers", &json).map_err(|error| error.to_string())?;

    for config in &mut metadata {
        let stored = read_mcp_secrets(&creds_path, &config.id).unwrap_or_default();
        *config = merge_secrets(config, &stored);
        mask_mcp_values(config);
    }
    Ok(metadata)
}

/// One advertised tool as the MCP page lists it — name and description only.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct McpToolView {
    name: String,
    description: String,
}

/// Connects the named server once and returns its tool list. The stored
/// secrets are merged **server-side** and never leave this process: the
/// webview sees tool names and descriptions only. A failure is the shell's
/// error string (transport, handshake or unknown id) — the page shows one
/// zh-CN line and the config stays editable. Runs async because the stdio
/// spawn can block for seconds (an `npx -y` first run downloads packages);
/// a sync command would freeze the whole UI on the Tauri main thread.
#[tauri::command(async)]
fn mcp_list_tools(id: String) -> Result<Vec<McpToolView>, String> {
    let settings_path = settings::settings_path()
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep MCP servers".to_owned())?;
    let creds_path = settings::credentials_path()
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep credentials".to_owned())?;

    let raw = settings::read(&settings_path, "mcp-servers").unwrap_or_default();
    let list: Vec<McpServerConfig> = if raw.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&raw).map_err(|error| error.to_string())?
    };
    let config = list
        .into_iter()
        .find(|config| config.id == id)
        .ok_or_else(|| format!("MCP server `{id}` is not configured"))?;
    let stored = read_mcp_secrets(&creds_path, &config.id).unwrap_or_default();
    let config = merge_secrets(&config, &stored);

    // Connect, list, drop: the transport (and any child process) lives only
    // for this call, exactly like one run's per-server client.
    let client = McpClient::connect(&config).map_err(|error| error.to_string())?;
    Ok(client
        .tools()
        .iter()
        .map(|tool| McpToolView {
            name: tool.name.clone(),
            description: tool.description.clone(),
        })
        .collect())
}

/// The directory user skills live in — the same one `run()` reads via
/// `RunRequest.user_skills_dir`.
fn skills_dir() -> Result<PathBuf, String> {
    settings::state_dir()
        .map(|dir| dir.join("skills"))
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep skills".to_owned())
}

/// The directory persona records live in — the same one `send_message`
/// reads via `agents::resolve_instructions` before a run starts.
fn agents_dir() -> Result<PathBuf, String> {
    settings::state_dir()
        .map(|dir| dir.join("agents"))
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep agents".to_owned())
}

/// The directory project skills live in — `<project>/.kodo/skills`, the same
/// one `run()` derives from the project root (`.kodo/` is excluded from
/// change tracking, so skills checked into a repo stay invisible to diffs).
fn project_skills_dir(project: &str) -> Result<PathBuf, String> {
    if project.trim().is_empty() {
        return Err("请先选择项目后再管理项目技能".to_owned());
    }
    Ok(PathBuf::from(project).join(".kodo").join("skills"))
}

/// Resolve the store for an optional `scope` (`user` is the default;
/// `project` needs a project path) plus the matching `SkillView.source`.
fn skill_store(
    scope: Option<&str>,
    project: Option<&str>,
) -> Result<(PathBuf, &'static str), String> {
    match scope {
        None | Some("user") => Ok((skills_dir()?, "user")),
        Some("project") => Ok((project_skills_dir(project.unwrap_or_default())?, "project")),
        Some(other) => Err(format!("未知技能来源 `{other}`")),
    }
}

/// Load one store's skills with their raw markdown (same loader rules the
/// agent crate applies on run).
fn load_store_skills(dir: &Path, source: &str) -> Vec<SkillView> {
    kodo_agent::load_user_skills(dir)
        .into_iter()
        .map(|spec| {
            let markdown =
                std::fs::read_to_string(dir.join(format!("{}.md", spec.name))).unwrap_or_default();
            skill_view(&spec, source, markdown)
        })
        .collect()
}

/// All skills for the Skills page: builtins first (read-only), then project
/// skills (when a project is selected), then the user's own files. Missing
/// directories are just empty tails. A name in both project and user stores
/// lists as two rows — the cards manage their own files, while `run()`'s
/// registry shadows the user copy with the project one.
#[tauri::command]
fn list_skills(project: Option<String>) -> Result<Vec<SkillView>, String> {
    let builtin = kodo_agent::AgentSkillRegistry::builtin();
    let mut out: Vec<SkillView> = builtin
        .skills()
        .iter()
        .map(|spec| {
            let markdown = kodo_agent::builtin_markdown(&spec.name)
                .unwrap_or_default()
                .to_owned();
            skill_view(spec, "builtin", markdown)
        })
        .collect();
    if let Some(root) = project.as_deref().filter(|p| !p.trim().is_empty()) {
        let dir = project_skills_dir(root)?;
        out.extend(load_store_skills(&dir, "project"));
    }
    let dir = skills_dir()?;
    out.extend(load_store_skills(&dir, "user"));
    Ok(out)
}

/// One skill's full detail for the preview modal. With an explicit `scope`
/// only that store is searched; without one the lookup mirrors the run-time
/// shadowing order: builtin → project → user.
#[tauri::command]
fn read_skill(
    name: String,
    scope: Option<String>,
    project: Option<String>,
) -> Result<SkillView, String> {
    if scope.is_some() {
        let (dir, source) = skill_store(scope.as_deref(), project.as_deref())?;
        return load_store_skills(&dir, source)
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| "技能不存在".to_owned());
    }
    let builtin = kodo_agent::AgentSkillRegistry::builtin();
    if let Some(spec) = builtin.get(&name) {
        let markdown = kodo_agent::builtin_markdown(&name)
            .unwrap_or_default()
            .to_owned();
        return Ok(skill_view(spec, "builtin", markdown));
    }
    let mut stores: Vec<(PathBuf, &str)> = Vec::new();
    if let Some(root) = project.as_deref().filter(|p| !p.trim().is_empty()) {
        stores.push((project_skills_dir(root)?, "project"));
    }
    stores.push((skills_dir()?, "user"));
    for (dir, source) in stores {
        if let Some(view) = load_store_skills(&dir, source)
            .into_iter()
            .find(|s| s.name == name)
        {
            return Ok(view);
        }
    }
    Err("技能不存在".to_owned())
}

/// Validate and persist one skill — `parse_skill` inside `save_user_skill` is
/// the validator; errors are zh-CN for inline display. `scope` picks the
/// store (`user` default, `project` writes `<project>/.kodo/skills`).
#[tauri::command]
fn save_skill(
    name: String,
    markdown: String,
    scope: Option<String>,
    project: Option<String>,
) -> Result<SkillView, String> {
    let (dir, source) = skill_store(scope.as_deref(), project.as_deref())?;
    let spec = kodo_agent::save_user_skill(&dir, &name, &markdown)?;
    Ok(skill_view(&spec, source, markdown))
}

/// Remove one skill file from the scoped store. Builtins are refused inside
/// the agent crate.
#[tauri::command]
fn delete_skill(
    name: String,
    scope: Option<String>,
    project: Option<String>,
) -> Result<(), String> {
    let (dir, _) = skill_store(scope.as_deref(), project.as_deref())?;
    kodo_agent::delete_user_skill(&dir, &name)
}

/// Load user-scope agents with their raw markdown. The 12 builtins are
/// appended by `list_agents` so this stays a pure user-dir reader.
fn load_store_agents(dir: &Path) -> Vec<AgentView> {
    load_store_agents_with_source(dir, "user")
}

/// Like `load_store_agents`, but tags every row with `source`. Used by
/// `read_agent` (project scope) and `list_agents` (project + user merge).
fn load_store_agents_with_source(dir: &Path, source: &str) -> Vec<AgentView> {
    kodo_agent::agents::load_agents_in(dir, false)
        .into_iter()
        .map(|spec| {
            let markdown =
                std::fs::read_to_string(dir.join(format!("{}.md", spec.name))).unwrap_or_default();
            agent_view(&spec, source, markdown)
        })
        .collect()
}

/// `<project>/.kodo/agents/` directory — empty path means "no project".
fn project_agents_dir(project: Option<&str>) -> Option<PathBuf> {
    project
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| Path::new(p).join(".kodo").join("agents"))
}

/// Map the 12 embedded parsers onto their wire view (with raw markdown
/// recovered via `builtin_markdown`).
fn builtin_agent_views() -> Vec<AgentView> {
    kodo_agent::agents::builtin_agents()
        .into_iter()
        .map(|spec| {
            let markdown = kodo_agent::agents::builtin_markdown(&spec.name)
                .unwrap_or_default()
                .to_owned();
            agent_view(&spec, "builtin", markdown)
        })
        .collect()
}

/// All persona records for the 插件 · 智能体 tab — builtins first (in
/// registry order), then project (when selected), then user files. Same
/// name in multiple scopes is deduplicated by the run-time resolution
/// order (project → user → builtin); here we keep one row per scope so
/// the UI can show each file's origin.
#[tauri::command]
fn list_agents(project: Option<String>) -> Result<Vec<AgentView>, String> {
    let mut views = builtin_agent_views();
    if let Some(dir) = project_agents_dir(project.as_deref()) {
        views.extend(load_store_agents_with_source(&dir, "project"));
    }
    views.extend(load_store_agents(&agents_dir()?));
    Ok(views)
}

/// One agent's full detail for the preview modal. Builtins win; user files
/// fill in otherwise.
#[tauri::command]
fn read_agent(name: String, project: Option<String>) -> Result<AgentView, String> {
    // Project scope shadows user + builtin (project may override a builtin).
    if let Some(dir) = project_agents_dir(project.as_deref()) {
        if let Some(view) = load_store_agents_with_source(&dir, "project")
            .into_iter()
            .find(|a| a.name == name)
        {
            return Ok(view);
        }
    }
    if let Some(view) = builtin_agent_views().into_iter().find(|a| a.name == name) {
        return Ok(view);
    }
    load_store_agents(&agents_dir()?)
        .into_iter()
        .find(|a| a.name == name)
        .ok_or_else(|| "智能体不存在".to_owned())
}

/// Validate and persist one persona — `parse_agent` inside `save_user_agent`
/// is the validator; builtin names are rejected there (zh-CN error). The
/// frontend shows the rejection inline.
#[tauri::command]
fn save_agent(name: String, markdown: String) -> Result<AgentView, String> {
    let dir = agents_dir()?;
    let spec = kodo_agent::agents::save_user_agent(&dir, &name, &markdown)?;
    Ok(agent_view(&spec, "user", markdown))
}

/// Remove one persona file. Builtin names are rejected in core. A missing
/// file is `智能体不存在`.
#[tauri::command]
fn delete_agent(name: String) -> Result<(), String> {
    kodo_agent::agents::delete_user_agent(&agents_dir()?, &name)
}

/// Auto-load the model list from the provider's `/models` endpoint.
/// `api_key` may be a freshly typed plaintext key; masked/empty falls back to
/// the stored secret for `provider_id`. Runs async so blocking ureq never
/// touches the Tauri main thread.
#[tauri::command(async)]
fn list_provider_models(
    template: String,
    endpoint: String,
    api_key: String,
    provider_id: Option<String>,
) -> Result<Vec<ModelOptionView>, String> {
    let incoming = api_key.trim();
    let key = if incoming.is_empty() || incoming.contains('•') {
        match provider_id {
            Some(id) if !id.is_empty() => {
                let creds_path = settings::credentials_path().ok_or_else(|| {
                    "HOME is not set, so there is nowhere to keep credentials".to_owned()
                })?;
                settings::read_credential(&creds_path, &id).unwrap_or_default()
            }
            _ => String::new(),
        }
    } else {
        incoming.to_owned()
    };
    let models = kodo_agent::fetch_models(&template, &endpoint, &key)?;
    let vision_default = kodo_agent::provider_template_vision(&template);
    Ok(models
        .into_iter()
        .map(|m| {
            let vision = m.vision_override.unwrap_or(vision_default);
            ModelOptionView {
                model_id: m.model_id,
                display_name: m.display_name,
                vision,
                suitable_for_long: m.suitable_for_long,
            }
        })
        .collect())
}

/// Paths the user attached explicitly (paste / drop with no shell-known
/// path) ride the multimodal inlining branch even when the saved provider
/// config still marks vision as off; the user's attach intent wins over
/// the model-capability gate. Empty by default.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn send_message(
    app: AppHandle,
    runs: State<'_, Runs>,
    approvals: State<'_, Approvals>,
    knowledge: State<'_, KnowledgeService>,
    apps: State<'_, AppService>,
    id: String,
    text: String,
    context: Option<Vec<String>>,
    project: Option<String>,
    agents: Option<Vec<String>>,
    // Optional budget-family override. `None` or `Some("")` = per-intent
    // default routing; `Some("summary")` lifts the run to read-only summary
    // caps. The desktop passes `Some("summary")` only from the "继续本轮"
    // affordance after a budget-exhausted partial delivery.
    budget_family: Option<String>,
    knowledge_token: Option<String>,
    knowledge_remote_approved: Option<bool>,
    force_inline: Option<Vec<String>>,
) -> Result<String, String> {
    let dir = sessions()?;

    // Provider, permission, mode and generation knobs come from the shared shell
    // config — the same settings.log/credentials.log the CLI reads.
    let config = kodo_shell::ShellConfig::load(kodo_shell::Overrides::default());

    // The agent chain must be resolved before session::open: a bad name must
    // not create a session (插件 · 智能体, resolved at send time). Empty
    // `agents` means "no persona for this turn" — fine, no validation needed.
    let personas: Vec<agent::PersonaBlock> = match agents.unwrap_or_default() {
        names if names.is_empty() => Vec::new(),
        names => {
            // The chain resolves against builtin → user → project (project
            // wins). Resolve against the project the message is bound to;
            // a no-project session cannot use project-scoped personas.
            let user_dir = agents_dir()?;
            kodo_agent::agents::resolve_chain(
                &user_dir,
                project_agents_dir(project.as_deref()).as_deref(),
                &names,
            )?
            .into_iter()
            .map(agent::PersonaBlock::from)
            .collect()
        }
    };

    let project_hint = project.clone();
    let created_session = id.trim().is_empty();
    let id = if id.trim().is_empty() {
        match project {
            Some(path) if !path.trim().is_empty() => session::open(
                &dir,
                Path::new(&path),
                "新对话",
                session::now(),
                core_mode(config.mode),
            )
            .map_err(|error| error.to_string())?,
            // No project selected: open a project-less session in either mode —
            // constrained Q&A, no tools (DESIGN.md §11).
            _ => session::open(
                &dir,
                Path::new(""),
                "新对话",
                session::now(),
                core_mode(config.mode),
            )
            .map_err(|error| error.to_string())?,
        }
    } else {
        id
    };

    let token = runs
        .begin(&id)
        .ok_or("this session already has a run in flight")?;
    let prepared = (|| -> Result<StartArgs, String> {
        let found = session::load(&dir, &id).map_err(|error| error.to_string())?;
        // Empty session project = no project (project-less conversations in both
        // modes). The session's own mode tag is the source of truth: it was
        // stamped at creation and never changes, unlike the global setting.
        let effective_project: Option<std::path::PathBuf> = if found.project.as_os_str().is_empty()
        {
            None
        } else {
            Some(found.project.clone())
        };

        let context_paths: Vec<String> = context.unwrap_or_default();
        for path in &context_paths {
            if effective_project.is_none() && !Path::new(path).is_absolute() {
                return Err("no project: context path must be absolute".into());
            }
            agent::tools::resolve_pinned_file(
                effective_project.as_deref().unwrap_or(Path::new(".")),
                path,
            )?;
        }
        let conversation_history = knowledge.safe_history(&dir, &found)?;
        let turn_seq = (found.turns.len() + 1) as u64;
        let knowledge_evidence = if let Some(preview) = knowledge_token.as_deref() {
            knowledge::ensure_enabled(&apps)?;
            let scope = knowledge::canonical_project(found.project.to_str())?;
            if scope != knowledge::canonical_project(project_hint.as_deref())? {
                return Err("scope_denied".into());
            }
            let (selection, hits) = knowledge.consume_turn(
                preview,
                &id,
                scope.as_deref(),
                &text,
                created_session,
                config.provider.as_ref(),
                knowledge_remote_approved.unwrap_or(false),
            )?;
            Some((scope, selection, hits))
        } else {
            None
        };
        session::record_ask_with_context(&dir, &id, session::now(), &text, &context_paths)
            .map_err(|error| error.to_string())?;
        let knowledge_context = if let Some((scope, selection, hits)) = knowledge_evidence {
            // Allocate the turn in the session before immutable citations so a
            // failed write cannot strand an occupied number for the next ask.
            let prepared = (|| {
                knowledge::mark_knowledge_turn(&dir, &id, turn_seq)?;
                knowledge.save_turn(
                    &id,
                    turn_seq,
                    scope.as_deref(),
                    &selection,
                    &hits,
                    created_session,
                )?;
                knowledge.context(&apps, &id, scope.as_deref(), &selection, &hits)
            })();
            match prepared {
                Ok(context) => Some(context),
                Err(error) => {
                    let _ = session::record_error(
                        &dir,
                        &id,
                        session::now(),
                        "knowledge context unavailable",
                    );
                    return Err(error);
                }
            }
        } else {
            None
        };
        let mut provider = config.provider;
        if knowledge_context.is_some() {
            if let Some(provider) = &mut provider {
                provider.fallbacks.clear();
            }
        }

        Ok(StartArgs {
            dir,
            id: id.clone(),
            project: effective_project,
            message: text,
            conversation_history,
            knowledge_context,
            context: context_paths,
            force_inline: force_inline.unwrap_or_default(),
            provider,
            permission: config.permission,
            mode: agent_mode(found.mode),
            fallback_to_local: config.fallback_to_local,
            max_output_tokens: config.max_output_tokens,
            extended_thinking: config.extended_thinking,
            local_trace: config.local_trace,
            mcp_servers: config.mcp_servers,
            // Legacy field: superseded by the chain in `personas`. Kept as
            // `None` so any older log replays still match the struct.
            agent_instructions: None,
            personas,
            budget_family,
        })
    })();
    let args = match prepared {
        Ok(args) => args,
        Err(error) => {
            runs.release(&id, &token);
            return Err(error);
        }
    };
    run::start(&app, runs.inner(), approvals.inner(), args, token)?;

    Ok(id)
}

/// Lists project-relative files for the composer's Add context picker.
/// Honors the same ignore set as the agent's ContextManager (no node_modules, etc.).
#[tauri::command]
fn list_project_files(
    project: String,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<String>, String> {
    let root = std::path::Path::new(&project);
    if !root.is_dir() {
        return Err(format!("project is not a directory: {project}"));
    }
    let mut manager = kodo_agent::ContextManager::new(
        root.to_path_buf(),
        kodo_agent::TurnContextBudget::default(),
    );
    manager
        .scan(&|| true)
        .map_err(|_| "cancelled while listing project files".to_owned())?;
    let query = query.unwrap_or_default();
    let needle = query.trim().to_ascii_lowercase();
    let entries: Vec<(String, bool)> = manager
        .file_map()
        .iter()
        .filter(|entry| needle.is_empty() || entry.path.to_ascii_lowercase().contains(&needle))
        .map(|entry| (entry.path.clone(), entry.from_git))
        .collect();
    // The composer picker caps at 200; the file viewer passes a larger limit.
    Ok(select_listed_paths(
        entries,
        limit.unwrap_or(200).clamp(1, 5000),
    ))
}

/// Tracked content first, then path order. The viewer re-sorts for display,
/// so this order only decides who survives truncation: real files are never
/// squeezed out by an untracked pile (venv backups, caches).
fn select_listed_paths(mut entries: Vec<(String, bool)>, limit: usize) -> Vec<String> {
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    entries.truncate(limit);
    entries.into_iter().map(|(path, _)| path).collect()
}

/// Validates that a context path exists and returns a short preview.
/// Relative paths stay inside the project; absolute paths (external files) are readable too.
#[tauri::command]
fn read_context_file(project: String, path: String) -> Result<String, String> {
    if path.trim().is_empty() || path.contains("..") {
        return Err(format!("invalid path: {path}"));
    }
    let root = std::path::Path::new(&project);
    if !Path::new(&path).is_absolute() && !root.is_dir() {
        return Err(format!("project is not a directory: {project}"));
    }
    let mut manager = kodo_agent::ContextManager::new(
        root.to_path_buf(),
        kodo_agent::TurnContextBudget::default(),
    );
    // Range-read a small preview; agent pins the fuller range later.
    manager
        .read_pinned_range(&path, 1, 40, "context preview")
        .map(|span| span.snippet)
}

/// A project root the user registered on the workspace list. Attachment
/// writes are confined to these so a script in the webview cannot turn
/// `save_pasted_blob` into an arbitrary-write primitive.
fn is_registered_project(root: &Path) -> bool {
    let Ok(canonical) = std::fs::canonicalize(root) else {
        return false;
    };
    let Some(log) = workspace::log_path() else {
        return false;
    };
    let Ok(projects) = workspace::load(&log) else {
        return false;
    };
    projects
        .iter()
        .any(|project| std::fs::canonicalize(&project.path).is_ok_and(|path| path == canonical))
}

/// Resolve where a pasted blob lands: `<registered-project>/.pasted` when the
/// renderer passed exactly that, otherwise the OS temp directory. The renderer
/// supplies the candidate path, so it is validated rather than trusted.
fn pasted_target_dir(app: &tauri::AppHandle, save_dir: Option<&str>) -> Result<PathBuf, String> {
    if let Some(dir) = save_dir.map(Path::new) {
        let named_pasted = dir.file_name().and_then(|name| name.to_str()) == Some(".pasted");
        if named_pasted && dir.parent().is_some_and(is_registered_project) {
            if std::fs::create_dir_all(dir).is_ok() {
                // Canonicalize after creating so a `.pasted` symlink planted
                // in the project cannot redirect the write outside it.
                if let Ok(canonical) = std::fs::canonicalize(dir) {
                    let stays_in_project = canonical.file_name().and_then(|name| name.to_str())
                        == Some(".pasted")
                        && canonical.parent().is_some_and(is_registered_project);
                    if stays_in_project {
                        return Ok(canonical);
                    }
                }
            }
        }
    }
    let dir = app
        .path()
        .temp_dir()
        .map_err(|error| error.to_string())?
        .join("kodo-pasted");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

/// Persist clipboard / drag-drop image bytes to disk so the composer can then
/// attach them. The clipboard API exposes `File` objects without a filesystem
/// path, so the renderer cannot use `pickFiles` for a paste — bytes have to
/// land on disk before the existing attachment probe + resolve pipeline can
/// run. 16 MiB mirrors the desktop image-preview cap; larger blobs are
/// rejected with a clear error so the renderer can surface a paste-size
/// warning.
#[tauri::command]
fn save_pasted_blob(
    app: tauri::AppHandle,
    bytes: Vec<u8>,
    suggested_filename: Option<String>,
    save_dir: Option<String>,
) -> Result<String, String> {
    // Project-rooted target directory the renderer pre-resolves (typically
    // `<project>/.pasted`). When `None`, unregistered or unwritable, fall
    // back to the OS temp dir so the paste always lands somewhere. Project
    // writes keep pasted images in agent `read_file` reach — the agent
    // otherwise can't cross the project boundary, leaving pasted images
    // unreachable.
    //
    // Must NOT be under `.kodo/` — `is_sensitive_relative_path` refuses to
    // open any path with a `.kodo` segment, so a `.kodo/.pasted/` would
    // be invisible to the agent's `read_file`. The renderer's
    // `pastedSaveDir` resolves this outside `.kodo/` at the project root.
    const MAX_BYTES: usize = 16 * 1024 * 1024;
    if bytes.is_empty() {
        return Err("pasted blob is empty".to_owned());
    }
    if bytes.len() > MAX_BYTES {
        return Err(format!(
            "pasted blob too large: {} bytes (cap {MAX_BYTES})",
            bytes.len()
        ));
    }
    // Sanitize the suggested filename: strip path separators, keep the
    // extension when present (image probes key off extension).
    let raw_name = suggested_filename.unwrap_or_else(|| "pasted".to_owned());
    let safe_name: String = raw_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe_name = if safe_name.is_empty() {
        "pasted".to_owned()
    } else {
        safe_name
    };
    let dir = pasted_target_dir(&app, save_dir.as_deref())?;
    // Append a monotonic counter to dodge same-millisecond collisions across
    // rapid pastes (e.g. two screenshots in a row); `create_new` keeps a
    // colliding write from overwriting an existing file.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    for attempt in 0..100 {
        let name = if attempt == 0 {
            format!("pasted-{stamp}-{safe_name}")
        } else {
            format!("pasted-{stamp}-{attempt}-{safe_name}")
        };
        let path = dir.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(&bytes).map_err(|error| error.to_string())?;
                return Ok(path.to_string_lossy().into_owned());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("could not allocate a unique pasted filename".to_owned())
}

/// Attachment probe for the composer cards: kind by extension/content, plus a
/// small preview (base64 image ≤ 4MB, or the first lines of a text file).
/// Unlike `read_context_file` this accepts every file type — images, PDFs and
/// Office documents attach fine and simply carry no inline preview.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ContextProbe {
    kind: String,
    name: String,
    size: u64,
    preview: Option<String>,
}

fn context_kind(path: &Path, head: Option<&[u8]>) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "avif" | "heic" => {
            return "image"
        }
        "pdf" => return "pdf",
        "doc" | "docx" | "rtf" | "odt" | "xls" | "xlsx" | "ods" | "ppt" | "pptx" | "odp" => {
            return "office"
        }
        "txt" | "md" | "markdown" | "json" | "jsonl" | "yml" | "yaml" | "toml" | "xml" | "html"
        | "css" | "js" | "jsx" | "ts" | "tsx" | "rs" | "py" | "go" | "java" | "kt" | "c" | "h"
        | "cpp" | "hpp" | "sh" | "zsh" | "bash" | "sql" | "log" | "csv" | "tsv" | "ini" | "cfg"
        | "conf" => return "text",
        _ => {}
    }
    // Unknown extension: NUL byte in the head means binary → "other".
    match head {
        Some(bytes) if bytes.contains(&0) => "other",
        Some(_) => "text",
        None => "other",
    }
}

fn image_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "heic" => "image/heic",
        _ => "image/png",
    }
}

/// Base64 without a dependency: small and local, so the alphabet is enough.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Full text of a project-relative file for the file viewer (line-capped).
/// Project-scoped only — external/absolute paths are attach-only, never read
/// here. Binary files are refused.
#[tauri::command]
fn read_project_file(project: String, path: String) -> Result<String, String> {
    if path.trim().is_empty() || path.contains("..") {
        return Err(format!("invalid path: {path}"));
    }
    let candidate = Path::new(&path);
    if candidate.is_absolute() {
        return Err(format!("file outside project refused: {path}"));
    }
    let root = Path::new(&project);
    if !root.is_dir() {
        return Err(format!("project is not a directory: {project}"));
    }
    let full = kodo_agent::tools::resolve_in_project(root, &path)?;
    if !full.is_file() {
        return Err(format!("file not found: {path}"));
    }
    let text = std::fs::read_to_string(&full).map_err(|e| format!("read failed: {e}"))?;
    if text.bytes().take(2048).any(|b| b == 0) {
        return Err(format!("binary file refused: {path}"));
    }
    const MAX_LINES: usize = 4000;
    let mut out: Vec<&str> = Vec::new();
    let mut truncated = false;
    for (index, line) in text.lines().enumerate() {
        if index >= MAX_LINES {
            truncated = true;
            break;
        }
        out.push(line);
    }
    if truncated {
        out.push("……（文件过大，已截断显示前 4000 行）");
    }
    Ok(out.join("\n"))
}

#[tauri::command]
fn probe_context_file(project: String, path: String) -> Result<ContextProbe, String> {
    if path.trim().is_empty() || path.contains("..") {
        return Err(format!("invalid path: {path}"));
    }
    let full = kodo_agent::tools::resolve_pinned_file(Path::new(&project), &path)?;
    if !full.is_file() {
        return Err(format!("file not found: {path}"));
    }
    let meta = std::fs::metadata(&full).map_err(|e| format!("read failed: {e}"))?;
    let size = meta.len();
    let name = full
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());

    // Sniff the head once: kind for unknown extensions, binary check for text.
    let head = std::fs::File::open(&full).ok().and_then(|mut f| {
        use std::io::Read;
        let mut buf = [0u8; 8192];
        let n = f.read(&mut buf).ok()?;
        Some(buf[..n].to_vec())
    });
    let kind = context_kind(&full, head.as_deref());

    let preview = match kind {
        "image" if size <= 4 * 1024 * 1024 => std::fs::read(&full).ok().map(|bytes| {
            format!(
                "data:{};base64,{}",
                image_mime(&full),
                base64_encode(&bytes)
            )
        }),
        "text" => std::fs::read_to_string(&full)
            .ok()
            .map(|text| text.lines().take(40).collect::<Vec<_>>().join("\n")),
        _ => None,
    };

    Ok(ContextProbe {
        kind: kind.to_owned(),
        name,
        size,
        preview,
    })
}

#[tauri::command]
fn stop_run(runs: State<'_, Runs>, approvals: State<'_, Approvals>, id: String) {
    runs.cancel(&id);
    approvals.clear_session(&id);
}

#[tauri::command]
fn respond_approval(
    approvals: State<'_, Approvals>,
    id: String,
    step: u32,
    run_id: Option<u64>,
    approved: bool,
    session_wide: Option<bool>,
) -> Result<(), String> {
    let choice = ApprovalChoice::from_parts(approved, session_wide.unwrap_or(false));
    let generation = run_id.ok_or("approval run id is missing or expired")?;
    if !approvals.resolve(&id, generation, step, choice) {
        return Err("approval has expired".into());
    }
    Ok(())
}

/// Per-file unified diffs for the turn's Kodo changes (empty when none).
/// Each entry reports pre-existing user dirt and live undo-conflict state.
#[tauri::command]
fn turn_changes(project: String, id: String) -> Result<Vec<view::TurnChangeView>, String> {
    let root = Path::new(&project);
    let Some(changeset) = kodo_agent::load_changeset(root, &id)? else {
        // No changeset: the turn recorded no file edits — an empty list, not
        // an error (the Files panel shows 尚未产生文件修改 either way).
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for path in changeset.kodo_changes() {
        let user_preexisting = changeset.was_pre_existing(path);
        let undo_state = match changeset.undo_file_state(root, path) {
            kodo_agent::UndoFileState::Clean => "clean",
            kodo_agent::UndoFileState::AlreadyBaseline => "already_baseline",
            kodo_agent::UndoFileState::Diverged => "diverged",
            kodo_agent::UndoFileState::Missing => "missing",
        };
        out.push(view::TurnChangeView {
            path: path.clone(),
            diff: changeset.diffs.get(path).cloned().unwrap_or_default(),
            user_preexisting,
            undo_state: undo_state.to_owned(),
            conflict: undo_state == "diverged" || undo_state == "missing",
        });
    }
    Ok(out)
}

/// Undo only Kodo's changes for this session. Never touches user-only edits
/// and never snapshot-overwrites post-turn user edits (returns conflicts).
#[tauri::command]
fn undo_turn(project: String, id: String) -> Result<view::UndoReportView, String> {
    let report = kodo_agent::undo_session_changes(Path::new(&project), &id)?;
    Ok(view::UndoReportView {
        restored: report.restored,
        conflicts: report
            .conflicts
            .into_iter()
            .map(|c| view::UndoConflictView {
                path: c.path,
                reason: c.reason.label().to_owned(),
                message: c.message,
            })
            .collect(),
    })
}

// ---------------------------------------------------------------------------
// Trace artifact commands (Phase D — transparency UI)
//
// `list_artifacts`, `load_llm_io` and `load_file_span` give the trace page
// read-only access to the bodies that the session log only stores chips for.
// Every command is wrapped in a friendly error string — the webview turns
// that into a toast, not a stack trace.
// ---------------------------------------------------------------------------

#[tauri::command]
fn list_artifacts(session_id: String, turn_seq: u32) -> Result<Vec<view::ArtifactRefView>, String> {
    traces::list_artifacts(&session_id, turn_seq)
        .map(|items| items.into_iter().map(view::ArtifactRefView::from).collect())
        .map_err(|error| format!("list_artifacts failed: {error}"))
}

#[tauri::command]
fn load_llm_io(
    session_id: String,
    turn_seq: u32,
    ref_path: String,
) -> Result<view::LlmIoView, String> {
    traces::read_llm_io(&session_id, turn_seq, &ref_path)
        .map(view::LlmIoView::from)
        .map_err(|error| format!("load_llm_io failed: {error}"))
}

#[tauri::command]
fn load_file_span(
    session_id: String,
    turn_seq: u32,
    ref_path: String,
) -> Result<view::FileSpanView, String> {
    traces::read_file_span(&session_id, turn_seq, &ref_path)
        .map(view::FileSpanView::from)
        .map_err(|error| format!("load_file_span failed: {error}"))
}

/// Cheap probe so the trace page can decide whether to show its tab — keeps
/// the cold path off the webview thread for sessions with no capture yet.
#[tauri::command]
fn turn_has_artifacts(session_id: String, turn_seq: u32) -> Result<bool, String> {
    Ok(traces::turn_has_artifacts(&session_id, turn_seq))
}

fn log() -> Result<PathBuf, String> {
    workspace::log_path()
        .ok_or_else(|| "HOME is not set, so there is nowhere to keep the project list".to_owned())
}

fn sessions() -> Result<PathBuf, String> {
    session::dir().ok_or_else(|| "HOME is not set, so there is nowhere to keep sessions".to_owned())
}

fn load(dir: &Path, id: &str) -> Result<SessionView, String> {
    session::load(dir, id)
        .map(SessionView::from)
        .map_err(|error| error.to_string())
}

fn snapshot() -> Result<Workspace, String> {
    let projects = workspace::load(&log()?).map_err(|error| error.to_string())?;
    let known = session::list(&sessions()?).map_err(|error| error.to_string())?;
    Ok(Workspace {
        projects: projects.into_iter().map(ProjectView::from).collect(),
        sessions: known.into_iter().map(SessionRefView::from).collect(),
    })
}

fn main() {
    // App restart: any Running persisted session becomes Interrupted here —
    // never Completed. No live runs exist yet, so the live set is empty.
    if let Some(dir) = session::dir() {
        let live = std::collections::HashSet::new();
        let _ = session::recover_interrupted(&dir, &live);
    }

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Runs::default())
        .manage(Approvals::default())
        .manage(BrowseWebview::default())
        .manage(AppService::boot())
        .manage(KnowledgeService::boot())
        .manage(Terminals::default())
        .setup(|app| {
            // Native surfaces come up first: the status item is independent of
            // both the scheduler and the workbench, so it has to exist even
            // when one of them fails to boot and returns early below.
            platform::build_tray(app.handle())?;
            // The scheduler needs a typed `AppHandle` to install the tick
            // thread and a `state_dir` to write its store under. Both
            // come from the desktop's `kodo_core::settings` module; the
            // Tauri binding has nothing to add to the path.
            let runs: tauri::State<'_, Runs> = app.state();
            let approvals: tauri::State<'_, Approvals> = app.state();
            let state_dir = match settings::state_dir() {
                Some(dir) => dir,
                None => {
                    eprintln!("kodo scheduler: state_dir unavailable, scheduler is disabled");
                    return Ok(());
                }
            };
            let service = match SchedulerService::boot(
                app.handle().clone(),
                state_dir,
                runs.inner().clone(),
                approvals.inner().clone(),
            ) {
                Ok(service) => service,
                Err(error) => {
                    eprintln!("kodo scheduler: boot failed: {error}");
                    return Ok(());
                }
            };
            app.manage(service);
            Ok(())
        })
        // The completion marker from `task_feedback` clears as soon as the
        // window is back in front — focus lives on the native side, so the
        // webview could never reset the title itself.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(true) = event {
                let _ = window.set_title("Kodo");
            }
            // Closing hides rather than quits: a run in flight outlives the
            // window, and the status item is the way back in. macOS-only —
            // `RunEvent::Reopen` below is the matching dock-icon path, and
            // neither exists elsewhere.
            #[cfg(target_os = "macos")]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            core_info,
            workspace,
            add_project,
            create_project,
            remove_project,
            rename_project,
            reorder_project,
            pick_folder,
            pick_files,
            setting,
            set_setting,
            git_branch,
            reveal_project,
            open_project_file,
            open_url,
            browse_open,
            browse_set_url,
            browse_close,
            browse_set_position,
            task_feedback,
            reveal_file,
            open_session,
            load_session,
            run_state,
            retitle_session,
            archive_session,
            restore_session,
            delete_session,
            list_archived,
            load_providers,
            save_providers,
            list_provider_models,
            load_mcp_servers,
            save_mcp_servers,
            mcp_list_tools,
            list_skills,
            read_skill,
            save_skill,
            delete_skill,
            list_agents,
            read_agent,
            save_agent,
            delete_agent,
            send_message,
            knowledge_list_bases,
            knowledge_create_base,
            knowledge_list_documents,
            knowledge_inspect_document,
            knowledge_import_text,
            knowledge_pick_import,
            knowledge_confirm_import,
            knowledge_set_enabled,
            knowledge_delete_document,
            knowledge_search,
            knowledge_get_bindings,
            knowledge_set_bindings,
            knowledge_prepare_turn,
            knowledge_turn_citations,
            stop_run,
            respond_approval,
            list_project_files,
            read_context_file,
            read_project_file,
            probe_context_file,
            save_pasted_blob,
            turn_changes,
            undo_turn,
            list_artifacts,
            load_llm_io,
            load_file_span,
            turn_has_artifacts,
            memory_status,
            memory_list,
            memory_search,
            memory_inspect,
            memory_disable,
            memory_doctor,
            list_scheduled_tasks,
            get_scheduled_task,
            create_scheduled_task,
            update_scheduled_task,
            set_scheduled_task_enabled,
            archive_scheduled_task,
            preview_schedule,
            run_scheduled_task_now,
            list_task_runs,
            cancel_task_run,
            list_job_types,
            list_apps,
            set_app_enabled,
            set_app_favorite,
            set_app_order,
            set_app_links,
            get_workbench_prefs,
            set_workbench_pref,
            list_todos,
            create_todo,
            update_todo,
            set_todo_status,
            set_todo_archived,
            delete_todo,
            reorder_todo,
            set_todo_tags,
            set_todo_links,
            bulk_update_todos,
            list_todo_statuses,
            create_todo_status,
            update_todo_status,
            delete_todo_status,
            reorder_todo_statuses,
            list_todo_tags,
            create_todo_tag,
            update_todo_tag,
            delete_todo_tag,
            list_todo_people,
            create_todo_person,
            list_todo_attachments,
            create_todo_attachment,
            update_todo_attachment,
            delete_todo_attachment,
            pick_todo_attachment,
            list_todo_reminders,
            create_todo_reminder,
            set_todo_reminder_enabled,
            delete_todo_reminder,
            list_todo_activity,
            add_todo_comment,
            list_calendar_events,
            create_calendar_event,
            update_calendar_event,
            delete_calendar_event,
            list_results,
            create_result,
            update_result,
            set_result_flags,
            delete_result,
            open_result_file,
            reveal_result_file,
            reveal_state_dir,
            set_menu_state,
            terminal_open,
            terminal_write,
            terminal_resize,
            terminal_close,
        ]);

    // The app menu bar exists only on macOS: `Menu::default` was the only thing
    // that ever installed one, so Linux has never had one and must not gain one
    // by accident. The tray is declared in `tauri.conf.json` and so is present
    // everywhere — which is why the menu *event* handler is not gated: it also
    // serves the tray menu.
    #[cfg(target_os = "macos")]
    let builder = builder.menu(platform::build_menu);

    let app = builder
        .on_menu_event(platform::on_menu_event)
        .on_tray_icon_event(platform::on_tray_event)
        .build(tauri::generate_context!())
        .expect("failed to build Kodo");

    app.run(|app, event| {
        if matches!(&event, tauri::RunEvent::Exit) {
            if let Some(service) = app.try_state::<SchedulerService>() {
                service.shutdown();
            }
        }
        // Clicking the dock icon while the window is hidden has to bring it
        // back; without this the hide-to-tray close button would be a one-way
        // door.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } = event
        {
            platform::show_main(app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{select_listed_paths, MenuStateDto};

    #[test]
    fn listing_keeps_tracked_content_under_truncation() {
        let entries = vec![
            (".venv.broken-backup/lib/a.py".to_string(), false),
            ("docs/z.md".to_string(), true),
            ("docs/a.md".to_string(), true),
            ("apple.rs".to_string(), false),
        ];
        // The pile of untracked junk must not crowd tracked files out;
        // untracked entries keep plain path order among themselves
        // (".venv.broken-backup/…" sorts before "apple.rs").
        assert_eq!(
            select_listed_paths(entries, 3),
            vec!["docs/a.md", "docs/z.md", ".venv.broken-backup/lib/a.py"]
        );
    }

    #[test]
    fn listing_applies_limit_after_prioritising_tracked() {
        let entries = vec![
            ("untracked.txt".to_string(), false),
            ("b.md".to_string(), true),
            ("a.md".to_string(), true),
        ];
        assert_eq!(select_listed_paths(entries, 2), vec!["a.md", "b.md"]);
    }

    /// The wire contract with `api.ts`'s `setMenuState`. Worth a test because
    /// the frontend fires it as a `void` — a field renamed on either side would
    /// just stop moving the menu's check marks, with nothing to show for it.
    #[test]
    fn menu_state_payload_matches_the_frontend() {
        let state: MenuStateDto =
            serde_json::from_str(r#"{"sidebar":true,"inspector":false,"theme":"dark"}"#).unwrap();
        assert!(state.sidebar);
        assert!(!state.inspector);
        assert_eq!(state.theme, "dark");
    }
}
