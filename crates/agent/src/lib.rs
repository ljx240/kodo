//! The agent loop behind a conversation turn.
//!
//! The shell owns threads and session logs; this crate decides what work to do.
//! Events are emitted **before** the work runs and again when it finishes, so
//! the GUI can show a live step rather than a post-mortem.
//!
//! Model↔tool traffic goes through the typed [`protocol`] module. The agent
//! loop does not parse Markdown fences itself; it only executes
//! [`protocol::ToolInvocation`] values.

mod context;
mod patch;
mod plan;
mod protocol;
mod provider;
mod state;
mod tools;
mod verify;

use std::path::{Path, PathBuf};
use std::time::Instant;

use context::{ContextBudget, ContextManager};
use patch::{apply_patch, create_file, delete_file, replace_range, ApplyPatchArgs, ReplaceRangeArgs};
use plan::TaskPlan;
use provider::{ChatMessage, NativeToolCall};
use protocol::{
    format_observations, parse_model_turn, protocol_instructions, ModelTurn, ToolArgs, ToolCall,
    ToolCallId, ToolError, ToolErrorCode, ToolInvocation, ToolName, ToolRegistry, ToolResult,
};
use state::{AgentEvent, AgentMachine, AgentState, Budget, FailReason};
use tools::{
    command_output_interruptible, is_dangerous_command, read_text, search_files,
    summarize_git_changes, write_project_file,
};
use verify::{FinalStatus, VerificationRunner};

pub use context::{ContextBudget as TurnContextBudget, ContextSpan, DEFAULT_CONTEXT_CHARS};
pub use patch::PatchOutcome;
pub use plan::{Subtask, SubtaskKind};
pub use protocol::{ToolDefinition, ToolError as AgentToolError, ToolRegistry as AgentToolRegistry};
pub use provider::{Provider, ProviderCapabilities};
pub use state::{AgentState as TurnState, Budget as TurnBudget, FailReason as TurnFailReason};
pub use tools::dangerous_reason;
pub use verify::{FinalStatus as TurnFinalStatus, VerificationRunner as TurnVerifier};

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
    /// Project-relative paths pinned as context for this turn (not file bodies).
    pub pinned_context: Vec<String>,
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

/// Keep=false stops the turn; result carries the observation for the model.
type StepOutcome = Result<(bool, Option<ToolResult>), String>;

/// Executes a command step: approval → start → interruptible run → finish.
fn command_step(
    command: &str,
    call_id: ToolCallId,
    project: &Path,
    request: &RunRequest,
    alive: &Alive,
    approve: &Approve,
    emit: &mut Emit,
    notes: &mut Vec<String>,
) -> StepOutcome {
    if !alive() {
        return Ok((false, None));
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
            let keep = finish_step(provisional, 0, true, emit);
            let result = ToolResult::failure(
                call_id,
                ToolName::RunCommand.label(),
                command,
                ToolError::permission_denied("用户拒绝了该命令"),
            );
            return Ok((keep, Some(result)));
        }
    }

    if !run_step(provisional.clone(), alive, emit) {
        return Ok((false, None));
    }
    let began = Instant::now();
    let (output, code, killed) = command_output_interruptible(project, command, alive);
    let duration_ms = began.elapsed().as_millis() as u64;
    let ok = code == Some(0) && !killed;
    let summary = if killed {
        "已中断"
    } else if code == Some(0) {
        "ok"
    } else {
        "非零退出"
    };
    let result = if ok {
        ToolResult::success(call_id, ToolName::RunCommand.label(), command, output.clone())
    } else {
        let code = if killed {
            ToolErrorCode::Interrupted
        } else {
            ToolErrorCode::ExecutionFailed
        };
        ToolResult::failure(
            call_id,
            ToolName::RunCommand.label(),
            command,
            ToolError::new(code, output.clone()),
        )
    };

    if killed && !alive() {
        let finished = Step::Command {
            command: command.to_owned(),
            cwd,
            output,
            exit_code: code,
        };
        let keep = finish_step(finished, duration_ms, false, emit);
        notes.push(format!("`{command}` → 已中断"));
        return Ok((keep, Some(result)));
    }
    let finished = Step::Command {
        command: command.to_owned(),
        cwd,
        output,
        exit_code: code,
    };
    let keep = finish_step(finished, duration_ms, false, emit);
    notes.push(format!("`{command}` → {summary}"));
    Ok((keep, Some(result)))
}

