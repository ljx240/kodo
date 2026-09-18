//! The agent loop behind a conversation turn.
//!
//! The shell owns threads and session logs; this crate decides what work to do.
//! Events are emitted **before** the work runs and again when it finishes, so
//! the GUI can show a live step rather than a post-mortem.

mod provider;
mod tools;

use std::path::{Path, PathBuf};
use std::time::Instant;

use provider::ChatMessage;
use tools::{
    command_output_interruptible, detect_verify_command, extract_writes, is_dangerous_command,
    read_text, search_files, summarize_git_changes, write_project_file,
};

pub use provider::Provider;
pub use tools::dangerous_reason;

/// Permission mode for tool execution. Default is Ask — Secure by Default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Every shell command and file write requires explicit approval.
    Ask,
    /// Safe commands auto-run; dangerous ones and writes require approval.
    Auto,
    /// All project-local work runs without approval.
    Full,
}

impl Permission {
    pub fn parse(value: &str) -> Self {
        match value {
            "ask" => Self::Ask,
            "auto" => Self::Auto,
            "full" => Self::Full,
            _ => Self::Ask,
        }
    }

    pub fn needs_approval(self, kind: StepKind, command: Option<&str>) -> bool {
        match kind {
            StepKind::Command => match self {
                Self::Full => false,
                Self::Auto => command.map(is_dangerous_command).unwrap_or(true),
                Self::Ask => true,
            },
            // Writes always need a human in ask/auto; Full allows project-local writes.
            StepKind::FileChange => !matches!(self, Self::Full),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Reasoning,
    Search,
    FileRead,
    Command,
    ModelCall,
    FileChange,
    AgentMessage,
}

/// One finished unit of work, ready to be written to the session log.
#[derive(Debug, Clone)]
pub enum Step {
    Reasoning { summary: String },
    Search { query: String, detail: String },
    FileRead { path: String, detail: String },
    Command { command: String, cwd: String, output: String, exit_code: Option<i32> },
    ModelCall { model: String, input_tokens: u32, output_tokens: u32 },
    FileChange { changes: Vec<FileDelta> },
    AgentMessage { text: String, checks: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDelta {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

impl Step {
    pub fn kind(&self) -> StepKind {
        match self {
            Step::Reasoning { .. } => StepKind::Reasoning,
            Step::Search { .. } => StepKind::Search,
            Step::FileRead { .. } => StepKind::FileRead,
            Step::Command { .. } => StepKind::Command,
            Step::ModelCall { .. } => StepKind::ModelCall,
            Step::FileChange { .. } => StepKind::FileChange,
            Step::AgentMessage { .. } => StepKind::AgentMessage,
        }
    }

    pub fn running(&self) -> Step {
        match self {
            Step::Command { command, cwd, .. } => Step::Command {
                command: command.clone(),
                cwd: cwd.clone(),
                output: String::new(),
                exit_code: None,
            },
            Step::FileChange { changes } => Step::FileChange { changes: changes.clone() },
            other => other.clone(),
        }
    }

    pub fn preview_detail(&self) -> String {
        match self {
            Step::Command { command, .. } => command.clone(),
            Step::Search { query, .. } => query.clone(),
            Step::FileRead { path, .. } => path.clone(),
            Step::ModelCall { model, .. } => model.clone(),
            Step::FileChange { changes } => changes
                .iter()
                .map(|c| c.path.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            _ => String::new(),
        }
    }
}

/// What the shell receives while a turn runs.
pub enum SinkEvent {
    Started { step: Step },
    Finished { step: Step, duration_ms: u64, denied: bool },
}

pub type Emit<'a> = dyn FnMut(SinkEvent) -> bool + 'a;
pub type Alive<'a> = dyn Fn() -> bool + 'a;
pub type Approve<'a> = dyn Fn(StepKind, &str) -> bool + 'a;

pub struct RunRequest {
    pub project: PathBuf,
    pub message: String,
    pub provider: Option<Provider>,
    pub permission: Permission,
    pub fallback_to_local: bool,
    pub max_output_tokens: u32,
    pub extended_thinking: bool,
}

fn run_step(step: Step, alive: &Alive, emit: &mut Emit) -> bool {
    if !alive() {
        return false;
    }
    emit(SinkEvent::Started { step })
}

fn finish_step(step: Step, duration_ms: u64, denied: bool, emit: &mut Emit) -> bool {
    emit(SinkEvent::Finished { step, duration_ms, denied })
}

/// Executes a command step: approval → start → interruptible run → finish.
fn command_step(
    command: &str,
    project: &Path,
    request: &RunRequest,
    alive: &Alive,
    approve: &Approve,
    emit: &mut Emit,
    notes: &mut Vec<String>,
) -> Result<bool, String> {
    if !alive() {
        return Ok(false);
    }
    let cwd = project.to_string_lossy().into_owned();
    let provisional = Step::Command {
        command: command.to_owned(),
        cwd: cwd.clone(),
        output: String::new(),
        exit_code: None,
    };

    if request.permission.needs_approval(StepKind::Command, Some(command)) {
        if !approve(StepKind::Command, command) {
            notes.push(format!("用户拒绝了 `{command}`"));
            return Ok(finish_step(provisional, 0, true, emit));
        }
    }

    if !run_step(provisional.clone(), alive, emit) {
        return Ok(false);
    }
    let began = Instant::now();
    let (output, code, killed) = command_output_interruptible(project, command, alive);
    let duration_ms = began.elapsed().as_millis() as u64;
    if killed && !alive() {
        // User stopped mid-command: still finish the log line as interrupted.
        let finished = Step::Command {
            command: command.to_owned(),
            cwd,
            output,
            exit_code: code,
        };
        return Ok(finish_step(finished, duration_ms, false, emit));
    }
    let finished = Step::Command {
        command: command.to_owned(),
        cwd,
        output,
        exit_code: code,
    };
    let keep = finish_step(finished, duration_ms, false, emit);
    notes.push(format!(
        "`{command}` → {}",
        if killed {
            "已中断"
        } else if code == Some(0) {
            "ok"
        } else {
            "非零退出"
        }
    ));
    Ok(keep)
}

/// Approval + write for one file operation.
fn write_step(
    path: &str,
    content: &str,
    project: &Path,
    request: &RunRequest,
    approve: &Approve,
    emit: &mut Emit,
    notes: &mut Vec<String>,
) -> Result<bool, String> {
    let label = format!("write {path}");
    if request.permission.needs_approval(StepKind::FileChange, Some(&label)) {
        if !approve(StepKind::FileChange, &label) {
            notes.push(format!("用户拒绝写入 `{path}`"));
            let denied = Step::FileChange {
                changes: vec![FileDelta { path: path.to_owned(), added: 0, removed: 0 }],
            };
            return Ok(finish_step(denied, 0, true, emit));
        }
    }

    let provisional = Step::FileChange {
        changes: vec![FileDelta { path: path.to_owned(), added: 0, removed: 0 }],
    };
    if !run_step(provisional.clone(), &|| true, emit) {
        return Ok(false);
    }
    let began = Instant::now();
    match write_project_file(project, path, content) {
        Ok((rel, added, removed)) => {
            let duration_ms = began.elapsed().as_millis() as u64;
            notes.push(format!("写入 `{rel}` (+{added} -{removed})"));
            Ok(finish_step(
                Step::FileChange { changes: vec![FileDelta { path: rel, added, removed }] },
                duration_ms,
                false,
                emit,
            ))
        }
        Err(error) => {
            notes.push(format!("写入 `{path}` 失败：{error}"));
            Ok(finish_step(provisional, 0, true, emit))
        }
    }
}

fn simple_step(step: Step, alive: &Alive, emit: &mut Emit) -> bool {
    if !run_step(step.clone(), alive, emit) {
        return false;
    }
    let began = Instant::now();
    finish_step(step, began.elapsed().as_millis() as u64, false, emit)
}

fn call_model(
    provider: &Provider,
    history: &[ChatMessage],
    request: &RunRequest,
    alive: &Alive,
    emit: &mut Emit,
) -> Result<Option<(String, u32, u32)>, String> {
    if !alive() {
        return Ok(None);
    }
    let call = Step::ModelCall {
        model: provider.model.clone(),
        input_tokens: 0,
        output_tokens: 0,
    };
    if !run_step(call, alive, emit) {
        return Ok(None);
    }
    let began = Instant::now();
    let result = provider::chat(provider, history, request.max_output_tokens);
    let duration_ms = began.elapsed().as_millis() as u64;
    if !alive() {
        return Ok(None);
    }
    match result {
        Ok(response) => {
            if !finish_step(
                Step::ModelCall {
                    model: provider.model.clone(),
                    input_tokens: response.input_tokens,
                    output_tokens: response.output_tokens,
                },
                duration_ms,
                false,
                emit,
            ) {
                return Ok(None);
            }
            Ok(Some((response.text, response.input_tokens, response.output_tokens)))
        }
        Err(error) => {
            finish_step(
                Step::ModelCall { model: provider.model.clone(), input_tokens: 0, output_tokens: 0 },
                duration_ms,
                true,
                emit,
            );
            if !request.fallback_to_local {
                return Err(format!("model call failed: {error}"));
            }
            Err(error)
        }
    }
}

pub fn run(request: &RunRequest, alive: &Alive, approve: &Approve, emit: &mut Emit) -> Result<(), String> {
    let project = request.project.as_path();
    let mut notes: Vec<String> = Vec::new();

    if !simple_step(
        Step::Reasoning { summary: "梳理任务目标，检查项目上下文与可用配置".to_owned() },
        alive,
        emit,
    ) {
        return Ok(());
    }

    let keywords = keywords_from(&request.message);
    let query = if keywords.is_empty() { "src".to_owned() } else { keywords.join(" ") };

    if alive() {
        let search = Step::Search { query: query.clone(), detail: String::new() };
        if !run_step(search, alive, emit) {
            return Ok(());
        }
        let began = Instant::now();
        let detail = search_files(project, &query);
        let duration_ms = began.elapsed().as_millis() as u64;
        if !alive() {
            return Ok(());
        }
        if !finish_step(Step::Search { query: query.clone(), detail: detail.clone() }, duration_ms, false, emit) {
            return Ok(());
        }
        notes.push(format!("搜索 `{query}`：{detail}"));
    }

    for path in ["README.md", "Cargo.toml", "package.json", "docs/design/DESIGN.md"] {
        if !alive() {
            return Ok(());
        }
        let full = project.join(path);
        if !full.is_file() {
            continue;
        }
        let relative = path.to_owned();
        if !run_step(Step::FileRead { path: relative.clone(), detail: String::new() }, alive, emit) {
            return Ok(());
        }
        let began = Instant::now();
        let detail = read_text(&full, 24);
        let duration_ms = began.elapsed().as_millis() as u64;
        if !alive() {
            return Ok(());
        }
        if !finish_step(
            Step::FileRead { path: relative.clone(), detail: detail.clone() },
            duration_ms,
            false,
            emit,
        ) {
            return Ok(());
        }
        notes.push(format!("读取 `{relative}`"));
    }

    if !command_step("git status --short", project, request, alive, approve, emit, &mut notes)? {
        return Ok(());
    }

    let provider_ready = request
        .provider
        .as_ref()
        .map(|p| !p.api_key.trim().is_empty() && !p.model.trim().is_empty())
        .unwrap_or(false);

    let mut answer = String::new();
    let mut checks: Vec<String> = Vec::new();
    let mut wrote_files = false;
    let mut verified = false;

    if provider_ready && alive() {
        let provider = request.provider.clone().expect("checked above");
        let mut system = system_prompt(project);
        if request.extended_thinking {
            system.push_str("\nThink step by step before answering.");
        }
        let mut history = vec![
            ChatMessage { role: "system".to_owned(), content: system },
            ChatMessage {
                role: "user".to_owned(),
                content: format!(
                    "{}\n\n## Local notes\n{}",
                    request.message,
                    if notes.is_empty() { "(none)".to_owned() } else { notes.join("\n") }
                ),
            },
        ];

        // Multi-round loop: model → tools (write/bash) → model → …
        for round in 0..6 {
            if !alive() {
                return Ok(());
            }
            if round > 0 {
                history.push(ChatMessage {
                    role: "user".to_owned(),
                    content: format!(
                        "Continue. Notes:\n{}\nWhen finished, answer without tool fences.",
                        notes.join("\n")
                    ),
                });
            }

            match call_model(&provider, &history, request, alive, emit) {
                Ok(Some((text, _, _))) => {
                    history.push(ChatMessage { role: "assistant".to_owned(), content: text.clone() });
                    let writes = extract_writes(&text);
                    let command = extract_command(&text);
                    let has_tools = !writes.is_empty() || command.is_some();

                    for write in writes {
                        if !alive() {
                            return Ok(());
                        }
                        if !write_step(&write.path, &write.content, project, request, approve, emit, &mut notes)? {
                            return Ok(());
                        }
                        wrote_files = true;
                    }

                    if let Some(command) = command {
                        if !command_step(&command, project, request, alive, approve, emit, &mut notes)? {
                            return Ok(());
                        }
                    }

                    if !has_tools {
                        answer = strip_tool_fences(&text);
                        checks = checks_from(&text);
                        break;
                    }

                    // After writes, run project verification once.
                    if wrote_files && !verified {
                        if let Some(verify) = detect_verify_command(project) {
                            if command_step(&verify, project, request, alive, approve, emit, &mut notes)? {
                                verified = true;
                            }
                        }
                    }

                    if round == 5 {
                        answer = strip_tool_fences(&text);
                        checks = checks_from(&text);
                    }
                }
                Ok(None) => return Ok(()),
                Err(error) => {
                    if !request.fallback_to_local {
                        return Err(format!("model call failed: {error}"));
                    }
                    answer = offline_answer(&request.message, &notes, Some(&error));
                    checks = vec!["已使用本地上下文（模型调用失败）".to_owned()];
                    break;
                }
            }
        }

        if answer.is_empty() {
            answer = offline_answer(&request.message, &notes, None);
            if checks.is_empty() {
                checks = vec!["模型未给出最终答复，以下为本地笔记".to_owned()];
            }
        }
    } else if !request.fallback_to_local && request.provider.is_some() {
        return Err("provider is missing an API key or model".to_owned());
    } else {
        let reason = if request.provider.is_none() {
            "尚未配置 AI Provider"
        } else {
            "当前 Provider 缺少 API Key 或模型"
        };
        answer = offline_answer(&request.message, &notes, None);
        checks = vec![format!("{reason}，本轮基于本地扫描")];
    }

    if alive() {
        let changes = summarize_git_changes(project);
        if !changes.is_empty() {
            let step = Step::FileChange {
                changes: changes
                    .into_iter()
                    .map(|(path, added, removed)| FileDelta { path, added, removed })
                    .collect(),
            };
            if !simple_step(step, alive, emit) {
                return Ok(());
            }
        }
    }

    if alive() {
        if verified {
            checks.push("已执行项目验证命令".to_owned());
        }
        if wrote_files {
            checks.push("已写入项目内文件".to_owned());
        }
        simple_step(Step::AgentMessage { text: answer, checks }, alive, emit);
    }
    Ok(())
}

fn system_prompt(project: &Path) -> String {
    format!(
        "You are Kodo, a concise coding agent working in {project}.\n\
         Prefer facts you can justify from the local notes.\n\
         To change a file, emit one or more fences:\n\
         ```write\n\
         path: relative/path.ext\n\
         ---\n\
         full file content\n\
         ```\n\
         Paths must stay inside the project (no .. or absolute paths).\n\
         To inspect or verify, emit a ```bash fence with a single command.\n\
         After writing files you may rely on Kodo to run project tests once.\n\
         End with 1-3 short checks as a markdown list starting with '- '.\n\
         When no more tools are needed, answer in prose without fences.",
        project = project.display()
    )
}

fn keywords_from(message: &str) -> Vec<String> {
    message
        .split(|c: char| !(c.is_alphanumeric() || c == '_') && !c.is_ascii_punctuation())
        .filter(|token| {
            let count = token.chars().count();
            count >= 2 && count <= 40
        })
        .filter(|token| {
            let lower = token.to_ascii_lowercase();
            !matches!(
                lower.as_str(),
                "the" | "and" | "for" | "with" | "that" | "this" | "一下" | "检查" | "进行" | "是否"
            )
        })
        .take(4)
        .map(|token| token.to_owned())
        .collect()
}

fn offline_answer(message: &str, notes: &[String], error: Option<&str>) -> String {
    let mut body = String::new();
    if error.is_some() {
        body.push_str("模型调用未成功，以下是基于项目本地扫描的说明。\n\n");
    } else {
        body.push_str("本轮未配置可用的模型服务，以下是基于项目本地扫描的说明。\n\n");
    }
    body.push_str(&format!("**任务**：{message}\n\n"));
    if !notes.is_empty() {
        body.push_str("**已执行**\n");
        for note in notes {
            body.push_str(&format!("- {note}\n"));
        }
        body.push('\n');
    }
    if let Some(error) = error {
        body.push_str(&format!("**模型错误**：{error}\n\n"));
    }
    body.push_str("配置 Settings → AI Provider 的 API Key 后，可获得结合代码语义的完整答复。");
    body
}

fn checks_from(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .take(5)
        .collect()
}

fn extract_command(text: &str) -> Option<String> {
    let start = text.find("```bash")?;
    let rest = &text[start + 7..];
    let end = rest.find("```")?;
    let block = rest[..end].trim();
    if block.is_empty() {
        None
    } else {
        Some(block.to_owned())
    }
}

fn strip_tool_fences(text: &str) -> String {
    let mut out = String::new();
    let mut skip = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if trimmed.starts_with("```bash") || trimmed.starts_with("```write") {
                skip = true;
                continue;
            }
            if skip {
                skip = false;
                continue;
            }
        }
        if !skip {
            out.push_str(line);
            out.push('\n');
        }
    }
    let trimmed = out.trim().to_owned();
    if trimmed.is_empty() { text.trim().to_owned() } else { trimmed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_permission_is_ask() {
        assert_eq!(Permission::parse(""), Permission::Ask);
        assert_eq!(Permission::parse("full"), Permission::Full);
    }

    #[test]
    fn ask_blocks_writes_and_commands() {
        assert!(Permission::Ask.needs_approval(StepKind::Command, Some("cargo check")));
        assert!(Permission::Ask.needs_approval(StepKind::FileChange, Some("write a.rs")));
        assert!(Permission::Auto.needs_approval(StepKind::FileChange, Some("write a.rs")));
        assert!(!Permission::Full.needs_approval(StepKind::FileChange, Some("write a.rs")));
        assert!(!Permission::Auto.needs_approval(StepKind::Command, Some("cargo check")));
        assert!(Permission::Auto.needs_approval(StepKind::Command, Some("rm -rf x")));
    }

    #[test]
    fn extract_command_keeps_multiline_blocks() {
        let text = "```bash\necho one\necho two\n```\n- ok";
        assert_eq!(extract_command(text).as_deref(), Some("echo one\necho two"));
    }

    #[test]
    fn strip_tool_fences_hides_write_and_bash() {
        let text = "```write\npath: a\n---\nx\n```\nhello\n```bash\nls\n```\nbye";
        let stripped = strip_tool_fences(text);
        assert!(stripped.contains("hello"));
        assert!(stripped.contains("bye"));
        assert!(!stripped.contains("ls"));
    }

    #[test]
    fn keywords_skip_stop_words() {
        let keys = keywords_from("请检查 k2k-rust 的 unknown table");
        assert!(keys.iter().any(|k| k.contains("k2k") || k.contains("unknown")));
    }
}
