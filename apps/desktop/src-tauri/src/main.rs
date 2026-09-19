//! Tauri shell. Its only job is to expose the core to the GUI.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use kodo_agent::Provider as AgentProvider;
use kodo_core::{session, settings, workspace};

use run::{Approvals, Runs, StartArgs};

mod run;
mod view;

use view::{ArchivedItemView, ProjectView, ProviderView, SessionRefView, SessionView, Workspace};

#[derive(serde::Serialize)]
struct CoreInfo {
    name: &'static str,
    version: &'static str,
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

#[tauri::command]
fn open_session(project: String, title: String) -> Result<SessionView, String> {
    let dir = sessions()?;
    let id = session::open(&dir, Path::new(&project), &title, session::now())
        .map_err(|error| error.to_string())?;
    load(&dir, &id)
}

#[tauri::command]
fn load_session(id: String) -> Result<SessionView, String> {
    load(&sessions()?, &id)
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
                    kodo_core::session::ItemKind::FileChange { changes } => {
                        for change in changes {
                            if seen_paths.insert(change.path.clone()) {
                                files_changed += 1;
                                added += change.added;
                                removed += change.removed;
                            }
                        }
                    }
                    kodo_core::session::ItemKind::AgentMessage { text, .. }
                        if !text.trim().is_empty() => {
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

#[tauri::command]
fn send_message(
    app: AppHandle,
    runs: State<'_, Runs>,
    approvals: State<'_, Approvals>,
    id: String,
    text: String,
    context: Option<Vec<String>>,
) -> Result<(), String> {
    let dir = sessions()?;
    let context_paths: Vec<String> = context.unwrap_or_default();
    // Reject paths that try to leave the project before any I/O.
    for path in &context_paths {
        if path.trim().is_empty() || path.contains("..") || std::path::Path::new(path).is_absolute()
        {
            return Err(format!("context path must stay inside the project: {path}"));
        }
    }
    session::record_ask_with_context(&dir, &id, session::now(), &text, &context_paths)
        .map_err(|error| error.to_string())?;
    let found = session::load(&dir, &id).map_err(|error| error.to_string())?;

    let settings_path = settings::settings_path();
    let read_setting = |key: &str| {
        settings_path
            .as_ref()
            .and_then(|path| settings::read(path, key))
    };

    let permission = run::permission_from_settings(read_setting("permission"));

    let provider = load_providers().ok().and_then(|list| {
        let active = read_setting("active-provider")
            .and_then(|raw| raw.parse::<usize>().ok())
            .unwrap_or(0);
        let chosen = list
            .get(active)
            .cloned()
            .or_else(|| list.iter().find(|p| p.has_key).cloned());
        chosen.and_then(|p| {
            // Keys live only in credentials; the shell re-reads them here.
            let creds = settings::credentials_path()?;
            let api_key = settings::read_credential(&creds, &p.id).unwrap_or_default();
            if api_key.is_empty() {
                return None;
            }
            Some(AgentProvider::new(
                p.template,
                api_key,
                p.endpoint,
                p.model_id
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(p.model),
            ))
        })
    });

    let max_output_tokens = read_setting("max-output-tokens")
        .and_then(|raw| raw.parse::<u32>().ok())
        .unwrap_or(4096);
    let extended_thinking = read_setting("extended-thinking").as_deref() == Some("true");
    // fallback-behavior=fail means no silent offline answers when a key exists.
    let fallback_to_local = read_setting("fallback-behavior").as_deref() != Some("fail");

    run::start(
        &app,
        runs.inner(),
        approvals.inner(),
        StartArgs {
            dir,
            id,
            project: found.project,
            message: text,
            context: context_paths,
            provider,
            permission,
            fallback_to_local,
            max_output_tokens,
            extended_thinking,
        },
    )
}

/// Lists project-relative files for the composer's Add context picker.
/// Honors the same ignore set as the agent's ContextManager (no node_modules, etc.).
#[tauri::command]
fn list_project_files(project: String, query: Option<String>) -> Result<Vec<String>, String> {
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
    let mut paths: Vec<String> = if query.trim().is_empty() {
        manager
            .file_map()
            .iter()
            .map(|entry| entry.path.clone())
            .collect()
    } else {
        manager
            .file_map()
            .iter()
            .filter(|entry| {
                entry
                    .path
                    .to_ascii_lowercase()
                    .contains(&query.to_ascii_lowercase())
            })
            .map(|entry| entry.path.clone())
            .collect()
    };
    paths.sort();
    paths.truncate(200);
    Ok(paths)
}

/// Validates that a context path is inside the project and returns a short preview.
/// Never returns file bodies to the React layer beyond this bounded preview.
#[tauri::command]
fn read_context_file(project: String, path: String) -> Result<String, String> {
    let root = std::path::Path::new(&project);
    if !root.is_dir() {
        return Err(format!("project is not a directory: {project}"));
    }
    if path.trim().is_empty() || path.contains("..") || std::path::Path::new(&path).is_absolute() {
        return Err(format!("path must stay inside the project: {path}"));
    }
    let manager = kodo_agent::ContextManager::new(
        root.to_path_buf(),
        kodo_agent::TurnContextBudget::default(),
    );
    // Range-read a small preview; agent pins the fuller range later.
    manager
        .read_range(&path, 1, 40, "context preview")
        .map(|span| span.snippet)
}

#[tauri::command]
fn stop_run(runs: State<'_, Runs>, approvals: State<'_, Approvals>, id: String) {
    runs.cancel(&id);
    approvals.clear_session(&id);
}

#[tauri::command]
fn respond_approval(approvals: State<'_, Approvals>, id: String, step: u32, approved: bool) {
    approvals.resolve(&id, step, approved);
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
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Runs::default())
        .manage(Approvals::default())
        .invoke_handler(tauri::generate_handler![
            core_info,
            workspace,
            add_project,
            create_project,
            remove_project,
            rename_project,
            reorder_project,
            pick_folder,
            setting,
            set_setting,
            git_branch,
            reveal_project,
            open_session,
            load_session,
            retitle_session,
            archive_session,
            restore_session,
            list_archived,
            load_providers,
            save_providers,
            send_message,
            stop_run,
            respond_approval,
            list_project_files,
            read_context_file,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Kodo");
}