/// Approval + write for one file operation.
fn write_step(
    path: &str,
    content: &str,
    call_id: ToolCallId,
    project: &Path,
    request: &RunRequest,
    approve: &Approve,
    emit: &mut Emit,
    notes: &mut Vec<String>,
) -> StepOutcome {
    let label = format!("write {path}");
    if request.permission.needs_approval(StepKind::FileChange, Some(&label)) {
        if !approve(StepKind::FileChange, &label) {
            notes.push(format!("用户拒绝写入 `{path}`"));
            let denied = Step::FileChange {
                changes: vec![FileDelta { path: path.to_owned(), added: 0, removed: 0 }],
            };
            let keep = finish_step(denied, 0, true, emit);
            let result = ToolResult::failure(
                call_id,
                ToolName::WriteFile.label(),
                path,
                ToolError::permission_denied("用户拒绝了写入"),
            );
            return Ok((keep, Some(result)));
        }
    }

    let provisional = Step::FileChange {
        changes: vec![FileDelta { path: path.to_owned(), added: 0, removed: 0 }],
    };
    if !run_step(provisional.clone(), &|| true, emit) {
        return Ok((false, None));
    }
    let began = Instant::now();
    match write_project_file(project, path, content) {
        Ok((rel, added, removed)) => {
            let duration_ms = began.elapsed().as_millis() as u64;
            notes.push(format!("写入 `{rel}` (+{added} -{removed})"));
            let result = ToolResult::success(
                call_id,
                ToolName::WriteFile.label(),
                rel.clone(),
                format!("wrote {rel} (+{added} -{removed})"),
            );
            let keep = finish_step(
                Step::FileChange { changes: vec![FileDelta { path: rel, added, removed }] },
                duration_ms,
                false,
                emit,
            );
            Ok((keep, Some(result)))
        }
        Err(error) => {
            notes.push(format!("写入 `{path}` 失败：{error}"));
            let path_error = if error.contains("escape") || error.contains("absolute") {
                ToolError::path_escape(error.clone())
            } else {
                ToolError::execution(error.clone())
            };
            let result = ToolResult::failure(
                call_id,
                ToolName::WriteFile.label(),
                path,
                path_error,
            );
            Ok((finish_step(provisional, 0, true, emit), Some(result)))
        }
    }
}

/// Runs one typed tool call and returns (keep, observation).
fn execute_tool_call(
    call: &ToolCall,
    project: &Path,
    request: &RunRequest,
    alive: &Alive,
    approve: &Approve,
    emit: &mut Emit,
    notes: &mut Vec<String>,
) -> StepOutcome {
    match (&call.name, &call.args) {
        (ToolName::RunCommand, ToolArgs::RunCommand { command }) => {
            command_step(command, call.id.clone(), project, request, alive, approve, emit, notes)
        }
        (ToolName::WriteFile, ToolArgs::WriteFile { path, content }) => {
            write_step(path, content, call.id.clone(), project, request, approve, emit, notes)
        }
        (ToolName::Search, ToolArgs::Search { query }) => {
            if !alive() {
                return Ok((false, None));
            }
            let step = Step::Search { query: query.clone(), detail: String::new() };
            if !run_step(step, alive, emit) {
                return Ok((false, None));
            }
            let began = Instant::now();
            let detail = search_files(project, query);
            let duration_ms = began.elapsed().as_millis() as u64;
            if !alive() {
                return Ok((false, None));
            }
            let keep = finish_step(
                Step::Search { query: query.clone(), detail: detail.clone() },
                duration_ms,
                false,
                emit,
            );
            notes.push(format!("搜索 `{query}`：{detail}"));
            Ok((keep, Some(ToolResult::success(call.id.clone(), ToolName::Search.label(), query.clone(), detail))))
        }
        (ToolName::ReadFile, ToolArgs::ReadFile { path }) => {
            if !alive() {
                return Ok((false, None));
            }
            let resolve = tools::resolve_in_project(project, path);
            let (detail, result) = match &resolve {
                Err(error) => {
                    let err = if error.contains("escape") || error.contains("absolute") {
                        ToolError::path_escape(error.clone())
                    } else {
                        ToolError::execution(error.clone())
                    };
                    (
                        format!("路径错误：{error}"),
                        ToolResult::failure(
                            call.id.clone(),
                            ToolName::ReadFile.label(),
                            path.clone(),
                            err,
                        ),
                    )
                }
                Ok(full) if !full.is_file() => {
                    let msg = format!("文件不存在：{path}");
                    (
                        msg.clone(),
                        ToolResult::failure(
                            call.id.clone(),
                            ToolName::ReadFile.label(),
                            path.clone(),
                            ToolError::execution(msg),
                        ),
                    )
                }
                Ok(full) => {
                    let text = read_text(full, 120);
                    (
                        text.clone(),
                        ToolResult::success(
                            call.id.clone(),
                            ToolName::ReadFile.label(),
                            path.clone(),
                            text,
                        ),
                    )
                }
            };
            let ok = result.ok;
            if !run_step(
                Step::FileRead { path: path.clone(), detail: detail.clone() },
                alive,
                emit,
            ) {
                return Ok((false, None));
            }
            let began = Instant::now();
            let keep = finish_step(
                Step::FileRead { path: path.clone(), detail: detail.clone() },
                began.elapsed().as_millis() as u64,
                !ok,
                emit,
            );
            notes.push(format!("读取 `{path}`{}", if ok { "" } else { " 失败" }));
            Ok((keep, Some(result)))
        }
        (ToolName::ApplyPatch, ToolArgs::ApplyPatch { path, old, new, start_line }) => {
            patch_approval_step(
                call.id.clone(),
                ToolName::ApplyPatch,
                path,
                project,
                request,
                approve,
                emit,
                notes,
                |project| {
                    apply_patch(
                        project,
                        &ApplyPatchArgs {
                            path: path.clone(),
                            old: old.clone(),
                            new: new.clone(),
                            start_line: *start_line,
                        },
                    )
                },
                alive,
            )
        }
        (ToolName::ReplaceRange, ToolArgs::ReplaceRange { path, start_line, end_line, new_text }) => {
            patch_approval_step(
                call.id.clone(),
                ToolName::ReplaceRange,
                path,
                project,
                request,
                approve,
                emit,
                notes,
                |project| {
                    replace_range(
                        project,
                        &ReplaceRangeArgs {
                            path: path.clone(),
                            start_line: *start_line,
                            end_line: *end_line,
                            new_text: new_text.clone(),
                        },
                    )
                },
                alive,
            )
        }
        (ToolName::CreateFile, ToolArgs::CreateFile { path, content }) => {
            patch_approval_step(
                call.id.clone(),
                ToolName::CreateFile,
                path,
                project,
                request,
                approve,
                emit,
                notes,
                |project| create_file(project, path, content),
                alive,
            )
        }
        (ToolName::DeleteFile, ToolArgs::DeleteFile { path }) => {
            patch_approval_step(
                call.id.clone(),
                ToolName::DeleteFile,
                path,
                project,
                request,
                approve,
                emit,
                notes,
                |project| delete_file(project, path),
                alive,
            )
        }
        // Defensive: name/args mismatch should not crash the session.
        (name, args) => Ok((
            true,
            Some(ToolResult::failure(
                call.id.clone(),
                name.label(),
                args.label(),
                ToolError::invalid_args("tool name and arguments do not match"),
            )),
        )),
    }
}

