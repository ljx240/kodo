//! One turn: session envelopes + agent run + renderer + approver wiring.
//!
//! Mirrors the desktop `run.rs` sink contract: record to the session log
//! first, then render; a failed envelope write aborts the turn.

use std::fs;
use std::io::{Stderr, Stdout};
use std::path::{Path, PathBuf};

use kodo_agent::{tools, Permission, RunRequest, SinkEvent};
use kodo_core::session;
use kodo_shell::{Logged, ShellConfig, TurnLogger, TurnOutcome};

use crate::approve::Approver;
use crate::interrupt;
use crate::render::Renderer;

/// How a turn ended — drives the process exit code in one-shot mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    Completed,
    Stopped,
    Failed,
}

pub fn permission_label(permission: Permission) -> &'static str {
    match permission {
        Permission::Ask => "ask",
        Permission::Auto => "auto",
        Permission::Full => "full",
    }
}

/// Session title from the first message: whitespace-flattened, 40 chars max.
fn session_title(message: &str) -> String {
    let flat = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return "新对话".to_owned();
    }
    let truncated: String = flat.chars().take(40).collect();
    if flat.chars().count() > 40 {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// Whether two paths refer to the same project (canonicalized when they exist).
pub fn same_project(a: &Path, b: &Path) -> bool {
    let a = fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let b = fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    a == b
}

pub struct TurnContext {
    pub project: PathBuf,
    pub session_dir: PathBuf,
    pub session_id: Option<String>,
    /// Live permission for this process — starts from settings/`--permission`;
    /// `[a]` at a prompt and `/permission` change it for subsequent turns.
    pub permission: Permission,
    pub config: ShellConfig,
    pub renderer: Renderer<Stdout, Stderr>,
    pub approver: Approver,
}

impl TurnContext {
    pub fn new(project: PathBuf, session_dir: PathBuf, config: ShellConfig) -> Self {
        let permission = config.permission;
        let approver = Approver::new(project.clone());
        Self {
            project,
            session_dir,
            session_id: None,
            permission,
            config,
            renderer: Renderer::stdio(),
            approver,
        }
    }

    /// Points the context at `id`, rejecting sessions of other projects.
    pub fn resolve_session(&mut self, id: &str) -> Result<(), String> {
        let session =
            session::load(&self.session_dir, id).map_err(|_| format!("找不到会话 {id}"))?;
        // A missing file loads as an empty session (no `open` line ever read).
        if session.project.as_os_str().is_empty() {
            return Err(format!("找不到会话 {id}"));
        }
        if !same_project(&session.project, &self.project) {
            return Err(format!(
                "会话 {id} 属于其他项目：{}",
                session.project.display()
            ));
        }
        self.session_id = Some(id.to_owned());
        Ok(())
    }

    /// Resumes the newest non-archived session for this project, if any.
    /// `session::list` is already newest-first.
    pub fn continue_last(&mut self) -> Result<Option<String>, String> {
        let sessions = session::list(&self.session_dir).map_err(|error| error.to_string())?;
        let found = sessions
            .into_iter()
            .find(|item| !item.archived && same_project(&item.project, &self.project))
            .map(|item| item.id);
        self.session_id = found.clone();
        Ok(found)
    }

    /// Lazily opens the session on the first message, so the GUI's session
    /// list never shows an empty conversation.
    fn ensure_session(&mut self, message: &str) -> Result<String, String> {
        if let Some(id) = &self.session_id {
            return Ok(id.clone());
        }
        let title = session_title(message);
        let id = session::open(&self.session_dir, &self.project, &title, session::now())
            .map_err(|error| format!("创建会话失败：{error}"))?;
        self.session_id = Some(id.clone());
        Ok(id)
    }

    /// Runs one full turn: ask envelope → agent → outcome envelope.
    pub fn run_turn(&mut self, message: &str) -> Result<TurnStatus, String> {
        // Drop a stale interrupt left over from the prompt or a previous turn.
        interrupt::set(false);
        self.renderer.reset_turn();

        let session_id = self.ensure_session(message)?;
        session::record_ask_with_context(
            &self.session_dir,
            &session_id,
            session::now(),
            message,
            &[],
        )
        .map_err(|error| format!("写入会话失败：{error}"))?;

        let request = RunRequest {
            project: self.project.clone(),
            message: message.to_owned(),
            pinned_context: Vec::new(),
            provider: self.config.provider.clone(),
            permission: self.permission,
            fallback_to_local: self.config.fallback_to_local,
            max_output_tokens: self.config.max_output_tokens,
            extended_thinking: self.config.extended_thinking,
            session_id: Some(session_id.clone()),
        };

        let mut logger = TurnLogger::new(self.session_dir.clone(), session_id);
        let result = {
            let logger_ref = &mut logger;
            let renderer = &mut self.renderer;
            let approver = &self.approver;
            let mut sink = move |event: SinkEvent| -> bool {
                if interrupt::pending() {
                    return false;
                }
                // The log is the source of truth — record before rendering.
                let logged = logger_ref.record(&event);
                if matches!(logged, Logged::WriteFailed) {
                    return false;
                }
                renderer.handle(&event);
                !interrupt::pending()
            };
            let alive = || !interrupt::pending();
            // Annotate params so the closure is higher-ranked over the
            // borrowed `&str` (Approve = dyn Fn(StepKind, &str) -> bool).
            let approve =
                |kind: kodo_agent::StepKind, command: &str| approver.approve(kind, command);
            kodo_agent::run(&request, &alive, &approve, &mut sink)
        };

        // `[a]` upgrades permission for subsequent turns (baked per RunRequest).
        if self.approver.take_escalate() {
            self.permission = Permission::Full;
        }

        let interrupted = interrupt::take();
        let (outcome, status) = match (&result, interrupted) {
            (Err(error), _) => (TurnOutcome::Error(error.clone()), TurnStatus::Failed),
            (Ok(()), true) => (TurnOutcome::Stopped, TurnStatus::Stopped),
            (Ok(()), false) => (TurnOutcome::Complete, TurnStatus::Completed),
        };
        if let Err(error) = logger.finish(outcome) {
            return Err(format!("写入会话失败：{error}"));
        }

        match status {
            TurnStatus::Stopped => self.renderer.note("⏹ 已取消"),
            TurnStatus::Failed => {
                let safe = tools::redact_secrets(&result.unwrap_err());
                self.renderer.warn(&format!("✗ {safe}"));
            }
            TurnStatus::Completed => {}
        }
        self.renderer.close_line();
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_ctx(project: PathBuf, session_dir: PathBuf) -> TurnContext {
        let config = ShellConfig {
            provider: None,
            permission: Permission::Ask,
            fallback_to_local: true,
            max_output_tokens: 4096,
            extended_thinking: false,
        };
        TurnContext::new(project, session_dir, config)
    }

    fn temp_root(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("kodo-cli-turn-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sessions")).expect("sessions dir");
        root
    }

    #[test]
    fn session_title_truncates_and_defaults() {
        let long = "问".repeat(50);
        let title = session_title(&long);
        assert_eq!(title.chars().count(), 41, "40 chars + ellipsis");
        assert!(title.ends_with('…'));

        let exact: String = "a".repeat(40);
        assert_eq!(session_title(&exact), exact);

        assert_eq!(session_title("  hello   world  "), "hello world");
        assert_eq!(session_title("   "), "新对话");
    }

    #[test]
    fn same_project_canonicalizes_both_sides() {
        let cwd = fs::canonicalize(std::env::current_dir().expect("cwd")).expect("canonical");
        assert!(same_project(&cwd, Path::new(".")));
        assert!(!same_project(&cwd, Path::new("/kodo-cli-no-such-project")));
    }

    #[test]
    fn continue_last_picks_newest_same_project_non_archived() {
        let root = temp_root("continue");
        let sessions = root.join("sessions");
        let proj_a = root.join("proj-a");
        let proj_b = root.join("proj-b");
        fs::create_dir_all(&proj_a).expect("proj-a");
        fs::create_dir_all(&proj_b).expect("proj-b");

        let id_old = session::open(&sessions, &proj_a, "old", 100).expect("old");
        let id_new = session::open(&sessions, &proj_a, "new", 200).expect("new");
        let id_other = session::open(&sessions, &proj_b, "other", 300).expect("other");
        let id_arch = session::open(&sessions, &proj_a, "arch", 400).expect("arch");
        session::archive(&sessions, &id_arch).expect("archive");

        let mut ctx = temp_ctx(proj_a.clone(), sessions.clone());
        let picked = ctx.continue_last().expect("list");
        assert_eq!(picked.as_deref(), Some(id_new.as_str()));

        // Foreign project is rejected, even by explicit id.
        let mut ctx = temp_ctx(proj_a, sessions.clone());
        let error = ctx.resolve_session(&id_other).expect_err("foreign");
        assert!(error.contains("属于其他项目"), "{error}");
        let error = ctx.resolve_session("no-such-session").expect_err("missing");
        assert!(error.contains("找不到会话"), "{error}");

        // Explicit id of an old (non-archived) session resolves fine.
        ctx.resolve_session(&id_old).expect("resolve old");
        assert_eq!(ctx.session_id.as_deref(), Some(id_old.as_str()));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn continue_last_on_an_empty_store_is_none() {
        let root = temp_root("empty");
        let mut ctx = temp_ctx(root.join("proj"), root.join("sessions"));
        assert_eq!(ctx.continue_last().expect("list"), None);
        assert!(ctx.session_id.is_none());
        let _ = fs::remove_dir_all(&root);
    }
}