/// Approval + emit FileChange for patch-family tools. Conflicts stay as failed results.
fn patch_approval_step(
    call_id: ToolCallId,
    name: ToolName,
    path: &str,
    project: &Path,
    request: &RunRequest,
    approve: &Approve,
    emit: &mut Emit,
    notes: &mut Vec<String>,
    run: impl FnOnce(&Path) -> Result<crate::patch::PatchOutcome, ToolError>,
    alive: &Alive,
) -> StepOutcome {
    if !alive() {
        return Ok((false, None));
    }
    let label = format!("{} {path}", name.label());
    if request.permission.needs_approval(StepKind::FileChange, Some(&label)) {
        if !approve(StepKind::FileChange, &label) {
            notes.push(format!("用户拒绝 `{label}`"));
            let denied = Step::FileChange {
                changes: vec![FileDelta { path: path.to_owned(), added: 0, removed: 0 }],
            };
            let keep = finish_step(denied, 0, true, emit);
            return Ok((
                keep,
                Some(ToolResult::failure(
                    call_id,
                    name.label(),
                    path.to_owned(),
                    ToolError::permission_denied("用户拒绝了写入"),
                )),
            ));
        }
    }

    let provisional = Step::FileChange {
        changes: vec![FileDelta { path: path.to_owned(), added: 0, removed: 0 }],
    };
    if !run_step(provisional.clone(), &|| true, emit) {
        return Ok((false, None));
    }
    let began = Instant::now();
    match run(project) {
        Ok(outcome) => {
            let duration_ms = began.elapsed().as_millis() as u64;
            notes.push(format!("{} (+{} -{})", outcome.summary, outcome.added, outcome.removed));
            let result = ToolResult::success(
                call_id,
                name.label(),
                outcome.path.clone(),
                format!(
                    "{}\nchanged: {} (+{} -{})",
                    outcome.summary, outcome.path, outcome.added, outcome.removed
                ),
            );
            let keep = finish_step(
                Step::FileChange {
                    changes: vec![FileDelta {
                        path: outcome.path,
                        added: outcome.added,
                        removed: outcome.removed,
                    }],
                },
                duration_ms,
                false,
                emit,
            );
            Ok((keep, Some(result)))
        }
        Err(error) => {
            notes.push(format!("`{label}` 冲突/失败：{}", error.message));
            let result = ToolResult::failure(call_id, name.label(), path.to_owned(), error);
            let keep = finish_step(provisional, 0, true, emit);
            Ok((keep, Some(result)))
        }
    }
}

/// Drive one model turn's invocations → results (rejections become results).
fn run_invocations(
    invocations: Vec<ToolInvocation>,
    project: &Path,
    request: &RunRequest,
    alive: &Alive,
    approve: &Approve,
    emit: &mut Emit,
    notes: &mut Vec<String>,
    wrote_files: &mut bool,
) -> Result<Option<Vec<ToolResult>>, String> {
    if invocations.is_empty() {
        return Ok(None);
    }
    let mut results = Vec::new();
    for invocation in invocations {
        if !alive() {
            return Ok(None);
        }
        match invocation {
            ToolInvocation::Rejected(reject) => {
                // Structured, recoverable — never abort the session.
                notes.push(format!(
                    "工具调用被拒绝 `{}`：{}",
                    reject.name, reject.error.message
                ));
                results.push(ToolResult::from_rejection(&reject));
            }
            ToolInvocation::Ready(call) => {
                let is_mutation = call.name.is_mutation();
                let (keep, result) =
                    execute_tool_call(&call, project, request, alive, approve, emit, notes)?;
                if let Some(result) = result {
                    if result.ok && is_mutation {
                        *wrote_files = true;
                    }
                    results.push(result);
                }
                if !keep {
                    return Ok(None);
                }
            }
        }
    }
    Ok(Some(results))
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
) -> Result<Option<(String, Vec<NativeToolCall>, u32, u32)>, String> {
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
            Ok(Some((
                response.text,
                response.native_tool_calls,
                response.input_tokens,
                response.output_tokens,
            )))
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

/// Convert native provider tool calls into typed invocations (boundary only).
fn invocations_from_native(
    native: Vec<NativeToolCall>,
    registry: &ToolRegistry,
) -> Vec<ToolInvocation> {
    native
        .into_iter()
        .enumerate()
        .map(|(index, call)| {
            let id = ToolCallId::new(if call.id.trim().is_empty() {
                format!("native_{index}")
            } else {
                call.id
            });
            registry.accept(id, &call.name, &call.arguments)
        })
        .collect()
}

pub fn run(request: &RunRequest, alive: &Alive, approve: &Approve, emit: &mut Emit) -> Result<(), String> {
    let project = request.project.as_path();
    let mut notes: Vec<String> = Vec::new();
    let mut pre_observations: Vec<ToolResult> = Vec::new();
    let registry = ToolRegistry::standard();
    let mut machine = AgentMachine::new(&request.message, Budget::default());
    machine.handle(AgentEvent::TaskReceived);

    if !simple_step(
        Step::Reasoning { summary: machine.progress_summary() },
        alive,
        emit,
    ) {
        machine.handle(AgentEvent::Cancel);
        return Ok(());
    }

    // Plan (heuristic structured plan; model JSON may refine later).
    machine.handle(AgentEvent::PlanReady);
    if !simple_step(
        Step::Reasoning {
            summary: format!("Plan · {}", machine.plan().progress_summary("locked")),
        },
        alive,
        emit,
    ) {
        machine.handle(AgentEvent::Cancel);
        return Ok(());
    }

    // Dynamic lexical context: file map → path/grep ranking → budgeted spans.
    // Replaces fixed README/Cargo/package/DESIGN head-of-file reads.
    let mut context_mgr = ContextManager::new(project, ContextBudget::default());

    // User-pinned context paths: validate inside the project and pin spans first.
    for rel in &request.pinned_context {
        if !alive() {
            machine.handle(AgentEvent::Cancel);
            return Ok(());
        }
        match context_mgr.read_range(rel, 1, 400, "pinned by user") {
            Ok(span) => {
                notes.push(format!("钉住上下文 `{}`", span.path));
                context_mgr.pin(span);
            }
            Err(error) => {
                // Context read failure is recoverable — surface and continue.
                notes.push(format!("上下文读取失败 `{rel}`：{error}"));
                pre_observations.push(ToolResult::failure(
                    ToolCallId::new(format!("pin_err_{rel}")),
                    ToolName::ReadFile.label(),
                    rel.clone(),
                    ToolError::execution(error),
                ));
            }
        }
    }

    let context_query = {
        let keys = keywords_from(&request.message);
        let goal = machine.plan().goal.clone();
        let goal_keys = keywords_from(&goal);
        let mut all = keys;
        all.extend(goal_keys);
        all.dedup();
        if all.is_empty() { "src".to_owned() } else { all.join(" ") }
    };

    if alive() {
        let search = Step::Search { query: context_query.clone(), detail: String::new() };
        if !run_step(search, alive, emit) {
            machine.handle(AgentEvent::Cancel);
            return Ok(());
        }
        let began = Instant::now();
        let scan_result = context_mgr.scan(alive);
        let spans = match scan_result {
            Ok(()) => match context_mgr.collect(&context_query, alive) {
                Ok(spans) => Some(spans),
                Err(_) => {
                    machine.handle(AgentEvent::Cancel);
                    return Ok(());
                }
            },
            Err(_) => {
                machine.handle(AgentEvent::Cancel);
                return Ok(());
            }
        };
        let duration_ms = began.elapsed().as_millis() as u64;
        if !alive() {
            machine.handle(AgentEvent::Cancel);
            return Ok(());
        }

        let spans = spans.unwrap_or_default();
        let path_hits: Vec<String> = spans.iter().map(|s| {
            format!("{}:{}-{}", s.path, s.start_line, s.end_line)
        }).collect();
        let detail = if path_hits.is_empty() {
            format!("0 处相关上下文（query=`{context_query}`）")
        } else {
            format!("{} 个相关片段：\n{}", spans.len(), path_hits.join("\n"))
        };
        if !finish_step(
            Step::Search { query: context_query.clone(), detail: detail.clone() },
            duration_ms,
            false,
            emit,
        ) {
            machine.handle(AgentEvent::Cancel);
            return Ok(());
        }
        notes.push(format!("上下文检索 `{context_query}`：{}", spans.len()));

        // Emit FileRead steps + observations for each packed span (range-aware).
        for (i, span) in spans.iter().enumerate() {
            if !alive() {
                machine.handle(AgentEvent::Cancel);
                return Ok(());
            }
            let preview = format!("{}-{}", span.start_line, span.end_line);
            if !run_step(
                Step::FileRead { path: span.path.clone(), detail: preview.clone() },
                alive,
                emit,
            ) {
                machine.handle(AgentEvent::Cancel);
                return Ok(());
            }
            let began = Instant::now();
            let ui_detail: String = span.snippet.chars().take(240).collect();
            let duration_ms = began.elapsed().as_millis() as u64;
            if !finish_step(
                Step::FileRead { path: span.path.clone(), detail: ui_detail },
                duration_ms,
                false,
                emit,
            ) {
                machine.handle(AgentEvent::Cancel);
                return Ok(());
            }
            let mut model_out = span.to_prompt_block();
            if model_out.chars().count() > 2400 {
                model_out = crate::context::truncate_chars_pub(&model_out, 2400);
                model_out.push_str("\n[result truncated: context block capped for model]\n");
            }
            pre_observations.push(ToolResult::success(
                ToolCallId::new(format!("ctx_{i}")),
                ToolName::ReadFile.label(),
                format!("{}:{}-{}", span.path, span.start_line, span.end_line),
                model_out,
            ));
        }

        // Optional short git search observation (kept for compatibility with plan Read subtask).
        let legacy_detail = search_files(project, &context_query);
        pre_observations.push(ToolResult::success(
            ToolCallId::new("pre_search"),
            ToolName::Search.label(),
            context_query.clone(),
            legacy_detail,
        ));
    }

    let (keep, git_obs) = command_step(
        "git status --short",
        ToolCallId::new("pre_git"),
        project,
        request,
        alive,
        approve,
        emit,
        &mut notes,
    )?;
    if let Some(result) = git_obs {
        pre_observations.push(result);
    }
    if !keep {
        machine.handle(AgentEvent::Cancel);
        return Ok(());
    }

    // Context gathered → Execute (or stay ready for model).
    machine.handle(AgentEvent::ContextGathered);
    machine.handle(AgentEvent::ToolsFinished { results: pre_observations.clone() });
    if !simple_step(
        Step::Reasoning { summary: machine.progress_summary() },
        alive,
        emit,
    ) {
        machine.handle(AgentEvent::Cancel);
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
        let mut system = system_prompt(project, &registry);
        if request.extended_thinking {
            system.push_str("\nThink step by step before answering.");
        }
        system.push_str("\n");
        system.push_str(&machine.plan().to_prompt_block());
        system.push_str(
            "Do not claim the task is complete unless acceptance criteria are met by tool evidence.",
        );
        let observation_block = format_observations(&pre_observations);
        let mut history = vec![
            ChatMessage { role: "system".to_owned(), content: system },
            ChatMessage {
                role: "user".to_owned(),
                content: format!("{}\n\n{}", request.message, observation_block),
            },
        ];

        // Multi-round loop driven by the state machine budgets.
        while !machine.state().is_terminal() {
            if !alive() {
                machine.handle(AgentEvent::Cancel);
                break;
            }

            // Verify gate when the machine is in Verify.
            if machine.state() == &AgentState::Verify {
                let verifier = VerificationRunner::new(90_000);
                let outcomes = verifier.run_all(project, alive, true);
                if !alive() {
                    machine.handle(AgentEvent::Cancel);
                    break;
                }
                let all_ok = !outcomes.is_empty() && outcomes.iter().all(|o| o.ok);
                let any_ok = outcomes.iter().any(|o| o.ok);
                // Prefer structured primary failure for the repair nudge.
                let mut repair_hint = String::new();
                if let Some(fail) = outcomes.iter().find(|o| !o.ok) {
                    if let Some(report) = &fail.failure {
                        repair_hint = report.to_prompt_block();
                        notes.push(format!(
                            "验证失败：{}\n{}",
                            report.command,
                            report.primary_error
                        ));
                    } else {
                        notes.push(format!("验证失败：{}", fail.command.command));
                    }
                    // Emit a Command step so the shell/session shows the verify run.
                    let _ = simple_step(
                        Step::Command {
                            command: fail.command.command.clone(),
                            cwd: project.to_string_lossy().into_owned(),
                            output: fail.output_tail.clone(),
                            exit_code: fail.exit_code,
                        },
                        alive,
                        emit,
                    );
                }

                if all_ok {
                    verified = true;
                    machine.handle(AgentEvent::VerifyFinished { ok: true });
                    history.push(ChatMessage {
                        role: "user".to_owned(),
                        content: "All verification commands passed.".to_owned(),
                    });
                } else {
                    verified = any_ok;
                    if !repair_hint.is_empty() {
                        history.push(ChatMessage {
                            role: "user".to_owned(),
                            content: format!(
                                "Verification failed. Compressed failure:\n{repair_hint}\n\
                                 Prefer apply_patch/replace_range on the failing files only. \
                                 Do not run destructive git commands. Do not edit unrelated files."
                            ),
                        });
                    }
                    machine.handle(AgentEvent::VerifyFinished { ok: false });
                }
                continue;
            }

            if machine.state() == &AgentState::Repair {
                if !simple_step(
                    Step::Reasoning { summary: machine.progress_summary() },
                    alive,
                    emit,
                ) {
                    machine.handle(AgentEvent::Cancel);
                    break;
                }
                machine.handle(AgentEvent::RepairApplied);
                continue;
            }

            if !matches!(
                machine.state(),
                AgentState::GatherContext | AgentState::Execute | AgentState::Plan
            ) {
                break;
            }

            match call_model(&provider, &history, request, alive, emit) {
                Ok(Some((text, native_calls, _, _))) => {
                    if !alive() {
                        machine.handle(AgentEvent::Cancel);
                        break;
                    }

                    // Optional: model may propose a refined plan in the first turn.
                    if machine.state() == &AgentState::Plan {
                        if let Some(plan) = TaskPlan::parse_model_json(&text) {
                            *machine.plan_mut() = plan;
                        }
                    }

                    let turn = if !native_calls.is_empty() {
                        ModelTurn::Tools { calls: invocations_from_native(native_calls, &registry) }
                    } else {
                        parse_model_turn(&text, &registry)
                    };

                    let (assistant_text, calls) = match turn {
                        ModelTurn::Final { text } => (text, Vec::new()),
                        ModelTurn::Tools { calls } => (text, calls),
                    };
                    history.push(ChatMessage {
                        role: "assistant".to_owned(),
                        content: assistant_text.clone(),
                    });

                    if calls.is_empty() {
                        // Model claims done — machine evaluates acceptance (not the text).
                        machine.handle(AgentEvent::ModelClaimedDone);
                        answer = strip_tool_artifacts(&assistant_text);
                        checks = checks_from(&assistant_text);
                        if machine.state() == &AgentState::Finish {
                            break;
                        }
                        // Claim rejected: keep looping until budget/terminal.
                        if machine.budget().rounds_exhausted() {
                            break;
                        }
                        // Feed a nudge so the model can continue.
                        history.push(ChatMessage {
                            role: "user".to_owned(),
                            content: format!(
                                "Acceptance criteria are not fully met yet.\n{}\nContinue with tools or fix gaps.",
                                machine.plan().to_prompt_block()
                            ),
                        });
                        continue;
                    }

                    machine.handle(AgentEvent::ModelRequestedTools { count: calls.len() });
                    if !simple_step(
                        Step::Reasoning { summary: machine.progress_summary() },
                        alive,
                        emit,
                    ) {
                        machine.handle(AgentEvent::Cancel);
                        break;
                    }

                    let Some(results) = run_invocations(
                        calls,
                        project,
                        request,
                        alive,
                        approve,
                        emit,
                        &mut notes,
                        &mut wrote_files,
                    )?
                    else {
                        machine.handle(AgentEvent::Cancel);
                        return Ok(());
                    };

                    history.push(ChatMessage {
                        role: "user".to_owned(),
                        content: format_observations(&results),
                    });
                    machine.handle(AgentEvent::ToolsFinished { results });
                    if !simple_step(
                        Step::Reasoning { summary: machine.progress_summary() },
                        alive,
                        emit,
                    ) {
                        machine.handle(AgentEvent::Cancel);
                        break;
                    }

                    // Auto-verify outside the Verify state (e.g. Plan/GatherContext writes).
                    if wrote_files && !verified && machine.state() == &AgentState::Verify {
                        // handled at loop top
                    }
                }
                Ok(None) => {
                    machine.handle(AgentEvent::Cancel);
                    return Ok(());
                }
                Err(error) => {
                    if !request.fallback_to_local {
                        return Err(format!("model call failed: {error}"));
                    }
                    answer = offline_answer(&request.message, &notes, Some(&error));
                    checks = vec!["已使用本地上下文（模型调用失败）".to_owned()];
                    // Do not pretend Finish — explicit failure path.
                    if !machine.state().is_terminal() {
                        machine.handle(AgentEvent::BudgetExceeded);
                    }
                    break;
                }
            }
        }

        if answer.is_empty() {
            match machine.state() {
                AgentState::Failed { reason } => {
                    answer = format!(
                        "任务未完成（{}）。\n\n{}",
                        match reason {
                            FailReason::BudgetExhausted => "预算耗尽",
                            FailReason::AcceptanceUnmet => "验收标准未满足",
                            FailReason::Unrecoverable => "不可恢复错误",
                        },
                        offline_answer(&request.message, &notes, None)
                    );
                    checks = vec!["已明确标记为失败，未伪装成功".to_owned()];
                }
                AgentState::Cancelled => {
                    answer = format!(
                        "已取消。\n\n{}",
                        offline_answer(&request.message, &notes, None)
                    );
                    checks = vec!["用户停止 / 会话取消".to_owned()];
                }
                AgentState::Finish => {
                    answer = offline_answer(&request.message, &notes, None);
                    if checks.is_empty() {
                        checks = vec!["验收标准已由工具证据满足".to_owned()];
                    }
                }
                _ => {
                    answer = offline_answer(&request.message, &notes, None);
                    if checks.is_empty() {
                        checks = vec!["模型未给出最终答复，以下为本地笔记".to_owned()];
                    }
                }
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
        // Explicit verification status — never silent about verify state.
        let status = if verified && wrote_files {
            FinalStatus::Verified
        } else if verified {
            FinalStatus::Verified
        } else if wrote_files {
            FinalStatus::NotVerified
        } else {
            // Read-only tasks that reached Finish without a verify command.
            if machine.state() == &AgentState::Finish {
                FinalStatus::PartiallyVerified
            } else {
                FinalStatus::NotVerified
            }
        };
        // Prefer machine-driven status when verification actually ran this turn.
        let status = if matches!(machine.state(), AgentState::Failed { .. }) {
            FinalStatus::VerificationFailed
        } else if matches!(machine.state(), AgentState::Cancelled) {
            FinalStatus::NotVerified
        } else {
            status
        };

        if verified {
            checks.push("已执行项目验证命令".to_owned());
        }
        if wrote_files {
            checks.push("已写入项目内文件".to_owned());
        }
        checks.push(format!("status: {}", status.label()));
        checks.push(format!("state: {}", machine.state().name()));
        answer = format!("**Verification status:** {}\n\n{}", status.label(), answer);
        simple_step(Step::AgentMessage { text: answer, checks }, alive, emit);
    }
    Ok(())
}

fn system_prompt(project: &Path, registry: &ToolRegistry) -> String {
    format!(
        "You are Kodo, a concise coding agent working in {project}.\n\
         Prefer facts from local notes and tool observations.\n\
         {protocol}\n\
         Paths must stay inside the project (no .. or absolute paths).\n\
         After writing files you may rely on Kodo to run project tests once.\n\
         End with 1-3 short checks as a markdown list starting with '- '.",
        project = project.display(),
        protocol = protocol_instructions(registry),
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

fn strip_tool_artifacts(text: &str) -> String {
    let mut out = String::new();
    let mut skip = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if trimmed.starts_with("```bash")
                || trimmed.starts_with("```write")
                || trimmed.starts_with("```search")
                || trimmed.starts_with("```read")
                || trimmed.starts_with("```json")
                || trimmed.starts_with("```run_command")
                || trimmed.starts_with("```read_file")
                || trimmed.starts_with("```write_file")
            {
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
    use protocol::parse_fence_invocations;

    fn registry() -> ToolRegistry {
        ToolRegistry::standard()
    }

    fn temp_request(dir: &Path, permission: Permission) -> RunRequest {
        RunRequest {
            project: dir.to_path_buf(),
            message: "test".to_owned(),
            pinned_context: Vec::new(),
            provider: None,
            permission,
            fallback_to_local: true,
            max_output_tokens: 512,
            extended_thinking: false,
        }
    }

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
    fn strip_tool_artifacts_hides_write_and_bash() {
        let text = "```write\npath: a\n---\nx\n```\nhello\n```bash\nls\n```\nbye";
        let stripped = strip_tool_artifacts(text);
        assert!(stripped.contains("hello"));
        assert!(stripped.contains("bye"));
        assert!(!stripped.contains("ls"));
    }

    #[test]
    fn strip_tool_artifacts_hides_search_and_read() {
        let text = "```search\nfoo\n```\nkeep\n```read\npath: a.txt\n```\nend";
        let stripped = strip_tool_artifacts(text);
        assert!(stripped.contains("keep"));
        assert!(stripped.contains("end"));
        assert!(!stripped.contains("foo"));
        assert!(!stripped.contains("a.txt"));
    }

    #[test]
    fn execute_tool_call_rejects_read_escape() {
        let dir = std::env::temp_dir().join(format!("kodo-obs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let call = ToolCall {
            id: ToolCallId::new("r1"),
            name: ToolName::ReadFile,
            args: ToolArgs::ReadFile { path: "../escape.txt".to_owned() },
        };
        let mut notes = Vec::new();
        let (keep, result) = execute_tool_call(
            &call,
            &dir,
            &request,
            &|| true,
            &|_, _| true,
            &mut |event| {
                let _ = event;
                true
            },
            &mut notes,
        )
        .expect("execute");
        assert!(keep);
        let result = result.expect("observation");
        assert!(!result.ok);
        assert_eq!(result.id.as_str(), "r1");
        assert!(result.error.as_ref().unwrap().code == ToolErrorCode::PathEscape);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scenario_e_tool_execution_failure_is_recoverable() {
        // Missing file → structured failure, turn keeps going (keep=true).
        let dir = std::env::temp_dir().join(format!("kodo-obs-e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let call = ToolCall {
            id: ToolCallId::new("e1"),
            name: ToolName::ReadFile,
            args: ToolArgs::ReadFile { path: "does-not-exist.txt".into() },
        };
        let mut notes = Vec::new();
        let (keep, result) = execute_tool_call(
            &call,
            &dir,
            &request,
            &|| true,
            &|_, _| true,
            &mut |event| {
                let _ = event;
                true
            },
            &mut notes,
        )
        .expect("execute");
        assert!(keep, "execution failure must not abort the session");
        let result = result.expect("observation");
        assert!(!result.ok);
        assert_eq!(result.error.as_ref().unwrap().code, ToolErrorCode::ExecutionFailed);
        assert_eq!(result.id.as_str(), "e1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn execute_tool_call_run_command_returns_output() {
        let dir = std::env::temp_dir().join(format!("kodo-obs-bash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Full);
        let call = ToolCall {
            id: ToolCallId::new("b1"),
            name: ToolName::RunCommand,
            args: ToolArgs::RunCommand { command: "echo observation-loop".to_owned() },
        };
        let mut notes = Vec::new();
        let (keep, result) = execute_tool_call(
            &call,
            &dir,
            &request,
            &|| true,
            &|_, _| true,
            &mut |event| {
                let _ = event;
                true
            },
            &mut notes,
        )
        .expect("execute");
        assert!(keep);
        let result = result.expect("observation");
        assert!(result.ok, "output={}", result.output);
        assert!(result.output.contains("observation-loop"));
        assert_eq!(result.name, "run_command");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn permission_approval_still_denies_write() {
        let dir = std::env::temp_dir().join(format!("kodo-obs-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let call = ToolCall {
            id: ToolCallId::new("w1"),
            name: ToolName::WriteFile,
            args: ToolArgs::WriteFile { path: "out.txt".into(), content: "x\n".into() },
        };
        let mut notes = Vec::new();
        let (keep, result) = execute_tool_call(
            &call,
            &dir,
            &request,
            &|| true,
            &|_, _| false, // deny
            &mut |event| {
                let _ = event;
                true
            },
            &mut notes,
        )
        .expect("execute");
        assert!(keep, "denial should not abort the turn");
        let result = result.expect("observation");
        assert!(!result.ok);
        assert_eq!(
            result.error.as_ref().unwrap().code,
            ToolErrorCode::PermissionDenied
        );
        assert!(!dir.join("out.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_invocations_maps_rejections_to_results_without_crash() {
        let dir = std::env::temp_dir().join(format!("kodo-inv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let invocations = parse_model_turn(
            r#"{"tool_calls":[
                {"id":"ok","name":"search","arguments":{"query":"foo"}},
                {"id":"bad","name":"nope","arguments":{}},
                {"id":"miss","name":"read_file","arguments":{}}
            ]}"#,
            &registry(),
        );
        let ModelTurn::Tools { calls } = invocations else { panic!("expected tools") };
        let mut notes = Vec::new();
        let mut wrote = false;
        let results = run_invocations(
            calls,
            &dir,
            &request,
            &|| true,
            &|_, _| true,
            &mut |event| {
                let _ = event;
                true
            },
            &mut notes,
            &mut wrote,
        )
        .expect("run")
        .expect("results");
        assert_eq!(results.len(), 3);
        assert!(results[0].ok, "search should succeed: {:?}", results[0]);
        assert!(!results[1].ok);
        assert_eq!(
            results[1].error.as_ref().unwrap().code,
            ToolErrorCode::UnknownTool
        );
        assert!(!results[2].ok);
        assert_eq!(
            results[2].error.as_ref().unwrap().code,
            ToolErrorCode::InvalidArguments
        );
        // All ids preserved
        assert_eq!(results[0].id.as_str(), "ok");
        assert_eq!(results[1].id.as_str(), "bad");
        assert_eq!(results[2].id.as_str(), "miss");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multiple_tool_calls_execute_in_order_not_just_first() {
        // Two run_command calls in one response — both must run.
        let dir = std::env::temp_dir().join(format!("kodo-multi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Full);
        let ModelTurn::Tools { calls } = parse_model_turn(
            r#"{"tool_calls":[
                {"id":"1","name":"run_command","arguments":{"command":"echo first"}},
                {"id":"2","name":"run_command","arguments":{"command":"echo second"}}
            ]}"#,
            &registry(),
        ) else {
            panic!("expected tools")
        };
        let mut notes = Vec::new();
        let mut wrote = false;
        let results = run_invocations(
            calls,
            &dir,
            &request,
            &|| true,
            &|_, _| true,
            &mut |event| {
                let _ = event;
                true
            },
            &mut notes,
            &mut wrote,
        )
        .expect("run")
        .expect("results");
        assert_eq!(results.len(), 2);
        assert!(results[0].output.contains("first"));
        assert!(results[1].output.contains("second"));
        assert_ne!(results[0].id, results[1].id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn native_tool_calls_convert_through_registry() {
        let native = vec![
            NativeToolCall {
                id: "n1".into(),
                name: "read_file".into(),
                arguments: serde_json::json!({"path":"src/lib.rs"}),
            },
            NativeToolCall {
                id: "n2".into(),
                name: "mystery".into(),
                arguments: serde_json::json!({}),
            },
        ];
        let inv = invocations_from_native(native, &registry());
        assert_eq!(inv.len(), 2);
        assert!(matches!(&inv[0], ToolInvocation::Ready(c) if c.id.as_str() == "n1"));
        assert!(matches!(&inv[1], ToolInvocation::Rejected(r) if r.error.code == ToolErrorCode::UnknownTool));
    }

    #[test]
    fn format_observations_embeds_tool_output_with_id() {
        let results = vec![ToolResult::success(
            ToolCallId::new("c1"),
            "run_command",
            "git status --short",
            " M crates/agent/src/lib.rs",
        )];
        let block = format_observations(&results);
        assert!(block.contains("<observations>"));
        assert!(block.contains("id=\"c1\""));
        assert!(block.contains("tool=\"run_command\""));
        assert!(block.contains("status=\"ok\""));
        assert!(block.contains(" M crates/agent/src/lib.rs"));
        assert!(block.contains("git status --short"));
    }

    #[test]
    fn format_observations_marks_errors() {
        let block = format_observations(&[ToolResult::failure(
            ToolCallId::new("e"),
            "write_file",
            "x.rs",
            ToolError::permission_denied("denied"),
        )]);
        assert!(block.contains("status=\"error\""));
        assert!(block.contains("PermissionDenied"));
    }

    #[test]
    fn keywords_skip_stop_words() {
        let keys = keywords_from("请检查 k2k-rust 的 unknown table");
        assert!(keys.iter().any(|k| k.contains("k2k") || k.contains("unknown")));
    }

    #[test]
    fn deprecated_fence_parser_still_available_via_protocol() {
        // Compatibility layer: fences still produce multiple typed calls.
        let calls = parse_fence_invocations(
            "```bash\necho a\n```\n```bash\necho b\n```",
            &registry(),
        );
        assert_eq!(calls.len(), 2, "must not stop at the first bash fence");
        assert!(calls.iter().all(|c| matches!(c, ToolInvocation::Ready(_))));
    }

    #[test]
    fn scenario_c_and_d_via_parse_model_turn() {
        // C: missing args (no path)
        let ModelTurn::Tools { calls } = parse_model_turn(
            r#"{"tool_calls":[{"id":"c","name":"write_file","arguments":{"content":"x"}}]}"#,
            &registry(),
        ) else {
            panic!("tools")
        };
        assert!(matches!(&calls[0], ToolInvocation::Rejected(r) if r.error.code == ToolErrorCode::InvalidArguments));

        // D: unknown tool
        let ModelTurn::Tools { calls } = parse_model_turn(
            r#"{"tool_calls":[{"id":"d","name":"launch_missiles","arguments":{}}]}"#,
            &registry(),
        ) else {
            panic!("tools")
        };
        assert!(matches!(&calls[0], ToolInvocation::Rejected(r) if r.error.code == ToolErrorCode::UnknownTool));
    }
}
