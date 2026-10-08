//! The agent loop behind a conversation turn.
//!
//! The shell owns threads and session logs; this crate decides what work to do.
//! Events are emitted **before** the work runs and again when it finishes, so
//! the GUI can show a live step rather than a post-mortem.
//!
//! Model↔tool traffic goes through the typed [`protocol`] module. The agent
//! loop does not parse Markdown fences itself; it only executes
//! [`protocol::ToolInvocation`] values.

pub mod agents;
pub mod checkpoint;
mod classify;
mod compaction;
mod context;
mod dedup;
pub mod evidence;
pub mod mcp;
pub mod office;
mod patch;
pub mod plan;
pub mod process;
pub mod protocol;
pub mod provider;
pub mod repomap;
mod skill;
#[cfg(test)]
mod skill_flow_tests;
pub mod state;
pub mod tools;
mod verify;
pub mod websearch;

use std::path::{Path, PathBuf};
use std::time::Instant;

use classify::classify;
use context::ContextBudget;
use mcp::{McpClients, McpError};
use patch::{
    apply_patch, create_file, delete_file, replace_range, ApplyPatchArgs, ReplaceRangeArgs,
};
use plan::TaskPlan;
use protocol::{
    format_observations, parse_model_turn, protocol_instructions, ExternalCall, ExternalToolDef,
    ModelTurn, ToolArgs, ToolCall, ToolCallId, ToolError, ToolErrorCode, ToolInvocation, ToolName,
    ToolRegistry, ToolResult,
};
use provider::{NativeToolCall, ProviderEvent, ProviderMessage, ToolSchema};
use skill::{SkillRegistry, SkillSpec};
use state::{AgentEvent, AgentMachine, AgentState, Budget, FailReason};
use tools::{
    classify_command_risk, command_run, read_text, search_files, write_project_file,
    CommandOutcomeKind,
};
use verify::{FailureClass, FinalStatus, RepairDecision, VerificationPlan, VerificationRunner};

pub use checkpoint::{
    unified_diff, TurnChangeSet, UndoConflict, UndoConflictReason, UndoFileState, UndoReport,
};
pub use classify::{classify as classify_task, TaskType};
pub use context::{
    ContextBudget as TurnContextBudget, ContextManager, ContextSpan,
    Observation as TurnObservation, DEFAULT_CONTEXT_CHARS, MAX_HISTORY_MESSAGES,
};
pub use evidence::{
    AcceptanceCriterion, CommandExpectation, EvidenceItem as AgentEvidenceItem,
    EvidenceKind as AgentEvidenceKind, EvidenceRequirement, FailureExpectation, ReproductionRecord,
    SubtaskRequirement,
};
pub use patch::PatchOutcome;
pub use plan::{Evidence, Subtask, SubtaskKind, SubtaskStatus};
pub use process::{
    kill_process_tree, EnvPolicy, ProcessOutcome, ProcessRunner, ProcessSpec, ProcessStatus,
};
pub use protocol::{
    ToolDefinition, ToolError as AgentToolError, ToolRegistry as AgentToolRegistry,
};
pub use provider::{
    catalog_models, fetch_models, resolve_model_identity, ModelSpec, Provider,
    ProviderCapabilities, ProviderConfigRecord, ProviderError, ProviderFailureClass,
};
pub use repomap::{repo_map_cached, repo_map_invalidate, ReferenceEntry, RepoMap, SymbolEntry};
pub use skill::{
    builtin_markdown, delete_user_skill, load_user_skills, parse_skill, save_user_skill,
    ContextStrategy, SkillRegistry as AgentSkillRegistry, SkillSpec as AgentSkillSpec,
    VerificationPolicy, WorkflowStep,
};
pub use state::{AgentState as TurnState, Budget as TurnBudget, FailReason as TurnFailReason};
pub use tools::{dangerous_reason, CommandOutcome, CommandRisk};
pub use verify::{
    classify_verify_failure, CommandFailureKind as TurnFailureKind,
    FailureClass as TurnFailureClass, FinalStatus as TurnFinalStatus, PlannedVerifyCommand,
    RepairDecision as TurnRepairDecision, VerificationEvidence as TurnVerificationEvidence,
    VerificationPlan as TurnVerificationPlan, VerificationRunner as TurnVerifier, VerifyTier,
};

/// Permission mode for tool execution. Default is Ask — Secure by Default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Every shell command and file write requires explicit approval.
    Ask,
    /// Safe commands auto-run; dangerous ones and writes require approval.
    Auto,
    /// Project-local work runs without approval; catastrophic actions still ask.
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
                Self::Full => command
                    .map(classify_command_risk)
                    .map(|risk| risk.needs_approval_in_full())
                    .unwrap_or(false),
                Self::Auto => command
                    .map(|c| classify_command_risk(c).needs_approval_in_auto())
                    .unwrap_or(true),
                // Ask: every command — including Network and PackageInstall —
                // requires explicit approval.
                Self::Ask => true,
            },
            // Writes always need a human in ask/auto; Full allows project-local writes.
            StepKind::FileChange => !matches!(self, Self::Full),
            // External tools leave the machine for a server we don't control —
            // risk is at least Network, so Ask and Auto both require a human
            // (the risk of the call is opaque by construction: an arbitrary
            // server tool). Full lets them through like other project work.
            StepKind::ExternalTool => !matches!(self, Self::Full),
            // Public-web search is a network egress to a third party. Same
            // shape as ExternalTool: a leak here can leak the prompt.
            StepKind::WebSearch => !matches!(self, Self::Full),
            _ => false,
        }
    }
}

/// Global agent mode: code work in a project vs general work tasks.
/// Default is Code — Kodo stays a coding agent unless the user flips the mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    /// General work: documents, summaries, planning, Q&A.
    Work,
    /// Project coding tasks with classification, skills and verification.
    Code,
}

impl AgentMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "work" => Self::Work,
            "code" => Self::Code,
            _ => Self::Code,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Code => "code",
        }
    }
}

impl std::fmt::Display for AgentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
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
    /// External (MCP) tool call. Approval kind only — external work is
    /// traced as [`Step::Command`] so the session log stays closed.
    ExternalTool,
    /// Public-web search via the built-in websearch tool. Same approval shape
    /// as `ExternalTool`: Ask + Auto always require a human, Full passes.
    WebSearch,
    /// New transparency-side kinds below. They never gate approval; the
    /// `Approve` callback defaults them to Allow.
    Thinking,
    Compaction,
    Failover,
    PlanStep,
    Permission,
    ContextBudget,
}

/// One finished unit of work, ready to be written to the session log.
#[derive(Debug, Clone)]
pub enum Step {
    Reasoning {
        /// Short user-safe summary. Never budget/scheduling internals.
        summary: String,
        /// High-level public phase code: prepare/analyze/execute/verify/summarize.
        phase: &'static str,
        /// Internal diagnostics for Debug surfaces; hidden from the default UI.
        diagnostics: Option<String>,
    },
    Search {
        query: String,
        detail: String,
    },
    FileRead {
        path: String,
        detail: String,
    },
    Command {
        command: String,
        cwd: String,
        output: String,
        exit_code: Option<i32>,
    },
    /// One LLM call. The extended metadata rides alongside `model`/`tokens` so
    /// the trace UI can show the protocol path, finish reason, latency, and the
    /// reference into the full I/O archive (kept out of the session log).
    ModelCall {
        model: String,
        input_tokens: u32,
        output_tokens: u32,
        /// `native` / `json_fallback` / `tag_fallback` / `fence_fallback` /
        /// `text_only`. Empty when the agent's exact path is unknown.
        protocol: String,
        /// `stop` / `length` / `tool_calls` / `content_filter` / `cancelled` /
        /// `error`. Empty when unknown.
        finish_reason: String,
        /// Whether the response carried a private reasoning block (extended
        /// thinking). Drives an inline chip in the trace UI.
        saw_reasoning: bool,
        /// Streaming wall time. Recorded so the UI does not have to recompute.
        duration_ms: u64,
        /// Provider-reported end-to-end latency excluding failover retries.
        provider_latency_ms: u64,
        /// `n` for the n-th retry of the same call (0 on first attempt).
        retry_index: u32,
        /// Relative path under `traces/<session>/<turn>/` pointing at the full
        /// request/response JSON. `None` when capture is disabled.
        llm_io_ref: Option<String>,
    },
    FileChange {
        changes: Vec<FileDelta>,
        /// Relative path under `traces/<session>/<turn>/spans/<seq>-file-…json`.
        file_span_ref: Option<String>,
    },
    AgentMessage {
        text: String,
        checks: Vec<String>,
        /// `ready`/`partial`/`blocked`/`failed` — what the answer delivered.
        delivery: String,
        /// `not_run`/`running`/`passed`/`failed`/`blocked` — how acceptance went.
        verification: String,
        /// Conversational answer (你是谁/你好) — the UI renders the text alone,
        /// without the conclusion card or next-step chrome (2026-09-23).
        plain: bool,
    },
    /// A model's private reasoning block. Persisted so the transparency UI
    /// can show what the model was thinking, separate from its public
    /// `Reasoning` summary. `phase` is the public phase the block appeared in;
    /// `source` is which layer surfaced the block.
    Thinking {
        content: String,
        phase: String,
        source: String,
    },
    /// A history-compaction pass. `source_summary` is the text that fed into
    /// the compaction; without it the trace would only show the *event* of
    /// compacting, never the content that got compressed away.
    Compaction {
        dropped: usize,
        summarized_into: usize,
        freed_pct: u8,
        source_summary: String,
        covered_messages: usize,
    },
    /// A failover event. `error_class` is the bucketed cause (`rate_limit` /
    /// `server_error` / `timeout` / `parse` / `unknown`).
    Failover {
        from_provider: String,
        from_model: String,
        error_class: String,
        error: String,
        to_provider: String,
        to_model: String,
        retry_index: u32,
    },
    /// A task-plan subtask status update. `triggered_by` lists the item ids
    /// this step caused, so the trace can be re-walked plan-first.
    PlanStep {
        step_id: String,
        title: String,
        kind: String,
        status: String,
        triggered_by: Vec<u32>,
        evidence: Vec<String>,
    },
    /// An approval decision. Distinct from `Command` with `denied: true`
    /// because that one records the *effect*; this records the *decision*.
    Permission {
        kind: String,
        detail: String,
        decision: String,
        context: String,
    },
    /// Context window snapshot stamped at the same cadence as the live sink
    /// event, so the trace carries both the chip and the detail.
    ContextBudget {
        used: u32,
        window: u32,
        percent: u8,
        cumulative_chars: u64,
        compacted_count: u32,
        last_model: String,
    },
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
            Step::Thinking { .. } => StepKind::Thinking,
            Step::Compaction { .. } => StepKind::Compaction,
            Step::Failover { .. } => StepKind::Failover,
            Step::PlanStep { .. } => StepKind::PlanStep,
            Step::Permission { .. } => StepKind::Permission,
            Step::ContextBudget { .. } => StepKind::ContextBudget,
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
            Step::FileChange { changes, file_span_ref } => Step::FileChange {
                changes: changes.clone(),
                file_span_ref: file_span_ref.clone(),
            },
            other => other.clone(),
        }
    }

    pub fn preview_detail(&self) -> String {
        match self {
            Step::Command { command, .. } => command.clone(),
            Step::Search { query, .. } => query.clone(),
            Step::FileRead { path, .. } => path.clone(),
            Step::ModelCall { model, .. } => model.clone(),
            Step::FileChange { changes, .. } => changes
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
    Started {
        step: Step,
    },
    Finished {
        step: Step,
        duration_ms: u64,
        denied: bool,
    },
    /// Incremental assistant text from the provider stream (batched by caller
    /// if needed). Not persisted as a step — the finished ModelCall/AgentMessage is.
    TextDelta {
        text: String,
    },
    /// Structured progress phase for the UI (never chain-of-thought).
    Progress {
        /// High-level phase code (prepare/analyze/execute/verify/summarize).
        phase: String,
        detail: String,
    },
    /// Provider switched mid-turn (same streaming path; UI must see it).
    Failover {
        from_provider: String,
        from_model: String,
        error_class: String,
        error: String,
        to_provider: String,
        to_model: String,
    },
    /// Live context-window telemetry emitted after every successful model
    /// call. UI consumes this to render the Codex-style progress bar; the
    /// auto-compaction trigger reads `percent` in the agent loop.
    ContextBudget {
        /// Input tokens the most recent call consumed.
        used: u32,
        /// Effective context window for the active model (provider preset
        /// or user override). `> 0` always.
        window: u32,
        /// `used / window` as a 0..=100 integer (already rounded + clamped).
        percent: u8,
        /// Total chars currently held in the agent history (for the bar label).
        cumulative_chars: u64,
        /// Number of compaction passes that succeeded earlier this turn.
        compacted_count: u32,
        /// Display label of the model that produced `used`.
        last_model: String,
    },
    /// A history-compaction pass just finished. UI surfaces a short toast
    /// so the user sees the agent is recovering from budget strain.
    Compacted {
        /// How many older messages got replaced.
        dropped: usize,
        /// Always `1` today (the middle block becomes one summary message).
        summarized_into: usize,
        /// Approximate percentage of the window reclaimed (0..=100).
        freed_pct: u8,
    },
    /// Phase 0 second-pass — model-initiated ask-the-user round-trip. UI is
    /// expected to surface a dialog and feed the answer back into the
    /// session. Emitted at most once per `ask_user` tool call; the runtime
    /// does NOT loop waiting for a reply. Instead it returns a placeholder
    /// ToolResult so the agent loop continues, and a future turn picks up
    /// the user's answer from history. `options` is an optional list of
    /// quick-pick chips the UI can render.
    AskUser {
        question: String,
        options: Vec<String>,
    },
}

pub type Emit<'a> = dyn FnMut(SinkEvent) -> bool + 'a;
pub type Alive<'a> = dyn Fn() -> bool + 'a;
pub type Approve<'a> = dyn Fn(StepKind, &str) -> bool + 'a;

pub struct RunRequest {
    /// Project root. `None` = no project selected (Work mode pure conversation):
    /// no tools run, only pinned attachments are read.
    pub project: Option<PathBuf>,
    pub message: String,
    /// Project-relative paths pinned as context for this turn (not file bodies).
    /// Absolute paths are allowed for external attachments.
    pub pinned_context: Vec<String>,
    pub provider: Option<Provider>,
    pub permission: Permission,
    pub mode: AgentMode,
    pub fallback_to_local: bool,
    pub max_output_tokens: u32,
    pub extended_thinking: bool,
    /// Session id for changeset persistence (undo/diff UI). Optional for tests.
    pub session_id: Option<String>,
    /// Directory holding user-defined `*.md` skills (`state_dir()/skills` at
    /// the entry points). `None` = built-in skills only — tests and evals
    /// stay hermetic and never read the real user directory.
    pub user_skills_dir: Option<PathBuf>,
    /// MCP servers to connect for this run (`ShellConfig.mcp_servers` at the
    /// entry points). Empty = no external tools — tests and evals stay
    /// hermetic. Disabled servers are skipped by `McpClients::connect`.
    pub mcp_servers: Vec<mcp::McpServerConfig>,
    /// Persona instructions resolved by the entry point (插件 · 智能体).
    /// `None` = no agent selected — tests and evals stay hermetic.
    /// Legacy single-persona shape; new callers should prefer `personas`.
    pub agent_instructions: Option<String>,
    /// Persona chain (preferred over `agent_instructions` when non-empty).
    /// Each block carries its defaults / few-shot examples; the run-time
    /// substitutes the persona-declared template variables and emits the
    /// chain header when length > 1.
    pub personas: Vec<PersonaBlock>,
}

/// Where per-turn change sets are persisted for the diff/undo UI.
pub fn changeset_path(project: &Path, session_id: &str) -> PathBuf {
    project
        .join(".kodo")
        .join(format!("changeset-{session_id}.json"))
}

fn persist_changeset(
    project: &Path,
    session_id: Option<&str>,
    changeset: &TurnChangeSet,
) -> Result<(), String> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    if session_id.trim().is_empty() {
        return Ok(());
    }
    let path = changeset_path(project, session_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // One file per session, folded turn over turn: a later turn must never
    // wipe the diffs and ownership flags the UI still needs for earlier ones.
    let merged = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<TurnChangeSet>(&text) {
            Ok(earlier) => earlier.merged_with(changeset.clone()),
            Err(_) => changeset.clone(),
        },
        Err(_) => changeset.clone(),
    };
    let json = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

/// Load a persisted turn changeset (for UI diff view / undo). `Ok(None)` means
/// the turn recorded no changes — nothing was persisted, nothing to show.
pub fn load_changeset(project: &Path, session_id: &str) -> Result<Option<TurnChangeSet>, String> {
    let path = changeset_path(project, session_id);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// Undo only Kodo's changes for a session. A missing changeset means there is
/// nothing to undo — an empty report, not an error. Returns which files were
/// restored and which hit undo conflicts (never snapshot-overwrites user
/// post-turn edits).
pub fn undo_session_changes(project: &Path, session_id: &str) -> Result<UndoReport, String> {
    match load_changeset(project, session_id)? {
        Some(changeset) => changeset.undo_kodo_changes(project),
        None => Ok(UndoReport::default()),
    }
}

fn run_step(step: Step, alive: &Alive, emit: &mut Emit) -> bool {
    if !alive() {
        return false;
    }
    emit(SinkEvent::Started { step })
}

fn finish_step(step: Step, duration_ms: u64, denied: bool, emit: &mut Emit) -> bool {
    emit(SinkEvent::Finished {
        step,
        duration_ms,
        denied,
    })
}

/// Keep=false stops the turn; result carries the observation for the model.
type StepOutcome = Result<(bool, Option<ToolResult>), String>;

/// Executes a command step: approval → start → interruptible run → finish.
#[allow(clippy::too_many_arguments)]
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

    if request
        .permission
        .needs_approval(StepKind::Command, Some(command))
        && !approve(StepKind::Command, command)
    {
        let safe_cmd = tools::redact_secrets(command);
        notes.push(format!("用户拒绝了 `{safe_cmd}`"));
        let keep = finish_step(provisional, 0, true, emit);
        let result = ToolResult::failure(
            call_id,
            ToolName::RunCommand.label(),
            safe_cmd,
            ToolError::permission_denied("用户拒绝了该命令"),
        );
        return Ok((keep, Some(result)));
    }

    if !run_step(provisional.clone(), alive, emit) {
        return Ok((false, None));
    }
    let began = Instant::now();
    let outcome = command_run(project, command, alive, tools::DEFAULT_COMMAND_TIMEOUT_SECS);
    let duration_ms = began.elapsed().as_millis() as u64;
    let output = outcome.output.clone();
    let command_label = tools::redact_secrets(command);
    let code = outcome.exit_code;
    let ok = matches!(outcome.kind, CommandOutcomeKind::Success);
    let summary = match outcome.kind {
        CommandOutcomeKind::Success => "ok",
        CommandOutcomeKind::Failed => "非零退出",
        CommandOutcomeKind::TimedOut => "超时",
        CommandOutcomeKind::Cancelled => "已中断",
        CommandOutcomeKind::Error => "执行错误",
    };
    let result = if ok {
        ToolResult::success(
            call_id,
            ToolName::RunCommand.label(),
            command_label.clone(),
            output.clone(),
        )
    } else {
        let err_code = match outcome.kind {
            CommandOutcomeKind::Cancelled => ToolErrorCode::Interrupted,
            CommandOutcomeKind::TimedOut => ToolErrorCode::ExecutionFailed,
            _ => ToolErrorCode::ExecutionFailed,
        };
        let detail = format!(
            "{}\nexit={}",
            output,
            code.map(|c| c.to_string()).unwrap_or_else(|| "none".into())
        );
        ToolResult::failure(
            call_id,
            ToolName::RunCommand.label(),
            command_label.clone(),
            ToolError::new(err_code, detail),
        )
    };

    if outcome.cancelled && !alive() {
        let finished = Step::Command {
            command: command_label.clone(),
            cwd,
            output,
            exit_code: code,
        };
        let keep = finish_step(finished, duration_ms, false, emit);
        notes.push(format!("`{command_label}` → 已中断"));
        return Ok((keep, Some(result)));
    }
    let finished = Step::Command {
        command: command_label.clone(),
        cwd,
        output,
        exit_code: code,
    };
    let keep = finish_step(finished, duration_ms, false, emit);
    notes.push(format!("`{command_label}` → {summary}"));
    Ok((keep, Some(result)))
}

/// Executes one external (MCP) tool call: approval → start → dispatch → finish.
///
/// Traced as [`Step::Command`] with the wire name in the label — the session
/// log, Inspector and approval UIs need no new shapes for external work. The
/// approval detail carries the redacted args, never raw secrets.
#[allow(clippy::too_many_arguments)]
fn external_step(
    call: ExternalCall,
    project: &Path,
    request: &RunRequest,
    mcp: &mut McpClients,
    alive: &Alive,
    approve: &Approve,
    emit: &mut Emit,
    notes: &mut Vec<String>,
) -> StepOutcome {
    if !alive() {
        return Ok((false, None));
    }
    let wire = call.name.clone();
    let args_label = tools::redact_secrets(&call.args_label());
    let detail = format!("{wire} {args_label}");
    let cwd = project.to_string_lossy().into_owned();
    let provisional = Step::Command {
        command: detail.clone(),
        cwd: cwd.clone(),
        output: String::new(),
        exit_code: None,
    };

    if request
        .permission
        .needs_approval(StepKind::ExternalTool, Some(&detail))
        && !approve(StepKind::ExternalTool, &detail)
    {
        notes.push(format!("用户拒绝了 `{detail}`"));
        let keep = finish_step(provisional, 0, true, emit);
        let result = ToolResult::failure(
            call.id,
            wire,
            detail,
            ToolError::permission_denied("用户拒绝了该外部工具调用"),
        );
        return Ok((keep, Some(result)));
    }

    if !run_step(provisional, alive, emit) {
        return Ok((false, None));
    }
    let began = Instant::now();
    let outcome = mcp.call(&call.name, call.args.clone());
    let duration_ms = began.elapsed().as_millis() as u64;

    let (output, exit_code, result) = match outcome {
        Ok(tool) if !tool.is_error => {
            let result = ToolResult::success(call.id, wire.clone(), detail.clone(), tool.text.clone());
            (tool.text, Some(0), result)
        }
        // Server marked the call failed (`isError: true`) — a tool-level
        // failure the model should see, not a transport error.
        Ok(tool) => {
            let result = ToolResult::failure(
                call.id,
                wire.clone(),
                detail.clone(),
                ToolError::execution(tool.text.clone()),
            );
            (tool.text, Some(1), result)
        }
        Err(error) => {
            let mapped = match &error {
                // Auth failures are permission problems, not tool bugs.
                McpError::Http(401 | 403, _) => {
                    ToolError::permission_denied(format!("{wire}: {error}"))
                }
                _ => ToolError::execution(format!("{wire}: {error}")),
            };
            let output = mapped.message.clone();
            let result = ToolResult::failure(call.id, wire.clone(), detail.clone(), mapped);
            (output, Some(1), result)
        }
    };
    let summary = if exit_code == Some(0) {
        "ok"
    } else {
        "外部调用失败"
    };
    let finished = Step::Command {
        command: detail.clone(),
        cwd,
        output,
        exit_code,
    };
    let keep = finish_step(finished, duration_ms, false, emit);
    notes.push(format!("`{detail}` → {summary}"));
    Ok((keep, Some(result)))
}

/// Approval + write for one file operation.
#[allow(clippy::too_many_arguments)]
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
    // `.kodo/` bookkeeping: no approval, no FileChange step — same rule as
    // `patch_approval_step` (the session log's fileChange items are what the
    // UI counts as "changed files").
    let state_write = checkpoint::is_kodo_state_path(path);
    if !state_write
        && request
            .permission
            .needs_approval(StepKind::FileChange, Some(&label))
        && !approve(StepKind::FileChange, &label)
    {
        notes.push(format!("用户拒绝写入 `{path}`"));
        let denied = Step::FileChange {
            changes: vec![FileDelta {
                path: path.to_owned(),
                added: 0,
                removed: 0,
            }],
            file_span_ref: None,
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
    if state_write {
        return match write_project_file(project, path, content) {
            Ok((rel, added, removed)) => {
                notes.push(format!("写入 `{rel}` (+{added} -{removed})"));
                let result = ToolResult::success(
                    call_id,
                    ToolName::WriteFile.label(),
                    rel.clone(),
                    format!("wrote {rel} (+{added} -{removed})"),
                );
                Ok((true, Some(result)))
            }
            Err(error) => {
                notes.push(format!("写入 `{path}` 失败：{error}"));
                let path_error = if error.contains("escape") || error.contains("absolute") {
                    ToolError::path_escape(error.clone())
                } else {
                    ToolError::execution(error.clone())
                };
                let result =
                    ToolResult::failure(call_id, ToolName::WriteFile.label(), path, path_error);
                Ok((true, Some(result)))
            }
        };
    }

    let provisional = Step::FileChange {
        changes: vec![FileDelta {
            path: path.to_owned(),
            added: 0,
            removed: 0,
        }],
        file_span_ref: None,
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
                Step::FileChange {
                    changes: vec![FileDelta {
                        path: rel,
                        added,
                        removed,
                    }],
                    file_span_ref: None,
                },
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
            let result =
                ToolResult::failure(call_id, ToolName::WriteFile.label(), path, path_error);
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
    dedup: &mut dedup::TurnDedup,
) -> StepOutcome {
    match (&call.name, &call.args) {
        (ToolName::RunCommand, ToolArgs::RunCommand { command }) => command_step(
            command,
            call.id.clone(),
            project,
            request,
            alive,
            approve,
            emit,
            notes,
        ),
        (ToolName::WriteFile, ToolArgs::WriteFile { path, content }) => write_step(
            path,
            content,
            call.id.clone(),
            project,
            request,
            approve,
            emit,
            notes,
        ),
        (ToolName::Search, ToolArgs::Search { query }) => {
            if !alive() {
                return Ok((false, None));
            }
            let step = Step::Search {
                query: query.clone(),
                detail: String::new(),
            };
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
                Step::Search {
                    query: query.clone(),
                    detail: detail.clone(),
                },
                duration_ms,
                false,
                emit,
            );
            // The 已执行 list is a summary, not the trace: keep the count
            // line (the file list lives in the step detail) and fold repeated
            // asks of the same query into one entry — four identical 41-file
            // dumps drowned the answer (2026-09-30).
            let headline = detail.lines().next().unwrap_or("");
            let note = format!("搜索 `{query}`：{headline}");
            if !notes.contains(&note) {
                notes.push(note);
            }
            Ok((
                keep,
                Some(ToolResult::success(
                    call.id.clone(),
                    ToolName::Search.label(),
                    query.clone(),
                    detail,
                )),
            ))
        }
        (ToolName::WebSearch, ToolArgs::WebSearch { query, max_results }) => {
            if !alive() {
                return Ok((false, None));
            }
            let step = Step::Search {
                query: query.clone(),
                detail: String::new(),
            };
            if !run_step(step, alive, emit) {
                return Ok((false, None));
            }
            // Approval kind is WebSearch (network egress), but the session
            // log already shows it as a Search step above — permission has
            // already cleared via the pre-dispatch `approve` gate in the loop.
            let began = Instant::now();
            let cap = max_results.unwrap_or(8).clamp(1, 20);
            let detail = match crate::websearch::search(query, cap) {
                Ok(hits) => {
                    if hits.is_empty() {
                        format!("未找到与 `{}` 相关的网页结果", query)
                    } else {
                        let mut out = String::new();
                        for (i, hit) in hits.iter().enumerate() {
                            out.push_str(&format!(
                                "{}. {}\n   {}\n   {}\n",
                                i + 1,
                                hit.title,
                                hit.url,
                                hit.snippet,
                            ));
                        }
                        out.push_str(&format!(
                            "共 {} 条结果（已截断到 {} 条）",
                            hits.len(),
                            cap,
                        ));
                        out
                    }
                }
                Err(error) => {
                    let note = format!("网络搜索失败：{error}");
                    if !notes.contains(&note) {
                        notes.push(note.clone());
                    }
                    format!("网络搜索失败：{error}")
                }
            };
            let duration_ms = began.elapsed().as_millis() as u64;
            if !alive() {
                return Ok((false, None));
            }
            let keep = finish_step(
                Step::Search {
                    query: query.clone(),
                    detail: detail.clone(),
                },
                duration_ms,
                false,
                emit,
            );
            let note = format!("搜索 `{query}`：{}", detail.lines().next().unwrap_or(""));
            if !notes.contains(&note) {
                notes.push(note);
            }
            Ok((
                keep,
                Some(ToolResult::success(
                    call.id.clone(),
                    ToolName::WebSearch.label(),
                    query.clone(),
                    detail,
                )),
            ))
        }
        // Phase 0 second-pass — model asks the user a question. Surface a
        // `SinkEvent::AskUser` so the UI can pop a dialog; return a
        // placeholder ToolResult so the agent loop continues even when no
        // UI is wired up. The answer is intended to flow back in via the
        // next user message in the same session.
        (ToolName::AskUser, ToolArgs::AskUser { question, options }) => {
            // No permission gate: ask_user is read-only on the codebase
            // and only emits to the sink. We do, however, surface it as a
            // Reasoning step so the UI trace shows the model paused to ask.
            let step_summary = if options.is_empty() {
                format!("询问用户：{question}")
            } else {
                format!(
                    "询问用户：{question}（建议选项：{}）",
                    options.join(" / ")
                )
            };
            let step = Step::Reasoning {
                summary: step_summary,
                phase: "ask",
                diagnostics: Some(format!("ask_user · {question}")),
            };
            let began = Instant::now();
            if !run_step(step, alive, emit) {
                return Ok((false, None));
            }
            let _ = emit(SinkEvent::AskUser {
                question: question.clone(),
                options: options.clone(),
            });
            let duration_ms = began.elapsed().as_millis() as u64;
            let keep = finish_step(
                Step::Reasoning {
                    summary: format!("已询问用户：{question}"),
                    phase: "ask",
                    diagnostics: Some(format!("ask_user · {question}")),
                },
                duration_ms,
                false,
                emit,
            );
            notes.push(format!("询问用户 `{question}`"));
            // Placeholder answer so the tool loop can carry on. The UI is
            // expected to inject the real answer as the next user message;
            // a future turn will see it in history.
            let placeholder = if options.is_empty() {
                "(UI 尚未接线，问题已发出；等待用户在下一轮回复)".to_owned()
            } else {
                format!(
                    "(UI 尚未接线，问题已发出，建议选项：{}；等待用户在下一轮回复)",
                    options.join(" / ")
                )
            };
            Ok((
                keep,
                Some(ToolResult::success(
                    call.id.clone(),
                    ToolName::AskUser.label(),
                    question.clone(),
                    placeholder,
                )),
            ))
        }
        (ToolName::ReadFile, ToolArgs::ReadFile { path }) => {
            if !alive() {
                return Ok((false, None));
            }
            // Any successful read settles the list-files repeat warning,
            // even if it fails — a failed read shows the model moved on.
            dedup.note_read();
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
                Step::FileRead {
                    path: path.clone(),
                    detail: detail.clone(),
                },
                alive,
                emit,
            ) {
                return Ok((false, None));
            }
            let began = Instant::now();
            let keep = finish_step(
                Step::FileRead {
                    path: path.clone(),
                    detail: detail.clone(),
                },
                began.elapsed().as_millis() as u64,
                !ok,
                emit,
            );
            notes.push(format!("读取 `{path}`{}", if ok { "" } else { " 失败" }));
            Ok((keep, Some(result)))
        }
        (
            ToolName::ApplyPatch,
            ToolArgs::ApplyPatch {
                path,
                old,
                new,
                start_line,
            },
        ) => patch_approval_step(
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
        ),
        (
            ToolName::ReplaceRange,
            ToolArgs::ReplaceRange {
                path,
                start_line,
                end_line,
                new_text,
            },
        ) => patch_approval_step(
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
        ),
        (ToolName::CreateFile, ToolArgs::CreateFile { path, content }) => patch_approval_step(
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
        ),
        (ToolName::DeleteFile, ToolArgs::DeleteFile { path }) => patch_approval_step(
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
        ),
        (ToolName::ListFiles, ToolArgs::ListFiles { prefix }) => {
            if !alive() {
                return Ok((false, None));
            }
            // Phase 0 second-pass hard block: when the model has listed the
            // same prefix ≥ 5 times in a row with no intervening read_file,
            // refuse the call instead of executing it. The screenshot showed
            // the model happily making 10 list_files calls on the same
            // prefix, burning through the 6-round budget. Reset dedup state
            // so a different prefix next turn still works.
            if dedup.should_hard_block_list_repeat() {
                let step = Step::Search {
                    query: format!(
                        "list:{} (blocked: duplicate prefix)",
                        prefix.as_deref().unwrap_or("*")
                    ),
                    detail: "[blocked: 已对同一目录连续 list 5 次,请直接读取具体文件]"
                        .to_owned(),
                };
                let began = Instant::now();
                if !run_step(step, alive, emit) {
                    return Ok((false, None));
                }
                let duration_ms = began.elapsed().as_millis() as u64;
                let keep = finish_step(
                    Step::Search {
                        query: format!(
                            "list:{} (blocked)",
                            prefix.as_deref().unwrap_or("*")
                        ),
                        detail: String::new(),
                    },
                    duration_ms,
                    true,
                    emit,
                );
                dedup.note_hard_block();
                notes.push(format!(
                    "list_files → 拒绝：同一目录已列 5 次 (prefix={})",
                    prefix.as_deref().unwrap_or("*")
                ));
                return Ok((
                    keep,
                    Some(ToolResult::failure(
                        call.id.clone(),
                        ToolName::ListFiles.label(),
                        prefix.clone().unwrap_or_default(),
                        ToolError::execution(
                            "duplicate prefix: 已对同一目录连续 list 5 次,请直接 read_file 具体文件",
                        ),
                    )),
                ));
            }
            let step = Step::Search {
                query: format!("list:{}", prefix.as_deref().unwrap_or("*")),
                detail: String::new(),
            };
            if !run_step(step, alive, emit) {
                return Ok((false, None));
            }
            let began = Instant::now();
            let map = crate::repomap::repo_map_cached(project, alive).unwrap_or_default();
            // 50 lines instead of 80 — large directories overload the model's
            // ability to plan, and the per-turn dedup (see `TurnDedup`) makes
            // a smaller window safe to repeat.
            let entries = map.list_files(prefix.as_deref(), 50);
            let mut detail = if entries.is_empty() {
                "0 files".to_owned()
            } else {
                entries
                    .iter()
                    .map(|f| format!("{} ({}, {} lines)", f.path, f.language, f.lines))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            // Dedup: same prefix within the same turn gets a brief suffix so
            // the model knows the answer was just rebuilt, and a third repeat
            // gets a hint to break the cycle.
            let ts_ms = began.elapsed().as_millis() as u64;
            let repeated = dedup.note_list_prefix(prefix.as_deref(), ts_ms);
            if repeated.is_some() {
                if !detail.is_empty() {
                    detail.push('\n');
                }
                detail.push_str("[cached · same prefix this turn]");
            }
            if dedup.should_warn_list_repeat() {
                if !detail.is_empty() {
                    detail.push('\n');
                }
                detail.push_str(
                    "[hint: you have listed this directory several times this turn — read a specific file or skip the listed directory]",
                );
            }
            let duration_ms = began.elapsed().as_millis() as u64;
            let keep = finish_step(
                Step::Search {
                    query: format!("list:{}", prefix.as_deref().unwrap_or("*")),
                    detail: detail.clone(),
                },
                duration_ms,
                false,
                emit,
            );
            notes.push(format!("list_files → {} entries", entries.len()));
            Ok((
                keep,
                Some(ToolResult::success(
                    call.id.clone(),
                    ToolName::ListFiles.label(),
                    prefix.clone().unwrap_or_default(),
                    detail,
                )),
            ))
        }
        (ToolName::FindSymbol, ToolArgs::FindSymbol { name }) => {
            if !alive() {
                return Ok((false, None));
            }
            let step = Step::Search {
                query: format!("symbol:{name}"),
                detail: String::new(),
            };
            if !run_step(step, alive, emit) {
                return Ok((false, None));
            }
            let began = Instant::now();
            // Cached RepoMap: incremental invalidation on file change.
            let map = crate::repomap::repo_map_cached(project, alive).unwrap_or_default();
            let hits = map.find_symbol(name);
            let detail = if hits.is_empty() {
                "0 symbol matches".to_owned()
            } else {
                hits.iter()
                    .map(|s| {
                        format!(
                            "{} {} at {}:{}-{}{}",
                            s.kind,
                            s.name,
                            s.path,
                            s.line,
                            s.end_line,
                            if s.exported { " (exported)" } else { "" }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let duration_ms = began.elapsed().as_millis() as u64;
            let keep = finish_step(
                Step::Search {
                    query: format!("symbol:{name}"),
                    detail: detail.clone(),
                },
                duration_ms,
                false,
                emit,
            );
            notes.push(format!("find_symbol `{name}` → {}", hits.len()));
            Ok((
                keep,
                Some(ToolResult::success(
                    call.id.clone(),
                    ToolName::FindSymbol.label(),
                    name.clone(),
                    detail,
                )),
            ))
        }
        (ToolName::FindReferences, ToolArgs::FindReferences { name }) => {
            if !alive() {
                return Ok((false, None));
            }
            let step = Step::Search {
                query: format!("refs:{name}"),
                detail: String::new(),
            };
            if !run_step(step, alive, emit) {
                return Ok((false, None));
            }
            let began = Instant::now();
            // Syntax-aware word-boundary refs with confidence; lexical fallback
            // labeled explicitly (never pretends precision).
            let map = crate::repomap::repo_map_cached(project, alive).unwrap_or_default();
            let refs = map.find_references(name);
            let detail = if refs.is_empty() {
                // Last resort: file-level lexical search, labeled as fallback.
                let fallback = search_files(project, name);
                if fallback.starts_with('0') {
                    "0 references".to_owned()
                } else {
                    format!("confidence=lexical (repo-map miss)\n{fallback}")
                }
            } else {
                let high = refs.iter().filter(|r| r.confidence == "high").count();
                let mut out = format!(
                    "high={high} lexical={} (definition lines marked)\n",
                    refs.len() - high
                );
                for r in refs.iter().take(40) {
                    out.push_str(&format!(
                        "[{}] {}:{}{}  {}\n",
                        r.confidence,
                        r.path,
                        r.line,
                        if r.is_definition { " (def)" } else { "" },
                        r.snippet
                    ));
                }
                out
            };
            let duration_ms = began.elapsed().as_millis() as u64;
            let keep = finish_step(
                Step::Search {
                    query: format!("refs:{name}"),
                    detail: detail.clone(),
                },
                duration_ms,
                false,
                emit,
            );
            notes.push(format!("find_references `{name}`"));
            Ok((
                keep,
                Some(ToolResult::success(
                    call.id.clone(),
                    ToolName::FindReferences.label(),
                    name.clone(),
                    detail,
                )),
            ))
        }
        (
            ToolName::ReadRange,
            ToolArgs::ReadRange {
                path,
                start_line,
                end_line,
            },
        ) => {
            if !alive() {
                return Ok((false, None));
            }
            let step = Step::FileRead {
                path: path.clone(),
                detail: format!("{start_line}-{end_line}"),
            };
            if !run_step(step, alive, emit) {
                return Ok((false, None));
            }
            let began = Instant::now();
            let (detail, result) = match tools::resolve_in_project(project, path) {
                Err(error) => (
                    format!("路径错误：{error}"),
                    ToolResult::failure(
                        call.id.clone(),
                        ToolName::ReadRange.label(),
                        path.clone(),
                        ToolError::path_escape(error),
                    ),
                ),
                Ok(full) => match tools::read_text_range(&full, *start_line, *end_line, 6000) {
                    Ok(text) => (
                        text.clone(),
                        ToolResult::success(
                            call.id.clone(),
                            ToolName::ReadRange.label(),
                            path.clone(),
                            text,
                        ),
                    ),
                    Err(error) => (
                        format!("读取失败：{error}"),
                        ToolResult::failure(
                            call.id.clone(),
                            ToolName::ReadRange.label(),
                            path.clone(),
                            ToolError::execution(error),
                        ),
                    ),
                },
            };
            let duration_ms = began.elapsed().as_millis() as u64;
            let keep = finish_step(
                Step::FileRead {
                    path: path.clone(),
                    detail,
                },
                duration_ms,
                !result.ok,
                emit,
            );
            notes.push(format!("读取范围 `{path}`"));
            Ok((keep, Some(result)))
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
#[allow(clippy::too_many_arguments)]
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
    // `.kodo/` is Kodo's own state dir: bookkeeping writes never prompt and
    // never emit a FileChange step — the session log's fileChange items are
    // what the reply card, archive counts, and diff panel read as "changed
    // files", and Kodo's own bookkeeping is not one of them.
    let state_write = checkpoint::is_kodo_state_path(path);
    if !state_write
        && request
            .permission
            .needs_approval(StepKind::FileChange, Some(&label))
        && !approve(StepKind::FileChange, &label)
    {
        notes.push(format!("用户拒绝 `{label}`"));
        let denied = Step::FileChange {
            changes: vec![FileDelta {
                path: path.to_owned(),
                added: 0,
                removed: 0,
            }],
            file_span_ref: None,
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
    if state_write {
        return match run(project) {
            Ok(outcome) => {
                notes.push(format!(
                    "{} (+{} -{})",
                    outcome.summary, outcome.added, outcome.removed
                ));
                Ok((
                    true,
                    Some(ToolResult::success(
                        call_id,
                        name.label(),
                        outcome.path.clone(),
                        format!(
                            "{}\nchanged: {} (+{} -{})",
                            outcome.summary, outcome.path, outcome.added, outcome.removed
                        ),
                    )),
                ))
            }
            Err(error) => {
                notes.push(format!("`{label}` 冲突/失败：{}", error.message));
                let result = ToolResult::failure(call_id, name.label(), path.to_owned(), error);
                Ok((true, Some(result)))
            }
        };
    }

    let provisional = Step::FileChange {
        changes: vec![FileDelta {
            path: path.to_owned(),
            added: 0,
            removed: 0,
        }],
        file_span_ref: None,
    };
    if !run_step(provisional.clone(), &|| true, emit) {
        return Ok((false, None));
    }
    let began = Instant::now();
    match run(project) {
        Ok(outcome) => {
            let duration_ms = began.elapsed().as_millis() as u64;
            notes.push(format!(
                "{} (+{} -{})",
                outcome.summary, outcome.added, outcome.removed
            ));
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
                    file_span_ref: None,
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
///
/// When a skill is active, calls for disallowed tools are rejected **before**
/// the Permission approval path (a skill can narrow tools, never widen them).
#[allow(clippy::too_many_arguments)]
fn run_invocations(
    invocations: Vec<ToolInvocation>,
    project: &Path,
    request: &RunRequest,
    skill: Option<&SkillSpec>,
    mcp: &mut McpClients,
    alive: &Alive,
    approve: &Approve,
    emit: &mut Emit,
    notes: &mut Vec<String>,
    wrote_files: &mut bool,
    changeset: &mut TurnChangeSet,
    dedup: &mut dedup::TurnDedup,
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
            ToolInvocation::External(call) => {
                // Skill allowlists narrow builtins only — `mcp__*` labels pass
                // the gate unconditionally (see `SkillSpec::allows_label`) and
                // rely on Permission instead.
                let args_label = call.args_label();
                if let Some(sk) = skill {
                    if let Some(denied) = sk.gate(call.id.clone(), &call.name, &args_label) {
                        notes.push(format!(
                            "Skill `{}` 拒绝工具 `{}`（allowed_tools 白名单）",
                            sk.name, call.name
                        ));
                        results.push(denied);
                        continue;
                    }
                }
                let (keep, result) = external_step(
                    call, project, request, mcp, alive, approve, emit, notes,
                )?;
                if let Some(result) = result {
                    results.push(result);
                }
                if !keep {
                    return Ok(None);
                }
            }
            ToolInvocation::Ready(call) => {
                if let Some(sk) = skill {
                    if let Some(denied) =
                        sk.gate(call.id.clone(), call.name.label(), &call.args.label())
                    {
                        notes.push(format!(
                            "Skill `{}` 拒绝工具 `{}`（allowed_tools 白名单）",
                            sk.name,
                            call.name.label()
                        ));
                        results.push(denied);
                        continue;
                    }
                }
                let is_mutation = call.name.is_mutation();
                // Snapshot pre-mutation content so the Kodo-only diff and undo
                // see the real before state — record_kodo_change runs after the
                // write and would otherwise capture the written file as its own
                // baseline (diff "no changes", undo a no-op).
                if is_mutation {
                    if let Some(path) = call.args.path() {
                        changeset.snapshot_before(project, path.trim());
                    }
                }
                let (keep, result) = execute_tool_call(
                    &call,
                    project,
                    request,
                    alive,
                    approve,
                    emit,
                    notes,
                    dedup,
                )?;
                if let Some(result) = result {
                    // `.kodo/` bookkeeping is not a project-file write.
                    if result.ok && is_mutation && !checkpoint::is_kodo_state_path(&result.input) {
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

/// Tool schemas sent to providers with native tool calling.
fn tool_schemas(registry: &ToolRegistry) -> Vec<ToolSchema> {
    registry
        .definitions()
        .iter()
        .map(|def| ToolSchema {
            name: def.name.to_owned(),
            description: def.description.to_owned(),
            parameters: def.input_schema.clone(),
        })
        .chain(registry.external_definitions().iter().map(|ext| ToolSchema {
            name: ext.name.clone(),
            description: ext.description.clone(),
            // External schemas are the server's `inputSchema`, passed through
            // untouched so native tool calling works with zero mapping rules.
            parameters: ext.input_schema.clone(),
        }))
        .collect()
}

/// Registry-facing view of the connected servers' tools: wire name plus the
/// server's own description and `inputSchema`, passed through untouched.
fn external_defs(mcp: &McpClients) -> Vec<ExternalToolDef> {
    mcp.tools()
        .into_iter()
        .map(|(name, tool)| ExternalToolDef {
            name,
            description: tool.description,
            input_schema: tool.input_schema,
        })
        .collect()
}

/// One successful model attempt after (optional) streaming failover;
/// `provider` is the candidate that actually served the stream.
struct ModelAttempt {
    provider: Provider,
    text: String,
    native_calls: Vec<NativeToolCall>,
    /// Backend finish reason (`stop`, `length`, `tool_calls`, …) — `length`
    /// means the answer hit `max_output_tokens` and is truncated.
    finish_reason: String,
    /// The stream carried `reasoning_content` deltas (thinking-only output
    /// can explain an empty `text` without any provider misconfiguration).
    saw_reasoning: bool,
    /// Reported input-tokens for this call. `0` if the provider does not
    /// surface usage; the caller still charges the budget (zero is fine —
    /// it preserves the "no signal" state for the UI).
    input_tokens: u32,
}

#[allow(clippy::type_complexity)]
fn call_model(
    provider: &Provider,
    history: &[ProviderMessage],
    tools: &[ToolSchema],
    request: &RunRequest,
    alive: &Alive,
    emit: &mut Emit,
) -> Result<Option<ModelAttempt>, String> {
    if !alive() {
        return Ok(None);
    }
    let model_label = provider.display_label();
    let caps = provider.capabilities();
    let use_native = caps.native_tools;
    let call = Step::ModelCall {
        model: model_label.clone(),
        input_tokens: 0,
        output_tokens: 0,
        protocol: if use_native {
            "native".to_owned()
        } else {
            "text_only".to_owned()
        },
        finish_reason: String::new(),
        saw_reasoning: false,
        duration_ms: 0,
        provider_latency_ms: 0,
        retry_index: 0,
        llm_io_ref: None,
    };
    if !run_step(call, alive, emit) {
        return Ok(None);
    }
    let began = Instant::now();
    let mut text = String::new();
    let mut native_calls: Vec<NativeToolCall> = Vec::new();
    let mut usage = (0u32, 0u32);
    let mut saw_reasoning = false;
    let mut stream_err: Option<provider::ProviderError> = None;
    let mut failover_count: u32 = 0;
    let mut last_finish_reason = String::new();
    let tool_list: &[ToolSchema] = if use_native { tools } else { &[] };
    let allow_failover = request.fallback_to_local && !provider.fallbacks.is_empty();

    // Batch deltas so the UI gets frequent-but-not-per-token updates.
    let mut delta_buf = String::new();
    let mut last_flush = Instant::now();

    let result = provider::chat_stream_with_failover(
        provider,
        history,
        tool_list,
        request.max_output_tokens,
        allow_failover,
        alive,
        |event| {
            match event {
                ProviderEvent::TextDelta { text: delta } => {
                    text.push_str(&delta);
                    delta_buf.push_str(&delta);
                    if last_flush.elapsed().as_millis() >= 50 || delta_buf.chars().count() >= 24 {
                        let chunk = std::mem::take(&mut delta_buf);
                        last_flush = Instant::now();
                        if !emit(SinkEvent::TextDelta { text: chunk }) {
                            return false;
                        }
                    }
                }
                ProviderEvent::ToolCallComplete { call } => native_calls.push(call),
                ProviderEvent::ReasoningSeen => saw_reasoning = true,
                ProviderEvent::Usage {
                    input_tokens,
                    output_tokens,
                } => {
                    usage = (input_tokens, output_tokens);
                }
                ProviderEvent::Error { error, class } => {
                    stream_err = Some(provider::ProviderError {
                        class,
                        message: error,
                    });
                }
                ProviderEvent::Failover {
                    from_provider,
                    from_model,
                    error_class,
                    error,
                    to_provider,
                    to_model,
                } => {
                    // Partial stream from the failed candidate is discarded —
                    // the next candidate replays from MessageStart.
                    text.clear();
                    native_calls.clear();
                    usage = (0, 0);
                    saw_reasoning = false;
                    delta_buf.clear();
                    stream_err = None;
                    failover_count = failover_count.saturating_add(1);
                    let from_provider_str = from_provider.clone();
                    let from_model_str = from_model.clone();
                    let error_class_str = format!("{error_class:?}");
                    let error_str = error.clone();
                    let to_provider_str = to_provider.clone();
                    let to_model_str = to_model.clone();
                    if !emit(SinkEvent::Failover {
                        from_provider,
                        from_model,
                        error_class: error_class_str.clone(),
                        error,
                        to_provider,
                        to_model,
                    }) {
                        return false;
                    }
                    // Mirror the toast into a `Step::Failover` item so the
                    // persisted trace shows the same failover the live UI
                    // saw, with retry_index reflecting how many candidates
                    // this turn has already burned through.
                    let _ = emit(SinkEvent::Finished {
                        step: Step::Failover {
                            from_provider: from_provider_str,
                            from_model: from_model_str,
                            error_class: error_class_str,
                            error: error_str,
                            to_provider: to_provider_str,
                            to_model: to_model_str,
                            retry_index: failover_count,
                        },
                        duration_ms: 0,
                        denied: false,
                    });
                }
                _ => {}
            }
            // Every event path honors cancellation immediately.
            alive()
        },
    );
    let cancelled = match &result {
        Err(err) => err.class == provider::ProviderFailureClass::Cancelled,
        Ok(_) => false,
    };
    if cancelled || !alive() {
        // Stop 后不再产生用户可见 TextDelta — drop any unflushed buffer.
        delta_buf.clear();
        return Ok(None);
    }
    if !delta_buf.is_empty() {
        let chunk = std::mem::take(&mut delta_buf);
        if !emit(SinkEvent::TextDelta { text: chunk }) {
            return Ok(None);
        }
    }
    let duration_ms = began.elapsed().as_millis() as u64;
    if !alive() {
        return Ok(None);
    }
    match result.map_err(|err| {
        if stream_err.is_some() {
            stream_err.clone().unwrap_or(err)
        } else {
            err
        }
    }) {
        Ok((served, response)) => {
            let text = if text.is_empty() {
                response.text.clone()
            } else {
                text
            };
            let native_calls = if native_calls.is_empty() {
                response.native_tool_calls.clone()
            } else {
                native_calls
            };
            let input = if usage.0 > 0 {
                usage.0
            } else {
                response.input_tokens
            };
            let output_tokens = if usage.1 > 0 {
                usage.1
            } else {
                response.output_tokens
            };
            last_finish_reason = response.finish_reason.clone();
            if !finish_step(
                Step::ModelCall {
                    model: served.display_label(),
                    input_tokens: input,
                    output_tokens,
                    protocol: if use_native { "native".to_owned() } else { "text_only".to_owned() },
                    finish_reason: last_finish_reason.clone(),
                    saw_reasoning,
                    duration_ms,
                    provider_latency_ms: duration_ms,
                    retry_index: failover_count,
                    llm_io_ref: None,
                },
                duration_ms,
                false,
                emit,
            ) {
                return Ok(None);
            }
            Ok(Some(ModelAttempt {
                provider: served,
                text,
                native_calls,
                finish_reason: response.finish_reason,
                saw_reasoning,
                input_tokens: input,
            }))
        }
        Err(error) => {
            finish_step(
                Step::ModelCall {
                    model: model_label.clone(),
                    input_tokens: 0,
                    output_tokens: 0,
                    protocol: if use_native { "native".to_owned() } else { "text_only".to_owned() },
                    finish_reason: last_finish_reason.clone(),
                    saw_reasoning,
                    duration_ms,
                    provider_latency_ms: duration_ms,
                    retry_index: failover_count,
                    llm_io_ref: None,
                },
                duration_ms,
                true,
                emit,
            );
            // Failover already ran inside chat_stream_with_failover (same path).
            // Remaining error is terminal for this attempt chain.
            if !request.fallback_to_local {
                return Err(format!("model call failed: {error}"));
            }
            Err(error.to_string())
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

/// Append tool results as provider-native tool_result messages when possible.
fn push_tool_results(history: &mut Vec<ProviderMessage>, results: &[ToolResult], native: bool) {
    if native {
        for result in results {
            let body = if result.ok {
                // Bound huge command output: structured head/tail + ref.
                if result.name == "run_command" {
                    crate::context::ContextManager::format_command_output(
                        &result.input,
                        &result.output,
                    )
                } else {
                    let mut out = result.output.clone();
                    if out.chars().count() > crate::context::MAX_HISTORY_OBS_CHARS {
                        out = crate::context::truncate_chars_pub(
                            &out,
                            crate::context::MAX_HISTORY_OBS_CHARS,
                        );
                        out.push_str("\n[output truncated for history budget]");
                    }
                    out
                }
            } else {
                format!(
                    "ERROR: {}",
                    result
                        .error
                        .as_ref()
                        .map(|e| e.message.clone())
                        .unwrap_or_else(|| "tool failed".into())
                )
            };
            history.push(ProviderMessage::tool_result(
                result.id.to_string(),
                body,
                !result.ok,
            ));
        }
    } else {
        history.push(ProviderMessage::user(format_observations(results)));
    }
    // Context budget: never unbounded append of complete history.
    trim_history(history);
}

/// Approximate size of the agent history in chars (UTF-8 char count). Used
/// only for the `cumulative_chars` field on `SinkEvent::ContextBudget` so
/// the UI bar can show "12.4k chars" alongside the token figure when the
/// provider does not report usage. Cheap: O(n) over `history`.
fn history_chars(history: &[ProviderMessage]) -> u64 {
    let mut total: u64 = 0;
    for m in history {
        for block in &m.content {
            match block {
                provider::ContentBlock::Text { text } => {
                    total = total.saturating_add(text.chars().count() as u64);
                }
                provider::ContentBlock::ToolCall {
                    name, arguments, ..
                } => {
                    total = total.saturating_add(name.chars().count() as u64);
                    total =
                        total.saturating_add(arguments.to_string().chars().count() as u64);
                }
                provider::ContentBlock::ToolResult { content, .. } => {
                    total = total.saturating_add(content.chars().count() as u64);
                }
            }
        }
    }
    total
}

/// Cap history size: keep system + last N messages; mark dropped count.
fn trim_history(history: &mut Vec<ProviderMessage>) {
    if history.len() <= crate::context::MAX_HISTORY_MESSAGES {
        return;
    }
    // Preserve leading system messages.
    let mut system_end = 0;
    for (i, m) in history.iter().enumerate() {
        if m.role == provider::MessageRole::System {
            system_end = i + 1;
        } else {
            break;
        }
    }
    let keep_tail = crate::context::MAX_HISTORY_MESSAGES.saturating_sub(system_end);
    if history.len() <= system_end + keep_tail {
        return;
    }
    let drop_from = history.len() - keep_tail;
    // Never drop system prefix.
    let drop_from = drop_from.max(system_end);
    let dropped = drop_from - system_end;
    let mut new_hist: Vec<ProviderMessage> = history.split_off(system_end);
    new_hist.drain(0..(drop_from - system_end).min(new_hist.len()));
    let marker = ProviderMessage::user(format!(
        "[context budget: dropped {dropped} older messages; re-read files if needed]"
    ));
    let mut rebuilt: Vec<ProviderMessage> = history.drain(..system_end).collect();
    rebuilt.push(marker);
    rebuilt.extend(new_hist);
    *history = rebuilt;
}

/// Trigger one compaction pass when the budget is over the configured
/// percent-of-window threshold. No-op on small histories or when the
/// provider returned no token telemetry yet.
///
/// On success, increments `compacted_count` and emits `SinkEvent::Compacted`
/// with the dropped/summarized/freed metrics. On failure (provider error,
/// empty summary), `history` is not mutated — the next turn will still try
/// again, and if the budget eventually exhausts the legacy `trim_history`
/// path takes over. Compaction never raises; the run loop must continue.
fn compact_if_needed(
    history: &mut Vec<ProviderMessage>,
    budget: &mut Budget,
    provider: &Provider,
    alive: &dyn Fn() -> bool,
    emit: &mut Emit,
) {
    let cfg = compaction::CompactConfig::default();
    if !compaction::should_compact(budget, &cfg, history.len()) {
        return;
    }
    if let Some(report) = compaction::compact_history(history, &cfg, provider, alive) {
        budget.record_compaction();
        let _ = emit(SinkEvent::Compacted {
            dropped: report.dropped,
            summarized_into: report.summarized_into,
            freed_pct: report.freed_pct,
        });
        // Persist a `Compaction` step alongside the chalk toast so the trace
        // carries the source content that was compressed away — without it
        // the user only sees "compaction happened", never what got lost.
        if !finish_step(
            Step::Compaction {
                dropped: report.dropped,
                summarized_into: report.summarized_into,
                freed_pct: report.freed_pct,
                source_summary: report.source_summary.clone(),
                covered_messages: report.covered_messages,
            },
            0,
            false,
            emit,
        ) {
            return;
        }
    }
}

pub fn run(
    request: &RunRequest,
    alive: &Alive,
    approve: &Approve,
    emit: &mut Emit,
) -> Result<(), String> {
    let has_project = request.project.is_some();
    // No project is allowed in either mode as constrained Q&A (DESIGN.md §11):
    // the mode × project matrix hands run() an empty tool registry, verify is
    // clamped off, and only absolute attachment paths are accepted — the
    // answer cannot touch the system.
    // Internal root: empty PathBuf when no project — every project-scoped
    // phase below is guarded by `has_project`, so the empty root is never
    // scanned or joined for tool work.
    let root: PathBuf = request.project.clone().unwrap_or_default();
    let project = root.as_path();
    let mut notes: Vec<String> = Vec::new();
    let mut pre_observations: Vec<ToolResult> = Vec::new();

    // Conversational prompts do not need repository context or tools.
    // With pinned attachments present the canned reply would drop them
    // (a greeting + file must reach the model, not a local canned answer),
    // so the shortcut yields — the run continues into the conversational
    // channel below (`is_direct_conversation` is re-checked there).
    if request.pinned_context.is_empty() && is_direct_conversation(&request.message) {
        let answer = direct_conversation_answer(&request.message, request.mode)
            .expect("direct conversation answer");
        if alive() {
            let _ = simple_step(
                Step::AgentMessage {
                    text: answer,
                    checks: Vec::new(),
                    delivery: "ready".to_owned(),
                    verification: "not_run".to_owned(),
                    plain: true,
                },
                alive,
                emit,
            );
        }
        return Ok(());
    }

    // Chip → Skill Registry → skill-shaped plan (real Planner hook). A leading
    // 【技能：id】 line pins the skill directly (built-in or user); otherwise
    // work mode pins the `work` task type and code mode classifies — on the
    // chip-stripped text, since "bug" inside 【技能：bug-fix】 used to pollute
    // keyword scoring. The provider-visible message stays untouched.
    let (chip_ids, classify_text) = split_skill_chip(&request.message);
    let skill_registry =
        build_skill_registry(request.user_skills_dir.as_deref(), request.project.as_deref());
    let chip_skill = resolve_skill(&skill_registry, &chip_ids, request.mode).cloned();
    let task_type = match request.mode {
        // Work mode always pins Work — the two spaces never mix (DESIGN §11),
        // and the space filter already rejected out-of-space chips.
        AgentMode::Work => TaskType::Work,
        // Code mode: a chip pins the skill's own task type (parser guarantees
        // a non-empty list); otherwise classify the stripped message.
        AgentMode::Code => match &chip_skill {
            Some(sk) => sk.applicable_task_types[0],
            None => classify(classify_text),
        },
    };
    // Conversational channel (DESIGN.md §11, 2026-09-29): no project is
    // always constrained Q&A, and a knowledge/explanation question without a
    // pinned skill chip is a question — not a task. Both answer directly:
    // no skill criteria, no plan gates, no acceptance bounce; the first prose
    // answer ends the run. A pinned chip is explicit task intent and keeps
    // the full machinery.
    let conversational = !has_project
        || (chip_ids.is_empty()
            && (is_direct_conversation(classify_text)
                || is_conversational_question(classify_text)));
    let skill = if conversational {
        None
    } else {
        chip_skill.or_else(|| skill_registry.select(task_type).cloned())
    };
    // B8: connect the configured MCP servers before the registry is assembled —
    // their tools enter it as externals under `mcp__<server>__<tool>` names.
    // The direct-conversation early return already happened (a greeting never
    // spawns a server), and projectless Q&A stays sealed (DESIGN.md §11): no
    // project, no connections. One broken server is a zh-CN note, never fatal;
    // `Drop` tears the transports down (stdio process trees, http best-effort
    // DELETE) on every exit path from here.
    let mut mcp = if has_project {
        McpClients::connect(&request.mcp_servers)
    } else {
        McpClients::default()
    };
    notes.append(&mut mcp.notes);
    // Phase 0 second-pass: emit a one-shot MCP status event so the UI
    // trace shows which servers did NOT start (without forcing the model
    // to read the MCP block from the system prompt mid-task).
    if has_project {
        let hint = mcp.status_hint();
        if !hint.is_empty() {
            let _ = emit(SinkEvent::Progress {
                phase: "mcp".to_owned(),
                detail: hint,
            });
        }
    }
    // The mode × project matrix owns the base registry (DESIGN.md §11); a skill
    // only narrows it further. Tool availability therefore never depends on
    // which skill files happen to be embedded. MCP tools join as externals and
    // survive the skill filter (it narrows builtins only) — every call still
    // faces Permission.
    let registry = {
        let base =
            ToolRegistry::for_mode(request.mode, has_project).with_externals(external_defs(&mcp));
        match &skill {
            Some(sk) => sk.filter_registry(base),
            None => base,
        }
    };
    let mut plan = if conversational {
        TaskPlan::conversational(&request.message)
    } else if has_project && is_summary_intent(&request.message) {
        // Read-only summary path (Phase 0 second-pass): tasks like
        // 「为我输出该项目架构」need 30+ tool calls but never mutate the
        // project. Bypass the skill workflow, use the larger read-only
        // budget, and skip verify so the loop doesn't fail-closed on a
        // 30-tool sweep that ends in prose.
        TaskPlan::summarize(&request.message)
    } else {
        match &skill {
            Some(sk) => TaskPlan::from_skill(sk, &request.message),
            None => TaskPlan::from_task(&request.message),
        }
    };
    clamp_plan_verification(&mut plan, request.mode, has_project);
    // Budget selection (Phase 0 second-pass): summary tasks get a much
    // larger tool/round budget but zero repairs. The summary plan already
    // sets `requires_verify = false`, so the loop will not loop through
    // verify phases.
    let budget = if !conversational
        && has_project
        && is_summary_intent(&request.message)
    {
        Budget::for_summary()
    } else {
        Budget::default()
    };
    let mut machine = AgentMachine::with_plan(&request.message, plan, budget);
    // Anchor the planned context window to the resolved provider so the bar
    // and any auto-compaction trigger see a stable denominator all turn.
    if let Some(p) = request.provider.as_ref() {
        machine.budget_mut().set_window(p.context_window());
    }
    machine.handle(AgentEvent::TaskReceived);
    // Classification is internal detail: it lives in the trace diagnostics,
    // never in `notes` (those render verbatim in the offline answer's
    // 已执行 list, where `task_type=… · skill=none` is noise).
    let classification = match &skill {
        Some(sk) => format!(
            "task_type={} · skill={} · strategy={} · verify={}",
            task_type,
            sk.name,
            sk.context_strategy.label(),
            sk.verification_policy.label()
        ),
        None => format!("task_type={task_type} · skill=none"),
    };

    if !simple_step(
        Step::Reasoning {
            summary: "Understanding the request".to_owned(),
            phase: "prepare",
            diagnostics: Some(format!("{} · {}", machine.progress_summary(), classification)),
        },
        alive,
        emit,
    ) {
        machine.handle(AgentEvent::Cancel);
        return Ok(());
    }

    // Plan (skill-shaped or heuristic; model JSON may refine later).
    machine.handle(AgentEvent::PlanReady);
    let _ = emit(SinkEvent::Progress {
        phase: "prepare".into(),
        detail: machine.plan().progress_summary("locked"),
    });
    if !simple_step(
        Step::Reasoning {
            summary: "Planning the work".to_owned(),
            phase: "prepare",
            diagnostics: Some(format!(
                "Plan · {}",
                machine.plan().progress_summary("locked")
            )),
        },
        alive,
        emit,
    ) {
        machine.handle(AgentEvent::Cancel);
        return Ok(());
    }

    // Dynamic lexical context: file map → path/grep ranking → budgeted spans.
    // Budget preset comes from the skill's context_strategy when present.
    let context_budget = skill
        .as_ref()
        .map(|sk| sk.context_strategy.budget())
        .unwrap_or_else(ContextBudget::default);
    // Shared pool cap for the unconditional pinned-attachment block below —
    // captured before the budget moves into the ContextManager.
    let pinned_budget_chars = context_budget.max_chars;
    let mut context_mgr = ContextManager::new(project, context_budget);

    // Repo map for orientation — injected into the system prompt (not proof).
    // Cached + incremental: refreshed when source files change mid-turn.
    let repo_map = if has_project && !conversational {
        crate::repomap::repo_map_cached(project, alive).ok()
    } else {
        None
    };

    // User-pinned context paths: validate inside the project and pin spans first.
    for rel in &request.pinned_context {
        if !alive() {
            machine.handle(AgentEvent::Cancel);
            return Ok(());
        }
        // Without a project there is no relative base: only existing absolute
        // attachment paths are accepted (the composer already showed them).
        if !has_project && !std::path::Path::new(rel).is_absolute() {
            notes.push(format!(
                "上下文读取失败 `{rel}`：无项目时只支持绝对路径附件"
            ));
            pre_observations.push(ToolResult::failure(
                ToolCallId::new(format!("pin_err_{rel}")),
                ToolName::ReadFile.label(),
                rel.clone(),
                ToolError::permission_denied("no project: only absolute attachment paths"),
            ));
            continue;
        }
        match context_mgr.read_range(rel, 1, 400, "pinned by user") {
            Ok(span) => {
                notes.push(format!("钉住上下文 `{}`", span.path));
                context_mgr.pin(span);
            }
            Err(error) => {
                // Existing files that refuse text inlining (images, PDF, Office…)
                // are still valid attachments — the composer card already showed
                // them (2026-09-23). Soft-note and continue; only a missing or
                // invalid path stays a real failure.
                let path = std::path::Path::new(rel);
                let exists = if path.is_absolute() {
                    path.is_file()
                } else {
                    project.join(rel).is_file()
                };
                if exists {
                    notes.push(format!("已附带附件 `{rel}`（内容未内联）"));
                    // The model must see the attachment too — `notes` only
                    // surface in the offline fallback answer. A path/type
                    // descriptor replaces the missing body (COMPONENTS.md
                    // 「已附带附件（内容未内联）」); docx/pptx/pdf/images stay
                    // path-only, spreadsheets never reach this arm (extracted
                    // by `context::read_range` via office::read_spreadsheet).
                    let bytes = path.metadata().map(|m| m.len()).unwrap_or(0);
                    pre_observations.push(ToolResult::success(
                        ToolCallId::new(format!("pin_bin_{rel}")),
                        ToolName::ReadFile.label(),
                        rel.clone(),
                        format!(
                            "<attachment path=\"{rel}\" inlined=\"false\" kind=\"{}\" bytes=\"{bytes}\" />\n\
                             [已附带附件：内容未内联（二进制/Office 格式），本轮无法读取其正文；\n\
                              文件类型与路径如上，回答请以附件为准并说明这一限制。]\n",
                            attachment_kind_label(rel),
                        ),
                    ));
                } else {
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
    }

    // Pinned attachments inline into the first user message on EVERY arm
    // (DESIGN.md §11: a projectless / conversational answer comes from the
    // request and its attachments alone). The task-mode collect below still
    // packs repo hits; spans emitted here are skipped there via `span.pinned`,
    // so each attachment produces exactly one observation.
    let mut pinned_used = 0usize;
    for (i, span) in context_mgr.pinned().to_vec().iter().enumerate() {
        if !alive() {
            machine.handle(AgentEvent::Cancel);
            return Ok(());
        }
        let cost = span.snippet.chars().count() + span.path.len() + 80;
        if pinned_used + cost > pinned_budget_chars {
            // Shared pool exhausted → degrade to a path descriptor; sending
            // never fails because of an attachment.
            pre_observations.push(ToolResult::success(
                ToolCallId::new(format!("pin_budget_{i}")),
                ToolName::ReadFile.label(),
                span.path.clone(),
                format!(
                    "<attachment path=\"{}\" inlined=\"false\" />\n\
                     [内容超出本轮附件预算，仅提供路径；如需正文请告知重点行段。]\n",
                    span.path
                ),
            ));
            continue;
        }
        pinned_used += cost;
        // Same FileRead step the task-mode scan used to emit for pins —
        // shown on every arm so the transcript reflects the real read.
        let began = Instant::now();
        if !run_step(
            Step::FileRead {
                path: span.path.clone(),
                detail: format!("{}-{}", span.start_line, span.end_line),
            },
            alive,
            emit,
        ) {
            machine.handle(AgentEvent::Cancel);
            return Ok(());
        }
        let ui_detail: String = span.snippet.chars().take(240).collect();
        if !finish_step(
            Step::FileRead {
                path: span.path.clone(),
                detail: ui_detail,
            },
            began.elapsed().as_millis() as u64,
            false,
            emit,
        ) {
            machine.handle(AgentEvent::Cancel);
            return Ok(());
        }
        pre_observations.push(span_observation(span, &format!("pin_{i}")));
    }

    let context_query = context_query_for(&request.message, &machine.plan().goal);

    // Project context scan + orientation search — project-only (no project =
    // nothing to scan; the conversation relies on pinned attachments alone).
    // Conversational runs skip it too: a question is answered from the
    // request itself, plus whatever tools the model chooses to call.
    if has_project && !conversational && alive() {
        let search = Step::Search {
            query: context_query.clone(),
            detail: String::new(),
        };
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
        let path_hits: Vec<String> = spans
            .iter()
            .map(|s| format!("{}:{}-{}", s.path, s.start_line, s.end_line))
            .collect();
        let detail = if path_hits.is_empty() {
            format!("0 处相关上下文（query=`{context_query}`）")
        } else {
            format!("{} 个相关片段：\n{}", spans.len(), path_hits.join("\n"))
        };
        if !finish_step(
            Step::Search {
                query: context_query.clone(),
                detail: detail.clone(),
            },
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
            // Pinned attachments were already emitted by the unconditional
            // block above — `collect` re-packs them, skip to avoid duplicates.
            if span.pinned {
                continue;
            }
            if !alive() {
                machine.handle(AgentEvent::Cancel);
                return Ok(());
            }
            let preview = format!("{}-{}", span.start_line, span.end_line);
            if !run_step(
                Step::FileRead {
                    path: span.path.clone(),
                    detail: preview.clone(),
                },
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
                Step::FileRead {
                    path: span.path.clone(),
                    detail: ui_detail,
                },
                duration_ms,
                false,
                emit,
            ) {
                machine.handle(AgentEvent::Cancel);
                return Ok(());
            }
            pre_observations.push(span_observation(span, &format!("ctx_{i}")));
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

    // Orientation git status — only meaningful with a project root, and only
    // for task runs: a plain Q&A turn must not surface a command step.
    if has_project && !conversational {
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
    }

    // Context gathered → Execute (or stay ready for model).
    machine.handle(AgentEvent::ContextGathered);
    let _ = emit(SinkEvent::Progress {
        phase: "analyze".into(),
        detail: format!("{} context spans · repo map ready", pre_observations.len()),
    });
    machine.handle(AgentEvent::ToolsFinished {
        results: pre_observations.clone(),
    });
    if !simple_step(
        Step::Reasoning {
            summary: "Gathering project context".to_owned(),
            phase: "analyze",
            diagnostics: Some(machine.progress_summary()),
        },
        alive,
        emit,
    ) {
        machine.handle(AgentEvent::Cancel);
        return Ok(());
    }

    let provider_ready = request
        .provider
        .as_ref()
        .map(|p| !p.api_key.trim().is_empty() && !p.resolved_model_id().trim().is_empty())
        .unwrap_or(false);

    let mut answer = String::new();
    let mut checks: Vec<String> = Vec::new();
    let mut wrote_files = false;
    let mut verified = false;
    let mut provider_error_seen = false;
    // Per-turn tool-call dedup state. Live only for this `run()` invocation
    // — `AgentMachine` resets between turns, this struct follows the same
    // lifetime. See `dedup::TurnDedup` for the same-prefix / repeat logic.
    let mut turn_dedup = dedup::TurnDedup::new();
    // Why the last final-answer attempt produced no usable text — feeds the
    // honest offline-fallback detail instead of blaming provider config.
    let mut model_gap: Option<String> = None;
    let mut changeset = if has_project {
        TurnChangeSet::capture_baseline(project)
    } else {
        TurnChangeSet::default()
    };
    let mut active_tool = String::from("—");
    let mut repair_attempts = 0usize;
    let mut repair_started: Option<Instant> = None;
    let mut last_verify_plan: Option<VerificationPlan> = None;
    let mut partial_verify = false;

    if provider_ready && alive() {
        let mut provider = request.provider.clone().expect("checked above");
        // Per-persona model override (§2.1). Last persona in the chain
        // wins when multiple set a model — same UX rule as "user last-write
        // wins" for the composer. Cross-provider overrides (persona picks
        // a different template than the active provider) are silently
        // dropped: switching providers mid-run is out of scope.
        apply_persona_model_override(&mut provider, &request.personas);
        // Real failover chain when setting says try next best model.
        // No fallbacks configured → same behavior as fail-fast on provider error.
        if request.fallback_to_local && provider.fallbacks.is_empty() {
            // Keep going without inventing offline "success"; still no fake pass.
        }
        let mut system = system_prompt(
            request.project.as_deref(),
            request.mode,
            &registry,
            &mcp.status_hint(),
        );
        if let Some(map) = &repo_map {
            system.push('\n');
            system.push_str(&map.to_prompt_block());
            // Name only tools this registry actually advertises — the work
            // matrix narrows the symbol tools out (DESIGN.md §11).
            if registry.find("find_symbol").is_some() {
                system.push_str(
                    "Search policy: RepoMap first → find_symbol / find_references (check confidence) \
                     → read_range on hit files only. Prefer targeted ranges over reading whole \
                     unrelated files.\n",
                );
            } else {
                system.push_str(
                    "Search policy: RepoMap first → search for a pattern, then read_file on hit \
                     files only. Prefer targeted reads over whole unrelated files.\n",
                );
            }
        }
        if let Some(sk) = &skill {
            system.push_str(&skill_prompt_block(sk, task_type));
        }
        // Persona chain (§2.1-2.7). When the run was issued with the legacy
        // single-persona field, treat it as a chain of 1 so the same block
        // renderer applies (no template-variable substitution — the legacy
        // shape carried nothing to substitute against).
        let effective_personas: Vec<PersonaBlock> = if request.personas.is_empty() {
            match request.agent_instructions.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(body) => vec![PersonaBlock {
                    name: String::new(),
                    instructions: body.to_owned(),
                    defaults: Default::default(),
                    variables: Vec::new(),
                    examples: Vec::new(),
                }],
                None => Vec::new(),
            }
        } else {
            request.personas.clone()
        };
        let template_vars = build_persona_template_vars(&effective_personas, &provider, request);
        if !effective_personas.is_empty() {
            system.push_str(&persona_chain_block(&effective_personas, &template_vars));
            let examples = examples_block(&effective_personas);
            if !examples.is_empty() {
                system.push_str(&examples);
            }
        }
        if request.extended_thinking {
            system.push_str("\nThink step by step before answering.");
        }
        // Conversational runs carry no plan gates in the prompt: the empty
        // plan block plus "do not claim done without evidence" would ask the
        // model to satisfy criteria that (by design) cannot exist.
        if !conversational {
            system.push('\n');
            system.push_str(&machine.plan().to_prompt_block());
            system.push_str(
                "Do not claim the task is complete unless acceptance criteria are met by tool evidence.",
            );
        }
        let observation_block = format_observations(&pre_observations);
        let mut history = vec![
            ProviderMessage::system(system),
            ProviderMessage::user(format!("{}\n\n{}", request.message, observation_block)),
        ];
        let schemas = tool_schemas(&registry);
        let native_tools = provider.capabilities().native_tools;

        // Multi-round loop driven by the state machine budgets.
        while !machine.state().is_terminal() {
            if !alive() {
                machine.handle(AgentEvent::Cancel);
                break;
            }

            if machine.state() == &AgentState::Verify {
                let _ = emit(SinkEvent::Progress {
                    phase: "verify".into(),
                    detail: format!(
                        "repair attempt {repair_attempts}/{}",
                        machine.budget().max_repairs
                    ),
                });
                let verifier = VerificationRunner::new(90_000);
                let changed: Vec<String> = changeset
                    .kodo_changes()
                    .iter()
                    .map(|p| (*p).clone())
                    .collect();

                // Criterion-scoped plan: exact → package → typecheck → broad.
                let verify_criteria: Vec<(String, String)> = machine
                    .plan()
                    .criteria
                    .iter()
                    .filter(|c| {
                        matches!(
                            c.requirement,
                            crate::evidence::EvidenceRequirement::VerificationPassed
                                | crate::evidence::EvidenceRequirement::SemanticProof {
                                    target: crate::evidence::SemanticTarget::RegressionPrevented
                                }
                        ) || c.description.to_ascii_lowercase().contains("verif")
                    })
                    .map(|c| (c.id.clone(), c.description.clone()))
                    .collect();
                let mut plan = VerificationRunner::build_plan(
                    project,
                    &verify_criteria,
                    &changed,
                    task_type.label(),
                );
                // Skill policy may replace the command list, but we keep
                // criterion bindings from build_plan where possible.
                if let Some(sk) = &skill {
                    let skill_cmds = sk.verification_commands(&verifier, project);
                    if skill_cmds.is_empty() {
                        plan.commands.clear();
                    } else {
                        let skill_set: Vec<String> =
                            skill_cmds.iter().map(|c| c.command.clone()).collect();
                        plan.commands.retain(|pc| {
                            skill_set.iter().any(|s| {
                                *s == pc.command.command || s.contains(&pc.command.command)
                            })
                        });
                    }
                }
                if last_verify_plan.as_ref() != Some(&plan) {
                    // After repair we re-run the same plan (targeted first).
                    last_verify_plan = Some(plan.clone());
                }
                // Always populate plan.verify_target_ids / bindings from plan.
                {
                    let target_ids = plan.targeted_criterion_ids();
                    let bindings: Vec<(String, Vec<String>)> = plan
                        .commands
                        .iter()
                        .map(|c| (c.command.command.clone(), c.criterion_ids.clone()))
                        .collect();
                    let plan_mut = machine.plan_mut();
                    plan_mut.verify_target_ids = target_ids;
                    plan_mut.verify_bindings = bindings;
                }

                if !alive() {
                    machine.handle(AgentEvent::Cancel);
                    break;
                }
                if plan.commands.is_empty() {
                    notes.push("验证失败：当前 Skill 策略与项目没有可执行的验证命令".to_owned());
                    if let Some(evidence) = machine.evidence_mut() {
                        evidence.mark_verify_plan(false, Vec::new(), true);
                    }
                    machine.handle(AgentEvent::VerifyFinished { ok: false });
                    continue;
                }

                // Targeted first: stop after first product failure for repair.
                if !simple_step(
                    Step::Reasoning {
                        summary: "Running verification".to_owned(),
                        phase: "verify",
                        diagnostics: Some(format!(
                            "verify plan: {} command(s)",
                            plan.commands.len()
                        )),
                    },
                    alive,
                    emit,
                ) {
                    machine.handle(AgentEvent::Cancel);
                    break;
                }
                let evidence_list = verifier.run_plan(project, &plan, alive, true);
                if !alive() {
                    machine.handle(AgentEvent::Cancel);
                    break;
                }

                let all_ok = !evidence_list.is_empty() && evidence_list.iter().all(|e| e.ok);
                let any_ok = evidence_list.iter().any(|e| e.ok);
                partial_verify = any_ok && !all_ok;
                let budget_ok = !machine.budget().repair_budget_exhausted()
                    && !machine.budget().any_exhausted();
                let decision = RepairDecision::from_evidence(&evidence_list, &plan, budget_ok);

                // Every planned check lands in the trace — successes stay
                // collapsed; failures carry their root cause for merging.
                for ev in &evidence_list {
                    if !ev.ok {
                        notes.push(format!(
                            "验证失败 [{}] ({})：{}",
                            ev.command,
                            ev.failure_class.map(|c| c.label()).unwrap_or("unknown"),
                            ev.output_summary
                        ));
                    }
                    let _ = simple_step(
                        Step::Command {
                            command: ev.command.clone(),
                            cwd: project.to_string_lossy().into_owned(),
                            output: ev.output_summary.clone(),
                            exit_code: ev.exit_code,
                        },
                        alive,
                        emit,
                    );
                }

                if all_ok {
                    verified = true;
                    partial_verify = false;
                    changeset.verified = Some(true);
                    let cmds = evidence_list
                        .iter()
                        .map(|e| e.command.clone())
                        .collect::<Vec<_>>();
                    machine.plan_mut().verify_commands = cmds;
                    if let Some(ev) = machine.evidence_mut() {
                        ev.mark_verify_plan(true, evidence_list.clone(), false);
                    }
                    machine.handle(AgentEvent::VerifyFinished { ok: true });
                    history.push(ProviderMessage::user(format!(
                        "Verification plan passed (criterion-scoped):\n{}",
                        evidence_list
                            .iter()
                            .map(|e| format!(
                                "- {} → {:?} ({}ms{})",
                                e.command,
                                e.criterion_ids,
                                e.duration_ms,
                                if e.truncated { ", truncated" } else { "" }
                            ))
                            .collect::<Vec<_>>()
                            .join("\n")
                    )));
                } else {
                    changeset.verified = Some(false);
                    let infra = decision.failure_class == FailureClass::Infrastructure
                        || evidence_list
                            .iter()
                            .filter(|e| !e.ok)
                            .all(|e| e.failure_class == Some(FailureClass::Infrastructure));

                    if let Some(ev) = machine.evidence_mut() {
                        ev.mark_verify_plan(false, evidence_list.clone(), infra);
                    }

                    if infra {
                        notes.push(format!("基础设施验证失败（非产品回归）：{}", decision.hint));
                        history.push(ProviderMessage::user(format!(
                            "Infrastructure verification failure (not a product regression):\n{}",
                            decision.hint
                        )));
                        machine.handle(AgentEvent::VerifyFinished { ok: false });
                    } else if decision.budget_exhausted {
                        notes.push(format!("验证失败且修复预算耗尽：{}", decision.hint));
                        history.push(ProviderMessage::user(format!(
                            "Verification failed and repair budget exhausted.\n{}",
                            decision.hint
                        )));
                        machine.handle(AgentEvent::VerifyFinished { ok: false });
                    } else {
                        repair_attempts += 1;
                        // Prefer re-running failed targeted commands after repair.
                        let rerun: Vec<String> = decision
                            .targeted_rerun
                            .iter()
                            .map(|c| c.command.clone())
                            .collect();
                        if !rerun.is_empty() {
                            machine.plan_mut().verify_commands = rerun.clone();
                            notes.push(format!("修复后优先重跑：{}", rerun.join(" && ")));
                        }
                        if !decision.hint.is_empty() {
                            history.push(ProviderMessage::user(format!(
                                "Verification failed (product). Repair then re-run targeted verification:\n{}\n\
                                 Prefer apply_patch/replace_range on the failing files only. \
                                 Do not run destructive git commands. Do not edit unrelated files.",
                                decision.hint
                            )));
                        }
                        machine.handle(AgentEvent::VerifyFinished { ok: false });
                    }
                }
                continue;
            }

            if machine.state() == &AgentState::Repair {
                let wall_begin = Instant::now();
                if repair_started.is_none() {
                    repair_started = Some(Instant::now());
                }
                let _ = emit(SinkEvent::Progress {
                    phase: "execute".into(),
                    detail: format!("attempt {repair_attempts}/{}", machine.budget().max_repairs),
                });
                if !simple_step(
                    Step::Reasoning {
                        summary: format!("Repair attempt {repair_attempts}"),
                        phase: "execute",
                        diagnostics: Some(format!(
                            "{} · repair attempt {repair_attempts}/{}",
                            machine.progress_summary(),
                            machine.budget().max_repairs
                        )),
                    },
                    alive,
                    emit,
                ) {
                    machine.handle(AgentEvent::Cancel);
                    break;
                }
                machine.handle(AgentEvent::RepairApplied);
                machine
                    .budget_mut()
                    .charge_repair_wall(wall_begin.elapsed().as_millis() as u64);
                // After repair, next Verify (or tools→Verify) re-runs targeted plan.
                continue;
            }

            if !matches!(
                machine.state(),
                AgentState::GatherContext | AgentState::Execute | AgentState::Plan
            ) {
                break;
            }

            // Mid-turn compaction: when the window is filling up, summarize
            // the middle of history into one block. Failure falls through to
            // the legacy `trim_history` path on a future budget-exhausted
            // signal; this hook never blocks the turn.
            compact_if_needed(
                &mut history,
                machine.budget_mut(),
                &provider,
                &alive,
                emit,
            );

            match call_model(&provider, &history, &schemas, request, alive, emit) {
                Ok(Some(attempt)) => {
                    // Charge the budget with the reported input tokens and
                    // surface a fresh ContextBudget tick to the UI. The bar
                    // updates every model call, not only on the final turn.
                    machine.budget_mut().charge_tokens(attempt.input_tokens);
                    let used = attempt.input_tokens;
                    let window = machine.budget().window_tokens;
                    let percent = machine.budget().percent_used();
                    let chars = history_chars(&history);
                    let compacted = machine.budget().compacted_count;
                    let last_model = provider.display_label();
                    let _ = emit(SinkEvent::ContextBudget {
                        used,
                        window,
                        percent,
                        cumulative_chars: chars,
                        compacted_count: compacted,
                        last_model: last_model.clone(),
                    });
                    // Persist the snapshot alongside the live tick so the
                    // trace UI can chart context use across the turn, not
                    // just see the latest reading.
                    if !finish_step(
                        Step::ContextBudget {
                            used,
                            window,
                            percent,
                            cumulative_chars: chars,
                            compacted_count: compacted,
                            last_model,
                        },
                        0,
                        false,
                        emit,
                    ) {
                        return Ok(());
                    }
                    if attempt.provider.resolved_model_id() != provider.resolved_model_id()
                        || attempt.provider.display_label() != provider.display_label()
                    {
                        notes.push(format!(
                            "failover: {} → {}",
                            provider.display_label(),
                            attempt.provider.display_label()
                        ));
                        checks.push(format!(
                            "failover used provider: {}",
                            attempt.provider.display_label()
                        ));
                    }
                    provider = attempt.provider;
                    let text = attempt.text;
                    let native_calls = attempt.native_calls;
                    let attempt_finish = attempt.finish_reason;
                    let attempt_saw_reasoning = attempt.saw_reasoning;
                    if !alive() {
                        machine.handle(AgentEvent::Cancel);
                        break;
                    }

                    if machine.state() == &AgentState::Plan && !conversational {
                        if let Some(plan) = TaskPlan::parse_model_json(&text) {
                            let plan = match &skill {
                                Some(sk) => refine_plan_with_skill(plan, sk),
                                None => plan,
                            };
                            *machine.plan_mut() = plan;
                        }
                    }

                    let turn = if !native_calls.is_empty() {
                        ModelTurn::Tools {
                            calls: invocations_from_native(native_calls, &registry),
                        }
                    } else {
                        parse_model_turn(&text, &registry)
                    };

                    let (assistant_text, calls) = match turn {
                        ModelTurn::Final { text } => (text, Vec::new()),
                        ModelTurn::Tools { calls } => (text, calls),
                    };
                    // Provider-native assistant turn: text + tool calls preserved.
                    if native_tools {
                        let tool_calls: Vec<NativeToolCall> = calls
                            .iter()
                            .filter_map(|inv| match inv {
                                ToolInvocation::Ready(call) => Some(NativeToolCall {
                                    id: call.id.to_string(),
                                    name: call.name.label().to_owned(),
                                    arguments: match &call.args {
                                        ToolArgs::RunCommand { command } => {
                                            serde_json::json!({ "command": command })
                                        }
                                        ToolArgs::ReadFile { path } => {
                                            serde_json::json!({ "path": path })
                                        }
                                        ToolArgs::Search { query } => {
                                            serde_json::json!({ "query": query })
                                        }
                                        ToolArgs::WriteFile { path, content } => {
                                            serde_json::json!({ "path": path, "content": content })
                                        }
                                        ToolArgs::CreateFile { path, content } => {
                                            serde_json::json!({ "path": path, "content": content })
                                        }
                                        ToolArgs::DeleteFile { path } => {
                                            serde_json::json!({ "path": path })
                                        }
                                        ToolArgs::ApplyPatch { path, old, new, start_line } => {
                                            serde_json::json!({ "path": path, "old": old, "new": new, "start_line": start_line })
                                        }
                                        ToolArgs::ReplaceRange {
                                            path,
                                            start_line,
                                            end_line,
                                            new_text,
                                        } => serde_json::json!({
                                            "path": path,
                                            "start_line": start_line,
                                            "end_line": end_line,
                                            "new_text": new_text
                                        }),
                                        ToolArgs::ListFiles { prefix } => {
                                            serde_json::json!({ "prefix": prefix })
                                        }
                                        ToolArgs::FindSymbol { name }
                                        | ToolArgs::FindReferences { name } => {
                                            serde_json::json!({ "name": name })
                                        }
                                        ToolArgs::ReadRange {
                                            path,
                                            start_line,
                                            end_line,
                                        } => serde_json::json!({
                                            "path": path,
                                            "start_line": start_line,
                                            "end_line": end_line
                                        }),
                                        ToolArgs::WebSearch { query, max_results } => {
                                            serde_json::json!({
                                                "query": query,
                                                "max_results": max_results
                                            })
                                        }
                                        ToolArgs::AskUser { question, options } => {
                                            serde_json::json!({
                                                "question": question,
                                                "options": options
                                            })
                                        }
                                    },
                                }),
                                // External (MCP) calls must round-trip into
                                // provider history under their wire name —
                                // dropping them would corrupt multi-turn
                                // native tool calling.
                                ToolInvocation::External(call) => Some(NativeToolCall {
                                    id: call.id.to_string(),
                                    name: call.name.clone(),
                                    arguments: call.args.clone(),
                                }),
                                _ => None,
                            })
                            .collect();
                        history.push(ProviderMessage::assistant(
                            assistant_text.clone(),
                            tool_calls,
                        ));
                    } else {
                        history.push(ProviderMessage::assistant(
                            assistant_text.clone(),
                            Vec::new(),
                        ));
                    }

                    if calls.is_empty() {
                        machine.handle(AgentEvent::ModelClaimedDone);
                        answer = strip_tool_artifacts(&assistant_text);
                        checks = checks_from(&assistant_text);
                        if answer.trim().is_empty() {
                            model_gap = Some(
                                empty_answer_detail(&attempt_finish, attempt_saw_reasoning)
                                    .to_owned(),
                            );
                        } else {
                            model_gap = None;
                            if attempt_finish == "length" {
                                notes.push(
                                    "输出达到 max-output-tokens 上限，回答可能不完整（finish_reason=length）"
                                        .to_owned(),
                                );
                            }
                        }
                        // Conversational runs gate nothing: the prose answer
                        // IS the delivery — never the "criteria not met"
                        // bounce that starves plain Q&A into a failed run.
                        if conversational || machine.state() == &AgentState::Finish {
                            break;
                        }
                        if machine.budget().rounds_exhausted() {
                            break;
                        }
                        history.push(ProviderMessage::user(format!(
                            "Acceptance criteria are not fully met yet.\n{}\nContinue with tools or fix gaps.",
                            machine.plan().to_prompt_block()
                        )));
                        continue;
                    }

                    machine.handle(AgentEvent::ModelRequestedTools { count: calls.len() });
                    if !simple_step(
                        Step::Reasoning {
                            summary: "Running tools".to_owned(),
                            phase: "execute",
                            diagnostics: Some(
                                machine.plan().progress_payload(
                                    machine.state().name(),
                                    calls
                                        .first()
                                        .map(|c| c.id().to_string())
                                        .as_deref()
                                        .or(Some("—")),
                                ),
                            ),
                        },
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
                        skill.as_ref(),
                        &mut mcp,
                        alive,
                        approve,
                        emit,
                        &mut notes,
                        &mut wrote_files,
                        &mut changeset,
                        &mut turn_dedup,
                    )?
                    else {
                        machine.handle(AgentEvent::Cancel);
                        return Ok(());
                    };

                    // Checkpoint: record Kodo-touched files after mutations.
                    for result in &results {
                        if result.ok
                            && ToolName::parse(&result.name)
                                .map(|n| n.is_mutation())
                                .unwrap_or(false)
                        {
                            changeset.record_kodo_change(project, &result.input);
                            // Invalidate prior context observations for this path
                            // so the model does not treat stale content as fact.
                            context_mgr.invalidate(&result.input);
                            history.push(ProviderMessage::user(format!(
                                "STALE context: `{}` was modified this turn. \
                                 Earlier reads of this file are outdated — re-read before relying on them.",
                                result.input
                            )));
                        }
                    }

                    push_tool_results(&mut history, &results, native_tools);
                    machine.handle(AgentEvent::ToolsFinished {
                        results: results.clone(),
                    });
                    if let Some(evidence) = machine.evidence_mut() {
                        evidence.absorb_results(&results);
                    }
                    if let Some(result) = results.last() {
                        active_tool = result.name.clone();
                    }
                    if !simple_step(
                        Step::Reasoning {
                            summary: "Reviewing tool results".to_owned(),
                            phase: "execute",
                            diagnostics: Some(
                                machine
                                    .plan()
                                    .progress_payload(machine.state().name(), Some(&active_tool)),
                            ),
                        },
                        alive,
                        emit,
                    ) {
                        machine.handle(AgentEvent::Cancel);
                        break;
                    }
                }
                Ok(None) => {
                    machine.handle(AgentEvent::Cancel);
                    return Ok(());
                }
                Err(error) => {
                    // Failover already ran on the streaming path inside
                    // call_model; a residual error is terminal (no blocking
                    // non-stream fallback, no silent offline).
                    provider_error_seen = true;
                    notes.push(format!("model call failed: {error}"));
                    break;
                }
            }
        }

        // A failed or cancelled run never ships interim prose: that text was a
        // claim the acceptance gate refused (or a draft the user stopped), so
        // the reply is the record itself. Only a genuinely empty answer hit
        // this block before, and a rounds-exhausted run then delivered raw
        // model chatter as the final answer (2026-09-30).
        let claim_rejected =
            matches!(machine.state(), AgentState::Failed { .. } | AgentState::Cancelled);
        if answer.is_empty() || claim_rejected {
            match machine.state() {
                AgentState::Failed { reason } => {
                    answer = format!(
                        "任务未完成（{}）。\n\n{}",
                        match reason {
                            FailReason::BudgetExhausted => "预算耗尽",
                            FailReason::AcceptanceUnmet => "验收标准未满足",
                            FailReason::Unrecoverable => "不可恢复错误",
                        },
                        offline_answer(&request.message, &notes, OfflineReason::Aborted)
                    );
                    checks = vec!["已明确标记为失败，未伪装成功".to_owned()];
                }
                AgentState::Cancelled => {
                    answer = format!(
                        "已取消。\n\n{}",
                        offline_answer(&request.message, &notes, OfflineReason::Aborted)
                    );
                    checks = vec!["用户停止 / 会话取消".to_owned()];
                }
                AgentState::Finish => {
                    answer = offline_answer(
                        &request.message,
                        &notes,
                        OfflineReason::NoAnswer(
                            model_gap
                                .as_deref()
                                .unwrap_or("模型结束回合但未返回正文"),
                        ),
                    );
                    if checks.is_empty() {
                        checks = vec!["验收标准已由工具证据满足".to_owned()];
                    }
                }
                _ => {
                    // The provider was configured (we are inside
                    // `provider_ready`) — never claim it wasn't. A stream
                    // error is a call failure; an empty final text is a
                    // no-answer gap with its own detail.
                    answer = if provider_error_seen {
                        let detail = notes
                            .iter()
                            .find_map(|note| note.strip_prefix("model call failed: "))
                            .unwrap_or("模型调用失败");
                        offline_answer(&request.message, &notes, OfflineReason::CallFailed(detail))
                    } else {
                        offline_answer(
                            &request.message,
                            &notes,
                            OfflineReason::NoAnswer(
                                model_gap.as_deref().unwrap_or("模型调用成功但未返回正文"),
                            ),
                        )
                    };
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
        answer = offline_answer(&request.message, &notes, OfflineReason::NoProvider);
        checks = vec![format!("{reason}，本轮基于本地扫描")];
    }

    // Persist the turn changeset so the UI can show real diffs and undo.
    // Project-only: without a root there is no `.kodo/` to write into.
    if has_project && (!changeset.kodo_touched.is_empty() || !changeset.baseline_dirty.is_empty()) {
        if let Err(error) = persist_changeset(project, request.session_id.as_deref(), &changeset) {
            notes.push(format!("changeset persist failed: {error}"));
        }
    }

    // No end-of-turn FileChange summary: each mutation already emitted its own
    // per-call FileChange step (real +/− for that call). A summary here would
    // double-count those deltas, and the old git working-tree fallback reported
    // unrelated pre-existing dirt as "this turn's changes". Read-only turns
    // therefore report zero modified files — which is the truth.

    if alive() {
        // Explicit verification status — never silent about verify state.
        // Distinguishes CompletedVerified / PartiallyVerified /
        // VerificationFailed / Blocked / Cancelled / ProviderError.
        let status = match machine.state() {
            AgentState::Cancelled => FinalStatus::Cancelled,
            AgentState::Failed { reason } => match reason {
                FailReason::Unrecoverable => FinalStatus::Blocked,
                FailReason::BudgetExhausted => {
                    if partial_verify || machine.evidence().partial_verified {
                        FinalStatus::PartiallyVerified
                    } else {
                        FinalStatus::VerificationFailed
                    }
                }
                FailReason::AcceptanceUnmet => FinalStatus::NotVerified,
            },
            _ => {
                if verified {
                    FinalStatus::Verified
                } else if provider_error_seen {
                    FinalStatus::ProviderError
                } else if machine.evidence().verify_infra_failure {
                    FinalStatus::Blocked
                } else if partial_verify || machine.evidence().partial_verified {
                    FinalStatus::PartiallyVerified
                } else if wrote_files {
                    FinalStatus::NotVerified
                } else if machine.state() == &AgentState::Finish {
                    if request.mode == AgentMode::Work {
                        // Work never verifies (DESIGN.md §11): a delivered
                        // work answer is completion, not a partial verify.
                        FinalStatus::NotVerified
                    } else {
                        // Read-only tasks that reached Finish without a verify command.
                        FinalStatus::PartiallyVerified
                    }
                } else {
                    FinalStatus::NotVerified
                }
            }
        };
        // Conversational runs never grade themselves against task evidence:
        // a Finish-without-verify or budget-exhausted exit both mean the
        // answer landed as Q&A (ready + not_run) — not partial/failed
        // verification (2026-09-29).
        let status = if conversational
            && matches!(
                status,
                FinalStatus::VerificationFailed | FinalStatus::PartiallyVerified
            )
        {
            FinalStatus::NotVerified
        } else {
            status
        };

        // Structured axes — the UI reads these instead of parsing answer text.
        // An answer that landed but could not be fully verified is never
        // "failed delivery": it is partial/ready with failed verification.
        let (delivery, verification) = match status {
            FinalStatus::Verified => ("ready", "passed"),
            FinalStatus::PartiallyVerified => ("partial", "failed"),
            FinalStatus::VerificationFailed => ("ready", "failed"),
            FinalStatus::NotVerified => ("ready", "not_run"),
            FinalStatus::Cancelled => ("partial", "not_run"),
            FinalStatus::Blocked => ("blocked", "blocked"),
            FinalStatus::ProviderError => ("failed", "not_run"),
        };

        if verified {
            checks.push("验收条件验证通过（criterion-scoped）".to_owned());
        } else if partial_verify {
            checks.push("部分验证通过，未达全部验收条件".to_owned());
        }
        if machine.evidence().verify_infra_failure {
            checks.push("基础设施验证失败（非产品回归）".to_owned());
        }
        if wrote_files {
            checks.push("已写入项目内文件".to_owned());
        }
        // Keep the verification state visible in the answer text itself:
        // consumers that only read the final message (evals, benchmarks)
        // must never have to infer it from side channels. Conversational
        // answers never claimed a verification — the footer would only add
        // noise to a plain Q&A sentence (2026-09-29).
        if !conversational {
            if !answer.is_empty() && !answer.ends_with('\n') {
                answer.push('\n');
            }
            answer.push_str(&format!("**Verification status:** {}\n", status.label()));
        }
        simple_step(
            Step::AgentMessage {
                text: answer,
                checks,
                delivery: delivery.to_owned(),
                verification: verification.to_owned(),
                plain: conversational,
            },
            alive,
            emit,
        );
    }
    Ok(())
}

fn system_prompt(
    project: Option<&Path>,
    mode: AgentMode,
    registry: &ToolRegistry,
    mcp_hint: &str,
) -> String {
    let protocol = protocol_instructions(registry);
    let checks = "End with 1-3 short checks as a markdown list starting with '- '.";
    // Phase 0 second-pass: MCP init failures (placeholder template,
    // transport closed) get one-shot exposure in the system prompt. The
    // model already saw them in `mcp.notes`, but the screenshot showed
    // it echoing them into the final "已执行" list. Up-front hint → the
    // model acknowledges the gap in its plan and never wastes a tool
    // slot on a server that isn't running.
    let mcp_block = if mcp_hint.is_empty() {
        String::new()
    } else {
        format!("\nMCP 状态：\n{mcp_hint}\n")
    };
    match (project, mode) {
        (Some(project), AgentMode::Code) => format!(
            "You are Kodo, a concise coding agent working in {project}.\n\
             Prefer facts from local notes and tool observations.\n\
             {protocol}\n\
             Paths must stay inside the project (no .. or absolute paths).\n\
             After writing files you may rely on Kodo to run project tests once.\n\
             {mcp_block}\
             {checks}",
            project = project.display(),
        ),
        (None, AgentMode::Code) => format!(
            "You are Kodo, a concise coding agent.\n\
             No project folder is selected — answer conversationally from the \
             request and its attachments; no file or command tools are available.\n\
             {protocol}\n\
             {checks}",
        ),
        (Some(project), AgentMode::Work) => format!(
            "You are Kodo, a work assistant working in {project}.\n\
             You help with documents, summaries, planning, and everyday writing — \
             not code-only tasks.\n\
             Prefer facts from local notes and tool observations.\n\
             {protocol}\n\
             Paths must stay inside the selected folder (no .. or absolute paths).\n\
             Deliverable quality is the acceptance bar; do not run project test \
             suites as proof for work tasks.\n\
             {mcp_block}\
             {checks}",
            project = project.display(),
        ),
        (None, AgentMode::Work) => format!(
            "You are Kodo, a work assistant.\n\
             You help with documents, summaries, planning, and everyday writing.\n\
             No folder is selected — answer directly from the request and its \
             attachments; no file or command tools are available.\n\
             {protocol}\n\
             {checks}",
        ),
    }
}

/// Assemble the skill registry: builtins → project (`.kodo/skills`) → user.
/// Project skills shadow user skills on a name collision (`get()` is
/// first-match), matching the git local-over-global convention — team skills
/// checked into the repo win over personal ones. Builtin names stay reserved
/// in both dirs (the loaders skip them), and `select()` keeps landing on
/// builtins first. Missing dirs are fine; tests pass no dirs and stay hermetic.
fn build_skill_registry(user_skills_dir: Option<&Path>, project: Option<&Path>) -> SkillRegistry {
    let user = user_skills_dir.map(load_user_skills).unwrap_or_default();
    let project_skills = project
        .map(|p| load_user_skills(&p.join(".kodo").join("skills")))
        .unwrap_or_default();
    SkillRegistry::builtin()
        .with_user(user)
        .with_user(project_skills)
}

/// The active-skill block appended to the system prompt. The skill body is
/// loaded **on match only** (progressive disclosure): description + guidance
/// reach the model here, workflow/criteria travel in the plan block instead.
fn skill_prompt_block(skill: &SkillSpec, task_type: TaskType) -> String {
    let mut out = format!(
        "\nActive skill: {} (task_type={}, strategy={}, verify_policy={}). \
         The available tool list above is already filtered by this skill. \
         Follow the plan's workflow and acceptance criteria; never claim \
         done without tool/verification evidence.\n",
        skill.name,
        task_type,
        skill.context_strategy.label(),
        skill.verification_policy.label(),
    );
    if !skill.description.is_empty() {
        out.push_str(&format!("Skill description: {}\n", skill.description));
    }
    if !skill.guidance.is_empty() {
        out.push_str(&format!("Skill guidance:\n{}\n", skill.guidance));
    }
    out
}

/// One persona's resolved shape, lifted into the core so `agent::run` can
/// consume the chain without re-walking the user / project / builtin stores.
/// The frontend / shell populates this list; tests build one inline.
#[derive(Debug, Clone)]
pub struct PersonaBlock {
    pub name: String,
    pub instructions: String,
    pub defaults: crate::agents::PersonaDefaults,
    pub variables: Vec<String>,
    pub examples: Vec<crate::agents::PersonaExample>,
}

/// Substitute every `{{var_name}}` placeholder in `body` with the
/// matching entry from `vars`. Unknown placeholders are left as-is so the
/// prompt stays debuggable when a persona declares a stale variable.
/// `allowed_names` gates substitution: only listed names are touched.
fn substitute_template_vars(
    body: &str,
    vars: &std::collections::HashMap<String, String>,
    allowed: &[String],
) -> String {
    if allowed.is_empty() {
        return body.to_owned();
    }
    let mut out = String::with_capacity(body.len());
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // find the matching `}}`
            let start = i + 2;
            let mut end = start;
            while end + 1 < bytes.len() && !(bytes[end] == b'}' && bytes[end + 1] == b'}') {
                end += 1;
            }
            if end + 1 < bytes.len() {
                let name = body[start..end].trim();
                if allowed.iter().any(|n| n == name) {
                    if let Some(value) = vars.get(name) {
                        out.push_str(value);
                    } else {
                        // known variable, missing value — keep the
                        // placeholder so the user can see the bind failed
                        out.push_str("{{");
                        out.push_str(name);
                        out.push_str("}}");
                    }
                } else {
                    // undeclared variable — keep verbatim
                    out.push_str("{{");
                    out.push_str(name);
                    out.push_str("}}");
                }
                i = end + 2;
                continue;
            }
        }
        // Safety: this is UTF-8; pushing a single byte from a multi-byte
        // sequence would corrupt text. Use char_indices for correctness.
        let rest = body[i..].chars().next().expect("non-empty");
        out.push(rest);
        i += rest.len_utf8();
    }
    out
}

/// Render the persona chain into a single system-prompt appendix. A chain
/// of length 1 renders exactly like the legacy single-persona block; a
/// longer chain prepends a "personas in order" header so the model can
/// tell which persona authored which guideline.
fn persona_chain_block(personas: &[PersonaBlock], vars: &std::collections::HashMap<String, String>) -> String {
    let mut blocks: Vec<String> = Vec::new();
    for persona in personas {
        let body = persona.instructions.trim();
        if body.is_empty() {
            continue;
        }
        let rendered = substitute_template_vars(body, vars, &persona.variables);
        blocks.push(rendered);
    }
    if blocks.is_empty() {
        return String::new();
    }
    if blocks.len() == 1 {
        return format!("\nActive agent persona:\n{}\n", blocks[0]);
    }
    let mut out = String::from("\nActive agent personas (in order):\n");
    for (idx, persona) in personas.iter().enumerate() {
        if persona.instructions.trim().is_empty() {
            continue;
        }
        out.push_str(&format!(
            "{}. {}:\n{}\n",
            idx + 1,
            persona.name,
            substitute_template_vars(persona.instructions.trim(), vars, &persona.variables)
        ));
    }
    out
}

/// Few-shot examples emitted after the persona block. Each example is a
/// labelled pair; the format is human-readable enough for the model to
/// distinguish `Input:` from `Output:`.
fn examples_block(personas: &[PersonaBlock]) -> String {
    let mut out = String::new();
    for persona in personas {
        if persona.examples.is_empty() {
            continue;
        }
        out.push_str(&format!("\nFew-shot examples from `{}`:\n", persona.name));
        for (idx, example) in persona.examples.iter().enumerate() {
            out.push_str(&format!(
                "Example {}:\nInput:\n{}\nOutput:\n{}\n",
                idx + 1,
                example.input.trim(),
                example.output.trim(),
            ));
        }
    }
    out
}

/// Apply the per-persona model override in-place. Walks the chain tail-first
/// so the last persona with a model setting wins. Provider-id mismatches
/// (persona says `defaults.provider = "anthropic"` but the active provider
/// is OpenAI) are silently skipped so we never silently switch providers.
fn apply_persona_model_override(provider: &mut Provider, personas: &[PersonaBlock]) {
    for persona in personas.iter().rev() {
        let Some(model) = persona.defaults.model.as_deref() else {
            continue;
        };
        if let Some(provider_id) = persona.defaults.provider.as_deref() {
            if provider_id != provider.template {
                continue;
            }
        }
        provider.model_id = Some(model.to_owned());
        provider.model = model.to_owned();
        return;
    }
}

/// Build the substitution table for persona template variables. Every
/// persona in the chain reads from the same table — the substitution
/// layer ignores undeclared names so a chain that mixes declared and
/// undeclared variables still works.
fn build_persona_template_vars(
    _personas: &[PersonaBlock],
    provider: &Provider,
    request: &RunRequest,
) -> std::collections::HashMap<String, String> {
    let mut vars = std::collections::HashMap::new();
    if let Some(path) = request.project.as_ref() {
        vars.insert("project_path".to_owned(), path.display().to_string());
    } else {
        vars.insert("project_path".to_owned(), String::new());
    }
    vars.insert("provider".to_owned(), provider.template.clone());
    vars.insert(
        "model".to_owned(),
        provider.model_id.clone().unwrap_or_else(|| provider.model.clone()),
    );
    // Branch is best-effort: read from a `KODO_BRANCH` env var first, then
    // walk a `git rev-parse --abbrev-ref HEAD` for the active project. A
    // failure leaves the slot empty — declared `branch` variables substitute
    // to "" rather than a hardcoded literal.
    let branch = std::env::var("KODO_BRANCH").ok().unwrap_or_else(|| {
        if let Some(project) = request.project.as_ref() {
            std::process::Command::new("git")
                .arg("-C")
                .arg(project)
                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                .output()
                .ok()
                .and_then(|o| {
                    if o.status.success() {
                        String::from_utf8(o.stdout).ok()
                    } else {
                        None
                    }
                })
                .map(|s| s.trim().to_owned())
                .unwrap_or_default()
        } else {
            String::new()
        }
    });
    vars.insert("branch".to_owned(), branch);
    vars
}

/// Clamp a model-refined plan to the active skill's contract:
/// policy owns `requires_verify`, skill completion criteria are a floor,
/// and edit work is stripped when the skill forbids mutations.
fn refine_plan_with_skill(mut plan: TaskPlan, skill: &SkillSpec) -> TaskPlan {
    plan.requires_verify = skill.verification_policy.needs_run();
    // Model-refined plans start at the code bar; re-stamp the work-mode
    // deliverable bar (DESIGN.md §11) from the active skill's task family.
    plan.deliverable_bar = skill.applicable_task_types.contains(&TaskType::Work);
    for criterion in &skill.completion_criteria {
        if !plan.acceptance_criteria.iter().any(|c| c == criterion) {
            plan.acceptance_criteria.push(criterion.clone());
        }
    }
    let can_write = skill.allowed_tools.iter().any(|t| t.is_mutation());
    if !can_write {
        plan.subtasks.retain(|s| s.kind != plan::SubtaskKind::Edit);
        if plan.subtasks.is_empty() {
            plan.subtasks = skill
                .workflow
                .iter()
                .map(|w| Subtask::new(w.id.clone(), w.title.clone(), w.kind))
                .collect();
        }
        plan.current_subtask = plan.current_subtask.min(plan.subtasks.len());
    }
    plan
}

/// Search query for the context scan: the request's keywords plus the plan
/// goal's, deduplicated while keeping order. Plain `Vec::dedup` only collapses
/// adjacent duplicates, so a goal that restates the message (the common case)
/// printed every token twice in the 上下文检索 note (2026-09-30).
fn context_query_for(message: &str, goal: &str) -> String {
    let mut all = keywords_from(message);
    for key in keywords_from(goal) {
        if !all.contains(&key) {
            all.push(key);
        }
    }
    if all.is_empty() {
        "src".to_owned()
    } else {
        all.join(" ")
    }
}

fn keywords_from(message: &str) -> Vec<String> {
    message
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c.is_ascii_punctuation()))
        .filter(|token| {
            let count = token.chars().count();
            (2..=40).contains(&count)
        })
        .filter(|token| {
            let lower = token.to_ascii_lowercase();
            !matches!(
                lower.as_str(),
                "the"
                    | "and"
                    | "for"
                    | "with"
                    | "that"
                    | "this"
                    | "一下"
                    | "检查"
                    | "进行"
                    | "是否"
            )
        })
        .take(4)
        .map(|token| token.to_owned())
        .collect()
}

/// One observation for a context span: prompt block capped at the observation
/// budget, with the legacy `[OLD VERSION]` marker kept for consumers that
/// match on it (`to_prompt_block` already embeds the full stale notice).
/// Ids use `ctx_`/`pin_` prefixes so evidence.rs classifies these as context
/// observations, never as proof.
fn span_observation(span: &ContextSpan, id: &str) -> ToolResult {
    let mut model_out = span.to_prompt_block();
    if model_out.chars().count() > 2400 {
        model_out = crate::context::truncate_chars_pub(&model_out, 2400);
        model_out.push_str("\n[result truncated: context block capped for model]\n");
    }
    if span.stale {
        model_out = format!("[OLD VERSION]\n{model_out}");
    }
    ToolResult::success(
        ToolCallId::new(id.to_owned()),
        ToolName::ReadFile.label(),
        format!("{}:{}-{}", span.path, span.start_line, span.end_line),
        model_out,
    )
}

/// Attachment type label for the binary path descriptor (by extension — the
/// probe's precise kind never reaches the backend).
fn attachment_kind_label(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => "pdf",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "avif" | "heic" => "image",
        "doc" | "docx" | "rtf" | "odt" => "doc",
        "ppt" | "pptx" | "odp" => "ppt",
        "xls" | "xlsx" | "xlsm" | "xlsb" | "ods" => "office",
        _ => "binary",
    }
}

/// Why a run fell back to a local (no-model) answer. Drives the opener, the
/// detail line and the closer so every branch tells the truth — only a
/// genuinely missing provider may claim "not configured".
#[derive(Clone, Copy)]
enum OfflineReason<'a> {
    /// No provider entry, empty API key, or empty model id.
    NoProvider,
    /// A configured provider's call errored out (stream/auth/HTTP failure).
    CallFailed(&'a str),
    /// The model was called but no usable final text came back.
    NoAnswer(&'a str),
    /// The run ended without an answer for non-provider reasons (cancel,
    /// machinery failure) — the caller already prefixed its own explanation.
    Aborted,
}

/// Honest detail for a successful call that produced no usable text: what
/// the finish reason and reasoning stream actually tell us.
fn empty_answer_detail(finish_reason: &str, saw_reasoning: bool) -> &'static str {
    match (finish_reason == "length", saw_reasoning) {
        (true, true) => "模型思考耗尽输出上限（finish_reason=length），未产出正文",
        (true, false) => "输出达到 max-output-tokens 上限被截断，未产出正文",
        (false, true) => "模型输出全部为思考内容，未产出正文",
        (false, false) => "模型调用成功但未返回正文",
    }
}

fn offline_answer(message: &str, notes: &[String], reason: OfflineReason<'_>) -> String {
    let mut body = String::new();
    match reason {
        OfflineReason::NoProvider => {
            body.push_str("本轮未配置可用的模型服务，以下是基于项目本地扫描的说明。\n\n");
        }
        OfflineReason::CallFailed(_) => {
            body.push_str("模型调用未成功，以下是基于项目本地扫描的说明。\n\n");
        }
        OfflineReason::NoAnswer(_) => {
            body.push_str("本轮模型已调用但未产出可用回答，以下是基于项目本地扫描的说明。\n\n");
        }
        OfflineReason::Aborted => {
            body.push_str("以下是基于项目本地扫描的说明。\n\n");
        }
    }
    body.push_str(&format!("**任务**：{message}\n\n"));
    if !notes.is_empty() {
        body.push_str("**已执行**\n");
        for note in notes {
            body.push_str(&format!("- {note}\n"));
        }
        body.push('\n');
    }
    match reason {
        OfflineReason::CallFailed(error) => {
            body.push_str(&format!("**模型错误**：{error}\n\n"));
            body.push_str(
                "配置 Settings → AI Provider 的 API Key 后，可获得结合代码语义的完整答复。",
            );
        }
        OfflineReason::NoAnswer(detail) => {
            body.push_str(&format!("**原因**：{detail}\n\n"));
            body.push_str("可在 Settings → 模型中调大「最大输出 tokens」后重试。");
        }
        OfflineReason::NoProvider => {
            body.push_str(
                "配置 Settings → AI Provider 的 API Key 后，可获得结合代码语义的完整答复。",
            );
        }
        OfflineReason::Aborted => {
            // Cancelled / budget-exhausted runs: the provider is configured
            // and working — blaming missing config would be false. The caller
            // already prefixed its own explanation; nothing else to add.
        }
    }
    body
}

fn direct_conversation_answer(message: &str, mode: AgentMode) -> Option<String> {
    let normalized = message
        .trim()
        .trim_matches(|character: char| matches!(character, '?' | '？' | '!' | '！' | '.' | '。'))
        .to_ascii_lowercase();
    let answer = match normalized.as_str() {
        "你是谁" | "你叫什么" | "who are you" | "what are you" => match mode {
            AgentMode::Work => "我是 Kodo，一个帮助你处理文档、总结和日常事务的工作助手。",
            AgentMode::Code => "我是 Kodo，一个帮助你阅读、修改和验证当前项目代码的编程助手。",
        },
        "你好" | "hi" | "hello" => match mode {
            AgentMode::Work => "你好，我是 Kodo。今天想处理什么工作？",
            AgentMode::Code => "你好，我是 Kodo。有什么代码问题需要我帮你处理？",
        },
        "谢谢" | "thanks" | "thank you" => match mode {
            AgentMode::Work => "不客气。有需要继续的工作，直接告诉我即可。",
            AgentMode::Code => "不客气。需要继续处理代码时，直接告诉我任务即可。",
        },
        _ => return None,
    };
    Some(answer.to_owned())
}

fn is_direct_conversation(message: &str) -> bool {
    direct_conversation_answer(message, AgentMode::Code).is_some()
}

/// True when the message looks like a read-only "summarize this project" /
/// "describe this codebase" / "output the layout" task. Such tasks need
/// 20-40+ tool calls to map a project, but never mutate it — so we route
/// them to `TaskPlan::summarize()` + `Budget::for_summary()` (128 tools /
/// 12 rounds / 0 repairs) instead of the code default (32 tools / 6 rounds).
///
/// Three-layer detection so false positives stay rare:
/// 1. **Mutating verb exclusion** — any of 修改/修复/创建/添加/重构/write/fix/
///    create/update/refactor/edit/delete → NOT summary intent, the user
///    wants the code budget.
/// 2. **Direct summary verb** — 总结/介绍/列出/说明/梳理/概览/概述/
///    summarize/overview/describe/introduce/explain/outline/走读 → summary.
/// 3. **Output + project noun** — "输出/给我/show me/give me/print" combined
///    with a project-shaped noun (项目/工程/代码/架构/仓库/代码库/目录/
///    project/codebase/repo/repository/architecture/layout/structure/folder)
///    → summary. Catches "为我输出该项目的总体架构设计".
fn is_summary_intent(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let mutating = [
        "write", "edit", "fix", "create", "update", "refactor", "delete",
        "修改", "修复", "创建", "更新", "添加", "重构", "删除",
    ];
    if mutating.iter().any(|k| lower.contains(k)) {
        return false;
    }
    let summary_verbs = [
        "总结", "介绍", "列出", "说明", "梳理", "概览", "概述",
        "summarize", "summarise", "summary", "overview", "describe",
        "introduce", "explain", "outline", "走读",
    ];
    if summary_verbs.iter().any(|k| lower.contains(k)) {
        return true;
    }
    let output_verbs = ["输出", "给我", "show me", "give me", "print", "printout"];
    let project_nouns = [
        "项目", "工程", "代码", "架构", "仓库", "代码库", "目录",
        "project", "codebase", "repo", "repository",
        "architecture", "layout", "structure", "folder",
    ];
    let has_output = output_verbs.iter().any(|k| lower.contains(k));
    let has_project = project_nouns.iter().any(|k| lower.contains(k));
    has_output && has_project
}

/// Knowledge/explanation questions are Q&A, not tasks (2026-09-29):
/// "介绍下什么是数据仓库" must be answered directly instead of inheriting
/// a feature skill's acceptance criteria. Conservative by design — an
/// explicit task verb always wins ("介绍下怎么修复" is a task), and a bare
/// "？" never flips the channel: a knowledge marker is required. The caller
/// additionally requires that no skill chip is pinned.
pub(crate) fn is_conversational_question(message: &str) -> bool {
    const KNOWLEDGE: &[&str] = &[
        "什么是",
        "是什么",
        "什么意思",
        "意思是什么",
        "什么叫",
        "指的是",
        "介绍下",
        "介绍一下",
        "介绍",
        "解释下",
        "解释一下",
        "解释",
        "讲讲",
        "讲一下",
        "讲一讲",
        "如何理解",
        "怎么理解",
        "原理",
        "区别",
        "差异",
        "含义",
        "分析",
        "梳理",
        "总结",
        "概览",
        "架构",
        "what is",
        "what's",
        "explain ",
        "difference between",
        "analyze ",
        "analyse ",
        "summarize",
    ];
    const TASK: &[&str] = &[
        "修复",
        "解决",
        "实现",
        "添加",
        "新增",
        "更新",
        "编写",
        "优化",
        "重构",
        "排查",
        "修改",
        "部署",
        "上线",
        "开发",
        "集成",
        "升级",
        "迁移",
        "删除",
        "移除",
        "安装",
        "调试",
        "合并",
        "写个",
        "写一个",
        "写一下",
        "帮我改",
        "改一下",
        "改下",
        "跑一下",
        "执行",
        "fix ",
        "implement",
        "refactor",
        "update ",
        "create ",
        "write a",
    ];
    let text = message.trim();
    if text.is_empty() {
        return false;
    }
    let lower = text.to_lowercase();
    if TASK.iter().any(|verb| lower.contains(verb)) {
        return false;
    }
    KNOWLEDGE.iter().any(|marker| lower.contains(marker))
}

/// Gates that must never let a plan reach Verify: no project root (nothing to
/// run against), and Work mode (DESIGN.md §11 — Work never runs the verify
/// pipeline, whatever the skill's verification_policy says; a user skill may
/// declare `full`, a chip may select a verify-bound skill).
fn clamp_plan_verification(plan: &mut TaskPlan, mode: AgentMode, has_project: bool) {
    if !has_project || mode == AgentMode::Work {
        plan.requires_verify = false;
    }
}

/// Strip leading `【技能：<id>】` chip lines (the Composer's existing payload
/// contract — `Composer.tsx` sends exactly this prefix). Only whole lines at
/// the top of the message count and ids must be `[A-Za-z0-9_-]{1,64}`; a
/// mid-line imitation or a malformed id is ordinary text and stays. Returns
/// the chip ids plus the remainder for `classify()` — the provider-visible
/// `request.message` is never rewritten.
fn split_skill_chip(message: &str) -> (Vec<String>, &str) {
    const OPEN: &str = "【技能：";
    const CLOSE: &str = "】";
    let mut ids: Vec<String> = Vec::new();
    let mut rest = message;
    while let Some(after_open) = rest.strip_prefix(OPEN) {
        let Some(close_at) = after_open.find(CLOSE) else {
            break;
        };
        let id = &after_open[..close_at];
        if id.is_empty()
            || id.len() > 64
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            break;
        }
        let after_close = &after_open[close_at + CLOSE.len()..];
        // The marker must occupy its whole line: newline, or end of message.
        let Some(next) = after_close.strip_prefix('\n') else {
            if after_close.is_empty() {
                rest = after_close;
                ids.push(id.to_owned());
            }
            break;
        };
        ids.push(id.to_owned());
        rest = next;
    }
    if ids.is_empty() {
        // Not a chip at all — classify sees the original message.
        return (ids, message);
    }
    (ids, rest)
}

/// First chip that names a registry skill **in the current space** wins
/// (DESIGN.md §11: work and code spaces never mix). A Work-mode chip must be
/// applicable to Work; a Code-mode chip must not be. Unknown or out-of-space
/// chips are ignored — the caller falls back to `select(task_type)`.
fn resolve_skill<'a>(
    registry: &'a SkillRegistry,
    chip_ids: &[String],
    mode: AgentMode,
) -> Option<&'a SkillSpec> {
    chip_ids.iter().find_map(|id| {
        let skill = registry.get(id)?;
        let in_space = match mode {
            AgentMode::Work => skill.applicable_task_types.contains(&TaskType::Work),
            AgentMode::Code => !skill.applicable_task_types.contains(&TaskType::Work),
        };
        in_space.then_some(skill)
    })
}

fn checks_from(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .take(5)
        .collect()
}

/// Remove XML-ish tool-protocol spans (`<tool_call>` / `<function=NAME` /
/// `<parameter=KEY`) even when the model packed several calls on one line. Returns
/// the prose around them; unterminated protocol debris is dropped with it.
fn strip_tag_protocol(text: &str) -> String {
    let pairs = [
        ("<tool_call>", "</tool_call>"),
        ("<function=", "</function>"),
        ("<parameter=", "</parameter>"),
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let next = pairs
            .iter()
            .filter_map(|(open, close)| rest.find(open).map(|pos| (pos, *open, *close)))
            .min_by_key(|(pos, _, _)| *pos);
        let Some((pos, open, close)) = next else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..pos]);
        match rest[pos + open.len()..].find(close) {
            Some(end) => {
                out.push('\n');
                rest = &rest[pos + open.len() + end + close.len()..];
            }
            // Unterminated debris — the remainder is all protocol, drop it.
            None => break,
        }
    }
    out
}

fn strip_tool_artifacts(text: &str) -> String {
    // Did the original carry anything this function hides? When everything
    // gets filtered the answer is protocol debris — never echo it back (that
    // is exactly how raw tool XML leaked into 本次完成).
    let had_hidden = text.contains("\"tool_calls\"")
        || text.contains("<tool_call>")
        || text.contains("</tool_call>")
        || text.contains("<function=")
        || text.contains("<parameter=")
        || text.contains("</function>")
        || text.contains("**工具协议**")
        || text.contains("**技能系统**")
        || text.contains("**工作原则**")
        || text.contains("**搜索策略**")
        || text.contains("**项目位置**")
        || text.contains("```bash")
        || text.contains("```write")
        || text.contains("```search")
        || text.contains("```read")
        || text.contains("```json")
        || text.contains("```run_command")
        || text.contains("```read_file")
        || text.contains("```write_file");
    let body = strip_tag_protocol(text);
    let mut out = String::new();
    let mut skip = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.contains("\"tool_calls\"")
            || trimmed.contains("<function=")
            || trimmed.contains("<parameter=")
            || trimmed.contains("</function>")
            || trimmed.starts_with("**工具协议**")
            || trimmed.starts_with("- **工具协议**")
            || trimmed.starts_with("**技能系统**")
            || trimmed.starts_with("- **技能系统**")
            || trimmed.starts_with("**工作原则**")
            || trimmed.starts_with("- **工作原则**")
            || trimmed.starts_with("**搜索策略**")
            || trimmed.starts_with("- **搜索策略**")
            || trimmed.starts_with("**项目位置**")
            || trimmed.starts_with("- **项目位置**")
        {
            continue;
        }
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
    if trimmed.is_empty() {
        if had_hidden {
            String::new()
        } else {
            text.trim().to_owned()
        }
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::PersonaExample;
    use crate::provider::{ContentBlock, MessageRole};

    #[test]
    fn history_chars_counts_text_blocks() {
        let history = vec![
            ProviderMessage::system("You are Kodo."),
            ProviderMessage::user("find the README"),
            ProviderMessage::assistant("I'll search.", vec![]),
        ];
        let chars = history_chars(&history);
        // "You are Kodo." (13) + "find the README" (15) + "I'll search." (12) = 40
        assert_eq!(chars, 40);
    }

    #[test]
    fn extended_thinking_appends_think_step_instruction() {
        let dir = std::env::temp_dir().join(format!("kodo-et-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut request = temp_request(&dir, Permission::Ask);
        request.extended_thinking = true;
        // Consumer is the system prompt assembly path in run(); assert the
        // flag drives the instruction the same way send_message wires it.
        assert!(request.extended_thinking);
        let mut system = String::from("base");
        if request.extended_thinking {
            system.push_str("\nThink step by step before answering.");
        }
        assert!(system.contains("Think step by step"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn max_output_tokens_is_forwarded_to_provider_clamp() {
        // main.rs reads max-output-tokens; provider clamps to [256, 8192].
        let clamped = 4096u32.clamp(256, 8192);
        assert_eq!(clamped, 4096);
        assert_eq!(1u32.clamp(256, 8192), 256);
        assert_eq!(99_999u32.clamp(256, 8192), 8192);
    }

    #[test]
    fn tool_schemas_map_externals_with_passthrough_input_schema() {
        // Native tool calling picks up MCP tools for free: parameters is the
        // server's inputSchema verbatim.
        let registry = registry().with_externals(vec![protocol::ExternalToolDef {
            name: "mcp__fake__echo".to_owned(),
            description: "Echo the input back".to_owned(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }),
        }]);
        let schemas = tool_schemas(&registry);
        let echo = schemas
            .iter()
            .find(|s| s.name == "mcp__fake__echo")
            .expect("external schema");
        assert_eq!(echo.description, "Echo the input back");
        assert_eq!(
            echo.parameters,
            serde_json::json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            })
        );
        // Builtins still map alongside.
        assert!(schemas.iter().any(|s| s.name == "read_file"));
    }

    #[test]
    fn direct_conversation_detection_skips_agent_workflow_for_identity_questions() {
        assert!(is_direct_conversation("你是谁"));
        assert!(is_direct_conversation("你好！"));
        assert!(is_direct_conversation("who are you?"));
        assert!(!is_direct_conversation("检查这个项目的路由并修复"));
    }

    #[test]
    fn split_skill_chip_consumes_only_leading_whole_line_markers() {
        // The Composer payload: chip line, then the real message.
        let (ids, rest) = split_skill_chip("【技能：bug-fix】\n修复这个问题");
        assert_eq!(ids, vec!["bug-fix".to_owned()]);
        assert_eq!(rest, "修复这个问题");

        // Multiple leading chips.
        let (ids, rest) = split_skill_chip("【技能：a1】\n【技能：b_2】\n正文");
        assert_eq!(ids, vec!["a1".to_owned(), "b_2".to_owned()]);
        assert_eq!(rest, "正文");

        // No chip → classify sees the original message unchanged.
        let (ids, rest) = split_skill_chip("修复 unknown-table 路由");
        assert!(ids.is_empty());
        assert_eq!(rest, "修复 unknown-table 路由");

        // Mid-line imitation is ordinary text.
        let (ids, rest) = split_skill_chip("请用【技能：bug-fix】的方式处理");
        assert!(ids.is_empty());
        assert_eq!(rest, "请用【技能：bug-fix】的方式处理");

        // Malformed ids (spaces, empty) are not chips; nothing is consumed.
        let (ids, rest) = split_skill_chip("【技能：bad id】\n正文");
        assert!(ids.is_empty());
        assert_eq!(rest, "【技能：bad id】\n正文");
        let (ids, rest) = split_skill_chip("【技能：】\n正文");
        assert!(ids.is_empty());
        assert_eq!(rest, "【技能：】\n正文");

        // Marker sharing its line with text is not a whole-line chip.
        let (ids, rest) = split_skill_chip("【技能：bug-fix】 修复");
        assert!(ids.is_empty());
        assert_eq!(rest, "【技能：bug-fix】 修复");

        // Chip as the entire message.
        let (ids, rest) = split_skill_chip("【技能：docs】");
        assert_eq!(ids, vec!["docs".to_owned()]);
        assert_eq!(rest, "");
    }

    const DIR_SKILL_MD: &str = "# Skill: {name}\n\
        ## description\n{desc}\n\
        ## applicable_task_types\n- work\n\
        ## context_strategy\nwork-focused\n\
        ## allowed_tools\n- read_file\n\
        ## verification_policy\nnone\n\
        ## workflow\n- s1 | read | Read the input\n\
        ## completion_criteria\n- Done\n\
        ## guidance\nG.\n";

    fn write_dir_skill(dir: &std::path::Path, name: &str, desc: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let md = DIR_SKILL_MD
            .replace("{name}", name)
            .replace("{desc}", desc);
        std::fs::write(dir.join(format!("{name}.md")), md).unwrap();
    }

    #[test]
    fn build_skill_registry_loads_project_dir_and_projects_shadow_user() {
        let base = std::env::temp_dir().join(format!("kodo-skreg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user_dir = base.join("user");
        let project = base.join("proj");
        let project_skills = project.join(".kodo").join("skills");

        write_dir_skill(&user_dir, "my-notes", "User copy.");
        write_dir_skill(&project_skills, "my-notes", "Project copy.");
        write_dir_skill(&project_skills, "team-checks", "Team skill.");
        // Builtin name in the project dir must never shadow an embedded skill.
        write_dir_skill(&project_skills, "work", "Hijack attempt.");

        let reg = build_skill_registry(Some(&user_dir), Some(&project));
        // Project shadows user on collision (first extras win get()).
        assert_eq!(reg.get("my-notes").unwrap().description, "Project copy.");
        // Project-only skill is chip-reachable.
        assert_eq!(reg.get("team-checks").unwrap().description, "Team skill.");
        // Builtin never shadowed; select() still lands on builtins.
        assert!(!reg.get("work").unwrap().description.is_empty());
        assert!(
            !reg.get("work").unwrap().description.contains("Hijack"),
            "builtin name must not be shadowable"
        );
        assert_eq!(reg.select(TaskType::Work).unwrap().name, "work");

        // No dirs → builtins only, hermetic.
        let bare = build_skill_registry(None, None);
        assert!(bare.get("my-notes").is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn skill_prompt_block_carries_description_and_guidance() {
        let sk = SkillRegistry::builtin()
            .get("bug-fix")
            .expect("builtin")
            .clone();
        let block = skill_prompt_block(&sk, TaskType::BugFix);
        assert!(block.contains("Active skill: bug-fix"));
        assert!(block.contains("task_type=bug-fix"));
        assert!(block.contains("Skill description: "));
        assert!(block.contains(&sk.description));
        assert!(block.contains("Skill guidance:\n"));
        assert!(block.contains(&sk.guidance));

        // Empty guidance omits the section entirely.
        let mut bare = sk.clone();
        bare.guidance = String::new();
        let block = skill_prompt_block(&bare, TaskType::BugFix);
        assert!(block.contains("Active skill: bug-fix"));
        assert!(!block.contains("Skill guidance"));

        // Empty description omits its line too (legacy on-disk shape).
        let mut bare = sk;
        bare.description = String::new();
        let block = skill_prompt_block(&bare, TaskType::BugFix);
        assert!(!block.contains("Skill description"));
    }

    #[test]
    fn persona_chain_block_carries_single_persona() {
        let persona = PersonaBlock {
            name: "reviewer".to_owned(),
            instructions: "Answer as a strict code reviewer.".to_owned(),
            defaults: Default::default(),
            variables: Vec::new(),
            examples: Vec::new(),
        };
        let block = persona_chain_block(&[persona], &std::collections::HashMap::new());
        assert!(block.contains("Active agent persona:"));
        assert!(block.contains("Answer as a strict code reviewer."));

        // Whitespace-only personas contribute nothing (no empty header).
        let blank = PersonaBlock {
            name: String::new(),
            instructions: "   \n  ".to_owned(),
            defaults: Default::default(),
            variables: Vec::new(),
            examples: Vec::new(),
        };
        let empty = persona_chain_block(&[blank], &std::collections::HashMap::new());
        assert!(empty.is_empty());
        assert!(!empty.contains("Active agent persona"));
    }

    #[test]
    fn persona_chain_block_concatenates_chain_in_order() {
        let a = PersonaBlock {
            name: "reviewer".to_owned(),
            instructions: "Review carefully.".to_owned(),
            defaults: Default::default(),
            variables: Vec::new(),
            examples: Vec::new(),
        };
        let b = PersonaBlock {
            name: "refactorer".to_owned(),
            instructions: "Then suggest cleanups.".to_owned(),
            defaults: Default::default(),
            variables: Vec::new(),
            examples: Vec::new(),
        };
        let block = persona_chain_block(&[a, b], &std::collections::HashMap::new());
        assert!(block.contains("Active agent personas (in order):"));
        assert!(block.contains("1. reviewer:"));
        assert!(block.contains("2. refactorer:"));
        // Order is preserved — reviewer first, refactorer second.
        let pos_r = block.find("reviewer").unwrap();
        let pos_f = block.find("refactorer").unwrap();
        assert!(pos_r < pos_f);
    }

    #[test]
    fn substitute_template_vars_replaces_declared_only() {
        let mut vars = std::collections::HashMap::new();
        vars.insert("project_path".to_owned(), "/tmp/proj".to_owned());
        vars.insert("branch".to_owned(), "feat/x".to_owned());
        let allowed = vec!["project_path".to_owned()];
        let body = "Review `{{project_path}}` on `{{branch}}`.";
        let out = substitute_template_vars(body, &vars, &allowed);
        assert_eq!(out, "Review `/tmp/proj` on `{{branch}}`.");
    }

    #[test]
    fn substitute_template_vars_preserves_unknown_variables() {
        let mut vars = std::collections::HashMap::new();
        vars.insert("project_path".to_owned(), "/tmp/proj".to_owned());
        let body = "Path: {{project_path}}, owner: {{owner}}.";
        let out = substitute_template_vars(body, &vars, &[]);
        // Empty allow-list = no substitution (variables are opt-in).
        assert_eq!(out, body);
    }

    #[test]
    fn examples_block_renders_each_pair() {
        let persona = PersonaBlock {
            name: "reviewer".to_owned(),
            instructions: String::new(),
            defaults: Default::default(),
            variables: Vec::new(),
            examples: vec![
                PersonaExample {
                    input: "评审这个 PR".to_owned(),
                    output: "风险 1：...".to_owned(),
                },
                PersonaExample {
                    input: "看这段代码".to_owned(),
                    output: "建议：...".to_owned(),
                },
            ],
        };
        let block = examples_block(&[persona]);
        assert!(block.contains("Few-shot examples from `reviewer`:"));
        assert!(block.contains("Example 1:"));
        assert!(block.contains("评审这个 PR"));
        assert!(block.contains("风险 1："));
        assert!(block.contains("Example 2:"));
    }

    #[test]
    fn apply_persona_model_override_swaps_model_when_provider_matches() {
        let mut provider = Provider::new("openai", "key", "https://api.example", "gpt-4o");
        let personas = vec![PersonaBlock {
            name: "reviewer".to_owned(),
            instructions: String::new(),
            defaults: crate::agents::PersonaDefaults {
                provider: Some("openai".to_owned()),
                model: Some("gpt-4o-mini".to_owned()),
                permission: None,
                skills: Vec::new(),
            },
            variables: Vec::new(),
            examples: Vec::new(),
        }];
        apply_persona_model_override(&mut provider, &personas);
        assert_eq!(provider.model.as_str(), "gpt-4o-mini");
        assert_eq!(provider.model_id.as_deref(), Some("gpt-4o-mini"));
    }

    #[test]
    fn apply_persona_model_override_skips_provider_mismatch() {
        // Persona wants Anthropic but the active provider is OpenAI — drop
        // the override so we never silently switch providers mid-run.
        let mut provider = Provider::new("openai", "key", "https://api.example", "gpt-4o");
        let personas = vec![PersonaBlock {
            name: "reviewer".to_owned(),
            instructions: String::new(),
            defaults: crate::agents::PersonaDefaults {
                provider: Some("anthropic".to_owned()),
                model: Some("claude-sonnet-4-5".to_owned()),
                permission: None,
                skills: Vec::new(),
            },
            variables: Vec::new(),
            examples: Vec::new(),
        }];
        apply_persona_model_override(&mut provider, &personas);
        assert_eq!(provider.model.as_str(), "gpt-4o");
    }

    #[test]
    fn apply_persona_model_override_last_in_chain_wins() {
        let mut provider = Provider::new("openai", "key", "https://api.example", "gpt-4o");
        let personas = vec![
            PersonaBlock {
                name: "first".to_owned(),
                instructions: String::new(),
                defaults: crate::agents::PersonaDefaults {
                    provider: None,
                    model: Some("gpt-4o-mini".to_owned()),
                    permission: None,
                    skills: Vec::new(),
                },
                variables: Vec::new(),
                examples: Vec::new(),
            },
            PersonaBlock {
                name: "second".to_owned(),
                instructions: String::new(),
                defaults: crate::agents::PersonaDefaults {
                    provider: None,
                    model: Some("gpt-4.1".to_owned()),
                    permission: None,
                    skills: Vec::new(),
                },
                variables: Vec::new(),
                examples: Vec::new(),
            },
        ];
        apply_persona_model_override(&mut provider, &personas);
        assert_eq!(provider.model.as_str(), "gpt-4.1");
    }

    #[test]
    fn resolve_skill_enforces_the_space_boundary() {
        let reg = SkillRegistry::builtin();
        let work_chip = vec!["workflow".to_owned()];
        let code_chip = vec!["bug-fix".to_owned()];

        // In-space chips resolve…
        assert_eq!(
            resolve_skill(&reg, &work_chip, AgentMode::Work).map(|s| s.name.as_str()),
            Some("workflow")
        );
        assert_eq!(
            resolve_skill(&reg, &code_chip, AgentMode::Code).map(|s| s.name.as_str()),
            Some("bug-fix")
        );
        // …out-of-space chips never do (DESIGN §11: spaces never mix), so the
        // caller falls back to select(task_type).
        assert!(resolve_skill(&reg, &code_chip, AgentMode::Work).is_none());
        assert!(resolve_skill(&reg, &work_chip, AgentMode::Code).is_none());
        // Unknown ids fall back too.
        assert!(resolve_skill(&reg, &["nope".to_owned()], AgentMode::Work).is_none());
        assert!(resolve_skill(&reg, &[], AgentMode::Code).is_none());
    }

    #[test]
    fn user_skill_is_chip_reachable_but_never_hijacks_select() {
        let user_md = "# Skill: my-notes\n\
            ## applicable_task_types\n- work\n\
            ## context_strategy\nwork-focused\n\
            ## allowed_tools\n- read_file\n\
            ## verification_policy\nnone\n\
            ## workflow\n- s1 | read | Read the input\n\
            ## completion_criteria\n- The notes are complete\n\
            ## guidance\nKeep it short.\n";
        let user = skill::parse_skill(user_md).expect("user spec");
        let reg = SkillRegistry::builtin().with_user(vec![user]);
        // Chip path: get() reaches user skills…
        let chip = vec!["my-notes".to_owned()];
        assert_eq!(
            resolve_skill(&reg, &chip, AgentMode::Work).map(|s| s.name.clone()),
            Some("my-notes".to_owned())
        );
        // …select() first-match still lands on the builtin `work`.
        assert_eq!(
            reg.select(TaskType::Work).map(|s| s.name.as_str()),
            Some("work")
        );
    }

    #[test]
    fn clamp_plan_verification_never_verifies_in_work_mode() {
        // A full-policy (code) skill's plan clamped for Work: Defense in
        // depth for DESIGN §11 — user skills may declare `full`.
        let bug_skill = SkillRegistry::builtin()
            .select(TaskType::BugFix)
            .expect("bug-fix")
            .clone();
        let mut plan = TaskPlan::from_skill(&bug_skill, "fix the panic");
        assert!(plan.requires_verify, "code mode keeps the fail-closed bar");
        clamp_plan_verification(&mut plan, AgentMode::Work, true);
        assert!(!plan.requires_verify, "work never verifies");
        // Code + project: unchanged. No project: off regardless of mode.
        let mut code_plan = TaskPlan::from_skill(&bug_skill, "fix the panic");
        clamp_plan_verification(&mut code_plan, AgentMode::Code, true);
        assert!(code_plan.requires_verify);
        clamp_plan_verification(&mut code_plan, AgentMode::Code, false);
        assert!(!code_plan.requires_verify);
    }

    use protocol::parse_fence_invocations;

    fn registry() -> ToolRegistry {
        ToolRegistry::standard()
    }

    fn temp_request(dir: &Path, permission: Permission) -> RunRequest {
        RunRequest {
            project: Some(dir.to_path_buf()),
            message: "test".to_owned(),
            pinned_context: Vec::new(),
            provider: None,
            permission,
            mode: AgentMode::Code,
            fallback_to_local: true,
            max_output_tokens: 512,
            extended_thinking: false,
            session_id: None,
            user_skills_dir: None,
            mcp_servers: Vec::new(),
            agent_instructions: None,
            personas: vec![],
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
        // Network + package install require approval even beyond Full catastrophic set.
        assert!(Permission::Ask.needs_approval(StepKind::Command, Some("npm install lodash")));
        assert!(Permission::Ask.needs_approval(StepKind::Command, Some("curl https://x.example")));
        assert!(Permission::Auto.needs_approval(StepKind::Command, Some("npm install lodash")));
        assert!(Permission::Auto.needs_approval(StepKind::Command, Some("curl https://x.example")));
        // Full keeps catastrophic + destructive git hard guard.
        assert!(Permission::Full.needs_approval(StepKind::Command, Some("rm -rf /")));
        assert!(Permission::Full.needs_approval(StepKind::Command, Some("git push --force")));
        assert!(Permission::Full.needs_approval(StepKind::Command, Some("sudo ls")));
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
    fn strip_tool_artifacts_hides_internal_protocol_blocks() {
        let text = "- **工具协议**：tool_calls\n- **技能系统**：feature\n\n正常回复";
        let stripped = strip_tool_artifacts(text);
        assert_eq!(stripped, "正常回复");
    }

    #[test]
    fn strip_tool_artifacts_hides_tag_tool_protocol() {
        // The captured mimo wire sample (protocol-only final) must surface as
        // empty — the caller falls back to the offline note, never the tags.
        let wire = concat!(
            "<tool_call><function=search><parameter=query>--kodo-canvas</parameter></function></tool_call>",
            "<tool_call><function=read_file><parameter=path>apps/desktop/src/styles/app.css</parameter>",
            "<parameter=limit>400</parameter><parameter=offset>100</parameter></function></tool_call>"
        );
        assert_eq!(strip_tool_artifacts(wire), "");

        let mixed = format!("{wire}\n评分：界面结构清晰，密度合格。");
        let stripped = strip_tool_artifacts(&mixed);
        assert!(stripped.contains("评分：界面结构清晰"));
        assert!(!stripped.contains("kodo-canvas"));
        assert!(!stripped.contains("<function="));
    }

    #[test]
    fn strip_tool_artifacts_drops_unterminated_tag_debris() {
        let text = "结论可用。\n<tool_call><function=search><parameter=query>never-closed";
        let stripped = strip_tool_artifacts(text);
        assert_eq!(stripped, "结论可用。");
    }

    #[test]
    fn offline_answer_only_no_provider_claims_missing_config() {
        let notes = vec!["钉住上下文 `/tmp/a.xlsx`".to_owned()];

        // Only the genuine no-provider branch may say "not configured".
        let no_provider = offline_answer("分析文档", &notes, OfflineReason::NoProvider);
        assert!(no_provider.contains("本轮未配置可用的模型服务"));
        assert!(no_provider.contains("配置 Settings → AI Provider 的 API Key"));

        // Called-but-empty: honest head, real detail, actionable closer —
        // and it must never claim the provider is missing (2026-09-30 bug:
        // a configured 82s mimo call that hit the output cap rendered the
        // no-provider copy).
        let no_answer = offline_answer(
            "分析文档",
            &notes,
            OfflineReason::NoAnswer("输出达到 max-output-tokens 上限被截断，未产出正文"),
        );
        assert!(!no_answer.contains("未配置可用的模型服务"));
        assert!(no_answer.contains("本轮模型已调用但未产出可用回答"));
        assert!(no_answer.contains("**原因**：输出达到 max-output-tokens 上限被截断"));
        assert!(no_answer.contains("最大输出 tokens"));
        assert!(no_answer.contains("- 钉住上下文 `/tmp/a.xlsx`"));

        let failed = offline_answer("分析文档", &notes, OfflineReason::CallFailed("401"));
        assert!(failed.contains("模型调用未成功"));
        assert!(failed.contains("**模型错误**：401"));
        assert!(!failed.contains("未配置可用的模型服务"));

        // Cancelled/failed runs prefix their own explanation; the shared
        // body must not blame provider config either.
        let aborted = offline_answer("分析文档", &notes, OfflineReason::Aborted);
        assert!(!aborted.contains("未配置可用的模型服务"));
        // The provider worked — an aborted run must not send the user to
        // Settings to paste an API key (2026-09-30).
        assert!(!aborted.contains("API Key"));
        assert!(aborted.contains("**已执行**"));
    }

    #[test]
    fn empty_answer_detail_reports_what_actually_happened() {
        assert_eq!(
            empty_answer_detail("length", true),
            "模型思考耗尽输出上限（finish_reason=length），未产出正文"
        );
        assert_eq!(
            empty_answer_detail("length", false),
            "输出达到 max-output-tokens 上限被截断，未产出正文"
        );
        assert_eq!(
            empty_answer_detail("stop", true),
            "模型输出全部为思考内容，未产出正文"
        );
        assert_eq!(
            empty_answer_detail("stop", false),
            "模型调用成功但未返回正文"
        );
    }

    #[test]
    fn undo_without_changeset_is_an_empty_report() {
        let dir = std::env::temp_dir().join(format!("kodo-undo-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_changeset(&dir, "missing-session").unwrap().is_none());
        let report = undo_session_changes(&dir, "missing-session").unwrap();
        assert!(report.restored.is_empty());
        assert!(report.conflicts.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn execute_tool_call_rejects_read_escape() {
        let dir = std::env::temp_dir().join(format!("kodo-obs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let call = ToolCall {
            id: ToolCallId::new("r1"),
            name: ToolName::ReadFile,
            args: ToolArgs::ReadFile {
                path: "../escape.txt".to_owned(),
            },
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
            &mut dedup::TurnDedup::new(),
        )
        .expect("execute");
        assert!(keep);
        let result = result.expect("observation");
        assert!(!result.ok);
        assert_eq!(result.id.as_str(), "r1");
        assert!(result.error.as_ref().unwrap().code == ToolErrorCode::PathEscape);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phase 0 second-pass — `ask_user` emits a SinkEvent::AskUser carrying
    /// the question and options, and returns a successful ToolResult so the
    /// agent loop keeps going (the UI answer comes through as the next user
    /// message).
    #[test]
    fn execute_tool_call_ask_user_emits_event_and_placeholder() {
        let dir = std::env::temp_dir().join(format!("kodo-ask-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let call = ToolCall {
            id: ToolCallId::new("ask1"),
            name: ToolName::AskUser,
            args: ToolArgs::AskUser {
                question: "用 Rust 还是 TS？".to_owned(),
                options: vec!["Rust".to_owned(), "TypeScript".to_owned()],
            },
        };
        let mut captured: Vec<SinkEvent> = Vec::new();
        let mut notes = Vec::new();
        let (keep, result) = execute_tool_call(
            &call,
            &dir,
            &request,
            &|| true,
            &|_, _| true,
            &mut |event| {
                captured.push(event);
                true
            },
            &mut notes,
            &mut dedup::TurnDedup::new(),
        )
        .expect("execute");
        assert!(keep, "ask_user must keep the turn alive");
        let result = result.expect("observation");
        assert!(result.ok, "ask_user placeholder must succeed");
        assert_eq!(result.name, "ask_user");
        assert!(
            result.input.contains("用 Rust 还是 TS？"),
            "input must carry the question: {}",
            result.input
        );
        let saw_ask_event = captured.iter().any(|e| match e {
            SinkEvent::AskUser { question, options } => {
                question == "用 Rust 还是 TS？" && options.len() == 2
            }
            _ => false,
        });
        assert!(saw_ask_event, "must emit SinkEvent::AskUser");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phase 0 second-pass — `ask_user` without options still works (no
    /// quick-pick chips, free-form answer expected).
    #[test]
    fn execute_tool_call_ask_user_without_options_succeeds() {
        let dir = std::env::temp_dir().join(format!("kodo-ask2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let call = ToolCall {
            id: ToolCallId::new("ask2"),
            name: ToolName::AskUser,
            args: ToolArgs::AskUser {
                question: "请补充输入文件路径".to_owned(),
                options: Vec::new(),
            },
        };
        let mut captured: Vec<SinkEvent> = Vec::new();
        let mut notes = Vec::new();
        let (keep, result) = execute_tool_call(
            &call,
            &dir,
            &request,
            &|| true,
            &|_, _| true,
            &mut |event| {
                captured.push(event);
                true
            },
            &mut notes,
            &mut dedup::TurnDedup::new(),
        )
        .expect("execute");
        assert!(keep);
        let result = result.expect("observation");
        assert!(result.ok);
        let saw = captured.iter().any(|e| matches!(
            e,
            SinkEvent::AskUser { options, .. } if options.is_empty()
        ));
        assert!(saw, "free-form question must still emit AskUser event");
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
            args: ToolArgs::ReadFile {
                path: "does-not-exist.txt".into(),
            },
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
            &mut dedup::TurnDedup::new(),
        )
        .expect("execute");
        assert!(keep, "execution failure must not abort the session");
        let result = result.expect("observation");
        assert!(!result.ok);
        assert_eq!(
            result.error.as_ref().unwrap().code,
            ToolErrorCode::ExecutionFailed
        );
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
            args: ToolArgs::RunCommand {
                command: "echo observation-loop".to_owned(),
            },
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
            &mut dedup::TurnDedup::new(),
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
            args: ToolArgs::WriteFile {
                path: "out.txt".into(),
                content: "x\n".into(),
            },
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
            &mut dedup::TurnDedup::new(),
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

    /// `.kodo/` is Kodo's own state: a deny-everything approver must not block
    /// it, and no FileChange step may reach the session log (the reply card
    /// and archive counts read those as "changed files").
    #[test]
    fn kodo_state_write_skips_approval_and_emits_no_file_change() {
        let dir = std::env::temp_dir().join(format!("kodo-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let call = ToolCall {
            id: ToolCallId::new("kw"),
            name: ToolName::WriteFile,
            args: ToolArgs::WriteFile {
                path: ".kodo/identity.md".into(),
                content: "who\n".into(),
            },
        };
        let mut notes = Vec::new();
        let mut steps = Vec::new();
        let (keep, result) = execute_tool_call(
            &call,
            &dir,
            &request,
            &|| true,
            &|_, _| false, // deny everything — state writes must bypass
            &mut |event| {
                steps.push(event);
                true
            },
            &mut notes,
            &mut dedup::TurnDedup::new(),
        )
        .expect("execute");
        assert!(keep);
        let result = result.expect("observation");
        assert!(result.ok, "error={:?}", result.error);
        assert!(dir.join(".kodo/identity.md").exists());
        assert_eq!(steps.len(), 0, "state writes must not emit steps");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `list_files` dedup: same prefix twice in one turn gets a `[cached]`
    /// suffix on the second response. Read between the calls resets the
    /// warning window.
    #[test]
    fn list_files_dedup_marks_repeats_in_same_turn() {
        let dir = std::env::temp_dir().join(format!(
            "kodo-list-dedup-{}-{}",
            std::process::id(),
            "list_files_dedup_marks_repeats_in_same_turn"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("crates")).unwrap();
        std::fs::write(dir.join("crates/a.rs"), "a").unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let mut dedup = dedup::TurnDedup::new();
        let mut notes = Vec::new();

        let call1 = ToolCall {
            id: ToolCallId::new("l1"),
            name: ToolName::ListFiles,
            args: ToolArgs::ListFiles {
                prefix: Some("crates/".into()),
            },
        };
        let (_, r1) = execute_tool_call(
            &call1,
            &dir,
            &request,
            &|| true,
            &|_, _| true,
            &mut |_| true,
            &mut notes,
            &mut dedup,
        )
        .expect("first list");
        let r1 = r1.expect("first obs");
        assert!(r1.ok);
        assert!(!r1.output.contains("[cached"), "first call must not self-mark");

        let call2 = ToolCall {
            id: ToolCallId::new("l2"),
            name: ToolName::ListFiles,
            args: ToolArgs::ListFiles {
                prefix: Some("crates/".into()),
            },
        };
        let (_, r2) = execute_tool_call(
            &call2,
            &dir,
            &request,
            &|| true,
            &|_, _| true,
            &mut |_| true,
            &mut notes,
            &mut dedup,
        )
        .expect("second list");
        let r2 = r2.expect("second obs");
        assert!(r2.ok);
        assert!(
            r2.output.contains("[cached · same prefix this turn]"),
            "second call must be marked cached: {r2:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Three same-prefix `list_files` calls in a row (no read between them)
    /// produce the warning hint on the third response.
    #[test]
    fn list_files_warns_after_three_consecutive_repeats() {
        let dir = std::env::temp_dir().join(format!(
            "kodo-list-warn-{}-{}",
            std::process::id(),
            "list_files_warns_after_three_consecutive_repeats"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("crates")).unwrap();
        std::fs::write(dir.join("crates/a.rs"), "a").unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let mut dedup = dedup::TurnDedup::new();
        let mut notes = Vec::new();

        for i in 0..3 {
            let call = ToolCall {
                id: ToolCallId::new(format!("w{i}")),
                name: ToolName::ListFiles,
                args: ToolArgs::ListFiles {
                    prefix: Some("crates/".into()),
                },
            };
            let (_, r) = execute_tool_call(
                &call,
                &dir,
                &request,
                &|| true,
                &|_, _| true,
                &mut |_| true,
                &mut notes,
                &mut dedup,
            )
            .expect("list");
            let r = r.expect("obs");
            assert!(r.ok);
            if i == 2 {
                assert!(
                    r.output.contains("[hint: you have listed this directory several times this turn"),
                    "third call must carry the hint: {r:?}"
                );
            } else {
                assert!(
                    !r.output.contains("[hint:"),
                    "calls 1 and 2 must not warn: {r:?}"
                );
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A read between repeated list calls resets the warning window.
    #[test]
    fn list_files_resets_warning_when_read_intervenes() {
        let dir = std::env::temp_dir().join(format!(
            "kodo-list-reset-{}-{}",
            std::process::id(),
            "list_files_resets_warning_when_read_intervenes"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("crates")).unwrap();
        std::fs::write(dir.join("crates/a.rs"), "hello\n").unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let mut dedup = dedup::TurnDedup::new();
        let mut notes = Vec::new();

        for i in 0..2 {
            let call = ToolCall {
                id: ToolCallId::new(format!("r{i}")),
                name: ToolName::ListFiles,
                args: ToolArgs::ListFiles {
                    prefix: Some("crates/".into()),
                },
            };
            let _ = execute_tool_call(
                &call,
                &dir,
                &request,
                &|| true,
                &|_, _| true,
                &mut |_| true,
                &mut notes,
                &mut dedup,
            )
            .expect("list");
        }

        // A read between lists settles the consecutive-repeats counter.
        let read = ToolCall {
            id: ToolCallId::new("rd"),
            name: ToolName::ReadFile,
            args: ToolArgs::ReadFile {
                path: "crates/a.rs".into(),
            },
        };
        let _ = execute_tool_call(
            &read,
            &dir,
            &request,
            &|| true,
            &|_, _| true,
            &mut |_| true,
            &mut notes,
            &mut dedup,
        )
        .expect("read");

        // After the read, one repeat is not enough to warn.
        let call = ToolCall {
            id: ToolCallId::new("r2"),
            name: ToolName::ListFiles,
            args: ToolArgs::ListFiles {
                prefix: Some("crates/".into()),
            },
        };
        let (_, r) = execute_tool_call(
            &call,
            &dir,
            &request,
            &|| true,
            &|_, _| true,
            &mut |_| true,
            &mut notes,
            &mut dedup,
        )
        .expect("third list");
        let r = r.expect("obs");
        assert!(
            !r.output.contains("[hint:"),
            "post-read single repeat must not warn: {r:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -- Phase C: auto-compaction integration tests -----------------------

    /// Build a fake-provider script that emits a fixed `text` reply (input
    /// tokens default to 0 — the compaction test injects the Budget
    /// directly, it does not rely on provider-reported usage).
    fn write_fake_summary_script(dir: &Path, body: &str) -> std::path::PathBuf {
        let script = dir.join("compaction_script.jsonl");
        std::fs::write(&script, format!("{}\n", serde_json::json!({ "text": body })))
            .expect("write fake script");
        script
    }

    fn build_history_for_compaction(n_middle: usize, n_tail: usize) -> Vec<ProviderMessage> {
        let mut history = vec![ProviderMessage::system("system prompt")];
        for i in 0..n_middle {
            history.push(ProviderMessage::user(format!("middle #{i}")));
        }
        for i in 0..n_tail {
            history.push(ProviderMessage::user(format!("tail #{i}")));
        }
        history
    }

    fn build_high_budget() -> Budget {
        let mut b = Budget::default();
        b.set_window(200_000);
        // 170k / 200k = 85% → above default 80% trigger.
        b.charge_tokens(170_000);
        b
    }

    /// Mid-turn compaction: summarize the middle, keep system + tail.
    /// Drives `compact_if_needed` directly so the integration hook is
    /// covered end-to-end (Budget state, history mutation, SinkEvent).
    #[test]
    fn compact_if_needed_replaces_middle_with_summary() {
        let dir = std::env::temp_dir().join(format!(
            "kodo-compact-pass-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = write_fake_summary_script(&dir, "Concise summary of earlier turns.");
        let provider = Provider::new(
            "fake",
            "fake-key",
            script.to_string_lossy(),
            "fake-model",
        );
        let mut history = build_history_for_compaction(10, 6);
        let original_len = history.len();
        let mut budget = build_high_budget();
        let mut captured: Vec<SinkEvent> = Vec::new();
        let mut emit = |event: SinkEvent| -> bool {
            captured.push(event);
            true
        };

        compact_if_needed(
            &mut history,
            &mut budget,
            &provider,
            &|| true,
            &mut emit,
        );

        assert_eq!(
            history.len(),
            1 /* system */ + 1 /* summary */ + 6 /* tail */
        );
        assert!(matches!(history[0].role, MessageRole::System));
        assert!(matches!(history[1].role, MessageRole::User));
        // The summary message holds the model's body verbatim.
        let summary_text = history[1]
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        assert!(summary_text.contains("Concise summary"), "{summary_text:?}");
        // The most recent 6 messages are kept verbatim and appear in order.
        for (i, expected) in (0..6).map(|i| format!("tail #{i}")).enumerate() {
            let actual = history[2 + i]
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            assert_eq!(actual, expected, "tail[{i}] mismatch");
        }
        // Budget counter advanced; Compacted event surfaced to the sink.
        assert_eq!(budget.compacted_count, 1);
        let compacted = captured
            .iter()
            .find_map(|e| match e {
                SinkEvent::Compacted { dropped, summarized_into, freed_pct } => {
                    Some((*dropped, *summarized_into, *freed_pct))
                }
                _ => None,
            })
            .expect("compacted event");
        assert_eq!(compacted.1, 1, "summarized_into must be 1");
        assert!(compacted.0 >= 4, "dropped should reflect middle size, got {}", compacted.0);
        assert!(compacted.2 <= 100);
        let _ = original_len;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// When the budget is under threshold, the hook is a no-op — no event,
    /// no history change, no count bump.
    #[test]
    fn compact_if_needed_skips_when_budget_low() {
        let dir = std::env::temp_dir().join(format!("kodo-compact-skip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = write_fake_summary_script(&dir, "should not be called");
        let provider = Provider::new(
            "fake",
            "fake-key",
            script.to_string_lossy(),
            "fake-model",
        );
        let mut history = build_history_for_compaction(10, 6);
        let original_history = history.clone();
        let mut budget = Budget::default();
        budget.set_window(200_000);
        budget.charge_tokens(50_000); // 25%
        let mut captured: Vec<SinkEvent> = Vec::new();
        let mut emit = |event: SinkEvent| -> bool {
            captured.push(event);
            true
        };
        compact_if_needed(
            &mut history,
            &mut budget,
            &provider,
            &|| true,
            &mut emit,
        );
        assert_eq!(history, original_history, "history untouched below threshold");
        assert_eq!(budget.compacted_count, 0);
        assert!(captured.is_empty(), "no event expected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// When the provider fails (here: empty script), compact_if_needed must
    /// not panic, must not mutate history, must not emit anything.
    #[test]
    fn compact_if_needed_falls_back_silently_on_provider_failure() {
        let dir = std::env::temp_dir().join(format!("kodo-compact-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Empty script → fake_text returns malformed-JSON error.
        let script = dir.join("empty.jsonl");
        std::fs::write(&script, "").unwrap();
        let p = Provider::new("fake", "fake-key", script.to_string_lossy(), "fake-model");
        let mut history = build_history_for_compaction(10, 6);
        let original_history = history.clone();
        let mut budget = build_high_budget();
        let mut captured: Vec<SinkEvent> = Vec::new();
        let mut emit = |event: SinkEvent| -> bool {
            captured.push(event);
            true
        };
        compact_if_needed(
            &mut history,
            &mut budget,
            &p,
            &|| true,
            &mut emit,
        );
        assert_eq!(history, original_history, "history untouched on provider error");
        assert_eq!(budget.compacted_count, 0, "count not bumped on failure");
        assert!(captured.is_empty(), "no Compacted event on failure");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A project-file write under Ask with a denying approver still fails —
    /// the bypass is exactly `.kodo/`, not all writes.
    #[test]
    fn kodo_state_bypass_is_path_scoped() {
        let dir = std::env::temp_dir().join(format!("kodo-state2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let call = ToolCall {
            id: ToolCallId::new("kw2"),
            name: ToolName::WriteFile,
            args: ToolArgs::WriteFile {
                path: "src/out.rs".into(),
                content: "x\n".into(),
            },
        };
        let mut notes = Vec::new();
        let (_keep, result) = execute_tool_call(
            &call,
            &dir,
            &request,
            &|| true,
            &|_, _| false,
            &mut |event| {
                let _ = event;
                true
            },
            &mut notes,
            &mut dedup::TurnDedup::new(),
        )
        .expect("execute");
        let result = result.expect("observation");
        assert!(!result.ok);
        assert_eq!(
            result.error.as_ref().unwrap().code,
            ToolErrorCode::PermissionDenied
        );
        assert!(!dir.join("src/out.rs").exists());
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
        let ModelTurn::Tools { calls } = invocations else {
            panic!("expected tools")
        };
        let mut notes = Vec::new();
        let mut wrote = false;
        let results = run_invocations(
            calls,
            &dir,
            &request,
            None,
            &mut McpClients::default(),
            &|| true,
            &|_, _| true,
            &mut |event| {
                let _ = event;
                true
            },
            &mut notes,
            &mut wrote,
            &mut TurnChangeSet::default(),
            &mut dedup::TurnDedup::new(),
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
    fn external_tool_needs_approval_in_ask_auto_but_not_full() {
        // External risk is opaque (arbitrary server tool) — risk ≥ Network,
        // so Ask and Auto both require a human; only Full lets them through.
        // The detail never changes the decision: there is no risk classifier
        // for wire calls.
        assert!(
            Permission::Ask.needs_approval(StepKind::ExternalTool, Some("mcp__fake__echo {}"))
        );
        assert!(
            Permission::Auto.needs_approval(StepKind::ExternalTool, Some("mcp__fake__echo {}"))
        );
        assert!(!Permission::Full.needs_approval(StepKind::ExternalTool, Some("mcp__fake__echo {}")));
        assert!(Permission::Ask.needs_approval(StepKind::ExternalTool, None));
        assert!(Permission::Auto.needs_approval(StepKind::ExternalTool, None));
        assert!(!Permission::Full.needs_approval(StepKind::ExternalTool, None));
    }

    #[test]
    fn external_call_denial_returns_permission_denied_and_denied_step() {
        let dir = std::env::temp_dir().join(format!("kodo-ext-deny-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let invocations = vec![ToolInvocation::External(ExternalCall {
            id: ToolCallId::new("ext1"),
            name: "mcp__fake__echo".to_owned(),
            args: serde_json::json!({"text": "hi"}),
        })];
        let mut notes = Vec::new();
        let mut events: Vec<String> = Vec::new();
        let results = run_invocations(
            invocations,
            &dir,
            &request,
            None,
            &mut McpClients::default(),
            &|| true,
            &|kind, detail| {
                // Approval sees the ExternalTool kind and the wire-named detail.
                assert_eq!(kind, StepKind::ExternalTool);
                assert!(detail.contains("mcp__fake__echo"), "detail: {detail}");
                false // deny
            },
            &mut |event| {
                if let SinkEvent::Finished { step, denied, .. } = &event {
                    events.push(format!("{}:{denied}", step.preview_detail()));
                }
                true
            },
            &mut notes,
            &mut false,
            &mut TurnChangeSet::default(),
            &mut dedup::TurnDedup::new(),
        )
        .expect("run")
        .expect("results");
        assert_eq!(results.len(), 1);
        assert!(!results[0].ok);
        assert_eq!(results[0].name, "mcp__fake__echo");
        assert_eq!(
            results[0].error.as_ref().unwrap().code,
            ToolErrorCode::PermissionDenied
        );
        assert!(
            notes.iter().any(|n| n.contains("用户拒绝了")),
            "notes: {notes:?}"
        );
        // Trace shape: one finished Step::Command with the wire name in the
        // label, flagged denied — no new Step variant for external work.
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert!(events[0].starts_with("mcp__fake__echo "), "events: {events:?}");
        assert!(events[0].ends_with(":true"), "events: {events:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn external_call_traces_as_command_step_and_fills_the_result() {
        // Handshaken fake server whose `echo` answers "pong".
        let transport = mcp::FakeTransport::new()
            .reply("initialize", serde_json::json!({}))
            .reply(
                "tools/list",
                serde_json::json!({
                    "tools": [{"name": "echo", "description": "echo", "inputSchema": {"type": "object"}}]
                }),
            )
            .reply(
                "tools/call",
                serde_json::json!({"content": [{"type": "text", "text": "pong"}]}),
            );
        let client =
            mcp::McpClient::handshake("fake", "Fake", Box::new(transport)).expect("handshake");
        let mut mcp = mcp::McpClients::from_clients(vec![client], Vec::new());

        let dir = std::env::temp_dir().join(format!("kodo-ext-ok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let invocations = vec![ToolInvocation::External(ExternalCall {
            id: ToolCallId::new("ext1"),
            name: "mcp__fake__echo".to_owned(),
            args: serde_json::json!({"text": "hi"}),
        })];
        let mut notes = Vec::new();
        let mut events: Vec<String> = Vec::new();
        let results = run_invocations(
            invocations,
            &dir,
            &request,
            None,
            &mut mcp,
            &|| true,
            &|_, _| true,
            &mut |event| {
                match &event {
                    SinkEvent::Started { step } => events.push(format!("start:{}", step.preview_detail())),
                    SinkEvent::Finished { step, denied, .. } => {
                        events.push(format!("finish:{}:{denied}", step.preview_detail()))
                    }
                    _ => {}
                }
                true
            },
            &mut notes,
            &mut false,
            &mut TurnChangeSet::default(),
            &mut dedup::TurnDedup::new(),
        )
        .expect("run")
        .expect("results");
        // Dispatch reached the server and the observation came back.
        assert_eq!(results.len(), 1);
        assert!(results[0].ok, "{:?}", results[0]);
        assert_eq!(results[0].output, "pong");
        // Trace shape: Started then Finished, both Step::Command carrying the
        // wire name in the label.
        assert_eq!(
            events,
            vec![
                "start:mcp__fake__echo {\"text\":\"hi\"}".to_owned(),
                "finish:mcp__fake__echo {\"text\":\"hi\"}:false".to_owned(),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_mcp_config_is_zero_connect_and_seals_wire_names() {
        // 空配置零连接 + 密闭性: nothing configured → nothing connects, and a
        // model-invented wire name cannot resolve — an unknown-tool rejection,
        // never an External dispatch.
        let mcp = McpClients::connect(&[]);
        assert!(mcp.is_empty());
        assert!(mcp.notes.is_empty());
        assert!(mcp.tools().is_empty());
        assert!(external_defs(&mcp).is_empty());

        let registry = registry().with_externals(external_defs(&mcp));
        let ModelTurn::Tools { calls } = parse_model_turn(
            r#"{"tool_calls":[{"id":"1","name":"mcp__fake__echo","arguments":{}}]}"#,
            &registry,
        ) else {
            panic!("expected a tool call");
        };
        assert!(
            matches!(&calls[0], ToolInvocation::Rejected(_)),
            "sealed registry rejects invented wire names: {:?}",
            calls[0]
        );
    }

    #[test]
    fn external_call_streams_from_model_text_through_ask_to_backfill() {
        // B8 流式场景: the model *emits* `mcp__fake__echo` (JSON tool protocol),
        // the name resolves to External through the same registry assembly
        // run() uses, Ask gates it as ExternalTool, the call dispatches to the
        // fake server, and "pong" is backfilled as the observation. B7 covered
        // hand-built ExternalCall inputs; this adds the model-turn parse leg.
        let transport = mcp::FakeTransport::new()
            .reply("initialize", serde_json::json!({}))
            .reply(
                "tools/list",
                serde_json::json!({
                    "tools": [{"name": "echo", "description": "echo back", "inputSchema": {"type": "object"}}]
                }),
            )
            .reply(
                "tools/call",
                serde_json::json!({"content": [{"type": "text", "text": "pong"}]}),
            );
        let log = transport.log.clone(); // clone before the transport is boxed
        let client =
            mcp::McpClient::handshake("fake", "Fake", Box::new(transport)).expect("handshake");
        let mut mcp = mcp::McpClients::from_clients(vec![client], Vec::new());

        let registry = registry().with_externals(external_defs(&mcp));
        let ModelTurn::Tools { calls } = parse_model_turn(
            r#"{"tool_calls":[{"id":"1","name":"mcp__fake__echo","arguments":{"text":"hi"}}]}"#,
            &registry,
        ) else {
            panic!("expected a tool call");
        };
        assert!(
            matches!(&calls[0], ToolInvocation::External(_)),
            "wire name resolves to External: {:?}",
            calls[0]
        );

        let dir = std::env::temp_dir().join(format!("kodo-ext-stream-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let request = temp_request(&dir, Permission::Ask);
        let approvals = std::cell::Cell::new(0);
        let mut notes = Vec::new();
        let results = run_invocations(
            calls,
            &dir,
            &request,
            None,
            &mut mcp,
            &|| true,
            &|kind, detail| {
                // The Ask rendezvous sees the wire-named external call.
                assert_eq!(kind, StepKind::ExternalTool);
                assert!(detail.contains("mcp__fake__echo"), "detail: {detail}");
                approvals.set(approvals.get() + 1);
                true // allow
            },
            &mut |_| true,
            &mut notes,
            &mut false,
            &mut TurnChangeSet::default(),
            &mut dedup::TurnDedup::new(),
        )
        .expect("run")
        .expect("results");

        assert_eq!(approvals.get(), 1, "exactly one Ask rendezvous");
        // Backfill: pong rides the result and the observation block the model
        // sees next.
        assert_eq!(results.len(), 1);
        assert!(results[0].ok, "{:?}", results[0]);
        assert_eq!(results[0].output, "pong");
        let observations = format_observations(&results);
        assert!(observations.contains("pong"), "backfill: {observations}");
        // Dispatch reached the fake server exactly once via tools/call.
        let log = log.lock().expect("log");
        assert_eq!(
            log.iter().filter(|method| *method == "tools/call").count(),
            1,
            "log: {log:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn run_connects_configured_servers_and_dispatches_wire_calls() {
        // B8 end-to-end through `run()` itself: `RunRequest.mcp_servers` →
        // connect → registry externals → the model's `mcp__fake__echo` → Ask
        // approval → stdio dispatch → "pong" back in the observation → prose
        // closes the conversational turn.
        let dir = std::env::temp_dir().join(format!("kodo-ext-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let project = dir.join("repo");
        std::fs::create_dir_all(&project).expect("temp project");
        std::fs::write(project.join("README.md"), "# Demo\n").expect("readme");
        // Turn 1: the model emits the wire-named call (JSON tool protocol).
        // Turn 2: prose that ends the conversational run.
        let script = dir.join("script.jsonl");
        let tool_line = serde_json::json!({
            "text": "{\"tool_calls\":[{\"id\":\"1\",\"name\":\"mcp__fake__echo\",\"arguments\":{\"text\":\"hi\"}}]}"
        });
        let prose_line = serde_json::json!({ "text": "查询完成。" });
        std::fs::write(&script, format!("{tool_line}\n{prose_line}\n")).expect("script");
        let provider = Provider::new("fake", "fake-key", script.to_string_lossy(), "fake-model");
        let request = RunRequest {
            project: Some(project),
            message: "介绍下什么是数据仓库".to_owned(),
            pinned_context: Vec::new(),
            provider: Some(provider),
            permission: Permission::Ask,
            mode: AgentMode::Code,
            fallback_to_local: true,
            max_output_tokens: 512,
            extended_thinking: false,
            session_id: None,
            user_skills_dir: None,
            mcp_servers: vec![mcp::McpServerConfig {
                id: "fake".to_owned(),
                name: "Fake".to_owned(),
                enabled: true,
                transport: mcp::McpTransportKind::Stdio,
                command: "/bin/sh".to_owned(),
                args: vec!["-c".to_owned(), mcp::stdio::CANNED_SH_RESPONDER.to_owned()],
                env: Default::default(),
                url: String::new(),
                headers: Default::default(),
            }],
            agent_instructions: None,
            personas: vec![],
        };
        let approvals = std::cell::Cell::new(0);
        let mut command_outputs: Vec<(String, String)> = Vec::new();
        let mut answers: Vec<String> = Vec::new();
        let mut emit = |event: SinkEvent| {
            if let SinkEvent::Finished {
                step: Step::Command { command, output, .. },
                ..
            } = &event
            {
                command_outputs.push((command.clone(), output.clone()));
            }
            if let SinkEvent::Started {
                step: Step::AgentMessage { text, .. },
            } = &event
            {
                answers.push(text.clone());
            }
            true
        };
        run(
            &request,
            &|| true,
            &|kind, detail| {
                assert_eq!(kind, StepKind::ExternalTool);
                assert!(detail.contains("mcp__fake__echo"), "detail: {detail}");
                approvals.set(approvals.get() + 1);
                true // allow
            },
            &mut emit,
        )
        .expect("run with a configured server");

        assert_eq!(approvals.get(), 1, "exactly one Ask rendezvous");
        // The finished Step::Command trace carries the wire name and pong.
        assert!(
            command_outputs
                .iter()
                .any(|(command, output)| command.starts_with("mcp__fake__echo")
                    && output.contains("pong")),
            "trace: {command_outputs:?}"
        );
        // The turn closed with the scripted prose.
        assert_eq!(answers, vec!["查询完成。".to_owned()]);
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
            None,
            &mut McpClients::default(),
            &|| true,
            &|_, _| true,
            &mut |event| {
                let _ = event;
                true
            },
            &mut notes,
            &mut wrote,
            &mut TurnChangeSet::default(),
            &mut dedup::TurnDedup::new(),
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
        assert!(
            matches!(&inv[1], ToolInvocation::Rejected(r) if r.error.code == ToolErrorCode::UnknownTool)
        );
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
        assert!(keys
            .iter()
            .any(|k| k.contains("k2k") || k.contains("unknown")));
    }

    #[test]
    fn context_query_dedupes_when_goal_echoes_the_message() {
        // The plan goal is usually the message's first line; every token must
        // appear once, not twice (the 上下文检索 note used to print 4 segments
        // for a 2-segment ask).
        let message = "为我分析下k2k-rust的代码，为我输出总体架构设计";
        let query = context_query_for(message, message);
        let tokens = keywords_from(message);
        assert!(!tokens.is_empty());
        for token in &tokens {
            let hits = query.matches(token.as_str()).count();
            assert_eq!(hits, 1, "`{token}` appears {hits} times in `{query}`");
        }
        assert_eq!(context_query_for("", ""), "src");
    }

    #[test]
    fn deprecated_fence_parser_still_available_via_protocol() {
        // Compatibility layer: fences still produce multiple typed calls.
        let calls =
            parse_fence_invocations("```bash\necho a\n```\n```bash\necho b\n```", &registry());
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
        assert!(
            matches!(&calls[0], ToolInvocation::Rejected(r) if r.error.code == ToolErrorCode::InvalidArguments)
        );

        // D: unknown tool
        let ModelTurn::Tools { calls } = parse_model_turn(
            r#"{"tool_calls":[{"id":"d","name":"launch_missiles","arguments":{}}]}"#,
            &registry(),
        ) else {
            panic!("tools")
        };
        assert!(
            matches!(&calls[0], ToolInvocation::Rejected(r) if r.error.code == ToolErrorCode::UnknownTool)
        );
    }

    #[test]
    fn system_prompt_varies_by_mode_and_project() {
        let reg = ToolRegistry::default();
        let dir = std::path::Path::new("/tmp/project-x");

        let code = system_prompt(Some(dir), AgentMode::Code, &reg, "");
        assert!(code.contains("coding agent working in /tmp/project-x"));
        assert!(code.contains("Paths must stay inside the project"));

        let work = system_prompt(Some(dir), AgentMode::Work, &reg, "");
        assert!(work.contains("work assistant working in /tmp/project-x"));
        assert!(work.contains("everyday writing"));
        assert!(!work.contains("run project tests"));

        let code_bare = system_prompt(None, AgentMode::Code, &reg, "");
        assert!(code_bare.contains("No project folder is selected"));
        assert!(!code_bare.contains("Paths must stay inside"));

        let work_bare = system_prompt(None, AgentMode::Work, &reg, "");
        assert!(work_bare.contains("No folder is selected"));
    }

    /// Phase 0 second-pass — when the MCP clients surface a non-empty
    /// `status_hint()`, the system prompt must include it (so the model
    /// sees MCP gaps up-front instead of echoing them into "已执行").
    #[test]
    fn system_prompt_includes_mcp_hint_when_provided() {
        let reg = ToolRegistry::default();
        let dir = std::path::Path::new("/tmp/project-x");
        let hint = "MCP 服务器「filesystem」配置仍是模板占位值 (/path/to/allowed)";
        let code = system_prompt(Some(dir), AgentMode::Code, &reg, hint);
        assert!(code.contains("MCP 状态"));
        assert!(code.contains("filesystem"));
        // Empty hint → no MCP block.
        let bare = system_prompt(Some(dir), AgentMode::Code, &reg, "");
        assert!(!bare.contains("MCP 状态"));
    }

    #[test]
    fn direct_conversation_answers_vary_by_mode() {
        let code = direct_conversation_answer("你是谁", AgentMode::Code).unwrap();
        assert!(code.contains("编程助手"));
        let work = direct_conversation_answer("你是谁", AgentMode::Work).unwrap();
        assert!(work.contains("工作助手"));
        assert!(direct_conversation_answer("写个函数", AgentMode::Code).is_none());
    }

    #[test]
    fn conversational_question_detects_knowledge_questions() {
        assert!(is_conversational_question("介绍下什么是数据仓库"));
        assert!(is_conversational_question("什么是流式湖仓"));
        assert!(is_conversational_question("数据仓库和数据湖有什么区别？"));
        assert!(is_conversational_question("解释一下 Kafka 的 ISR 原理"));
        assert!(is_conversational_question("what is a data warehouse?"));
        // Analysis-shaped asks are read-only too (2026-09-30: the same
        // question five times burned every budget under the feature skill).
        assert!(is_conversational_question(
            "为我分析下k2k-rust的代码，为我输出总体架构设计"
        ));
        assert!(is_conversational_question("帮我梳理一下实时链路的模块关系"));
        assert!(is_conversational_question("总结下这次设计的取舍"));
        assert!(is_conversational_question("analyze the router module"));
    }

    #[test]
    fn conversational_question_yields_to_tasks_and_bare_interrogatives() {
        assert!(!is_conversational_question("修复登录超时的问题"));
        // A knowledge marker plus an explicit task verb stays a task.
        assert!(!is_conversational_question("介绍下怎么修复这个 bug"));
        assert!(!is_conversational_question("实现一个内存缓存"));
        assert!(!is_conversational_question("部署到预发环境"));
        assert!(!is_conversational_question("分析这个报错并修复它"));
        assert!(!is_conversational_question("analyze the log and fix it"));
        // A bare "？" without a knowledge marker never flips the channel.
        assert!(!is_conversational_question("这个测试挂了？"));
        assert!(!is_conversational_question(""));
    }

    #[test]
    fn run_without_project_emits_no_command_or_file_steps() {
        let request = RunRequest {
            project: None,
            message: "帮我写一份周报总结".to_owned(),
            pinned_context: Vec::new(),
            provider: None,
            permission: Permission::Ask,
            mode: AgentMode::Work,
            fallback_to_local: true,
            max_output_tokens: 512,
            extended_thinking: false,
            session_id: None,
            user_skills_dir: None,
            mcp_servers: Vec::new(),
            agent_instructions: None,
            personas: vec![],
        };
        let mut steps: Vec<String> = Vec::new();
        let mut emit = |event: SinkEvent| {
            if let SinkEvent::Started { step } = event {
                steps.push(format!("{:?}", step.kind()));
            }
            true
        };
        run(&request, &|| true, &|_, _| true, &mut emit).expect("run");
        assert!(
            steps.iter().all(|k| k != "Command" && k != "FileChange"),
            "project-less run must not execute commands or writes: {steps:?}"
        );
    }

    #[test]
    fn run_allows_code_mode_without_a_project_as_constrained_qa() {
        // Projectless Code is Q&A with the constrained response the matrix
        // guarantees (DESIGN.md §11): the run completes, but no command or
        // file-write step may ever fire — the registry is empty without a root.
        let request = RunRequest {
            project: None,
            message: "修复未知表路由".to_owned(),
            pinned_context: Vec::new(),
            provider: None,
            permission: Permission::Ask,
            mode: AgentMode::Code,
            fallback_to_local: true,
            max_output_tokens: 512,
            extended_thinking: false,
            session_id: None,
            user_skills_dir: None,
            mcp_servers: Vec::new(),
            agent_instructions: None,
            personas: vec![],
        };
        let mut steps: Vec<String> = Vec::new();
        let mut emit = |event: SinkEvent| {
            if let SinkEvent::Started { step } = event {
                steps.push(format!("{:?}", step.kind()));
            }
            true
        };
        run(&request, &|| true, &|_, _| true, &mut emit).expect("projectless code run");
        assert!(
            steps.iter().all(|k| k != "Command" && k != "FileChange"),
            "projectless code Q&A must not execute commands or writes: {steps:?}"
        );
    }

    #[test]
    fn projectless_answers_land_as_plain_qa_without_status_footer() {
        // The conversational channel delivers ready + not_run with the plain
        // flag on: the UI renders the sentence alone, the outcome reads
        // completed, and no "**Verification status:**" footer is appended —
        // none of the report chrome that used to frame a knowledge answer.
        let request = RunRequest {
            project: None,
            message: "介绍下什么是数据仓库".to_owned(),
            pinned_context: Vec::new(),
            provider: None,
            permission: Permission::Ask,
            mode: AgentMode::Code,
            fallback_to_local: true,
            max_output_tokens: 512,
            extended_thinking: false,
            session_id: None,
            user_skills_dir: None,
            mcp_servers: Vec::new(),
            agent_instructions: None,
            personas: vec![],
        };
        let mut messages: Vec<(String, String, String, bool)> = Vec::new();
        let mut emit = |event: SinkEvent| {
            if let SinkEvent::Started {
                step:
                    Step::AgentMessage {
                        text,
                        delivery,
                        verification,
                        plain,
                        ..
                    },
            } = event
            {
                messages.push((text, delivery, verification, plain));
            }
            true
        };
        run(&request, &|| true, &|_, _| true, &mut emit).expect("projectless qa run");
        assert_eq!(messages.len(), 1, "exactly one final answer");
        let (text, delivery, verification, plain) = &messages[0];
        assert!(*plain, "projectless answers render plain");
        assert_eq!(delivery, "ready");
        assert_eq!(verification, "not_run");
        assert!(
            !text.contains("Verification status"),
            "no task footer on a Q&A answer: {text}"
        );
    }

    /// Absolute-path temp attachment kept alive by the caller's dir handle.
    fn temp_attachment(name: &str, body: &[u8]) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!(
            "kodo-attach-{}-{}",
            std::process::id(),
            name.replace('.', "_")
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(name);
        std::fs::write(&path, body).expect("attachment body");
        (dir, path.to_string_lossy().to_string())
    }

    #[test]
    fn projectless_qa_inlines_pinned_attachment_and_stays_plain() {
        // DESIGN §11: a projectless answer comes from the request and its
        // (absolute-path) attachments alone — so the attachment must become a
        // pre-observation on the conversational arm, where the task-mode
        // collect block never runs. The analyze progress carries
        // `pre_observations.len()` for every arm.
        let (_dir, abs) = temp_attachment(
            "base_pay.txt",
            "缴费基数按实际工资申报，上下限按社平工资调整。\n".as_bytes(),
        );
        let request = RunRequest {
            project: None,
            message: "介绍下什么是缴费基数".to_owned(),
            pinned_context: vec![abs],
            provider: None,
            permission: Permission::Ask,
            mode: AgentMode::Code,
            fallback_to_local: true,
            max_output_tokens: 512,
            extended_thinking: false,
            session_id: None,
            user_skills_dir: None,
            mcp_servers: Vec::new(),
            agent_instructions: None,
            personas: vec![],
        };
        let mut progress: Vec<String> = Vec::new();
        let mut messages: Vec<(String, bool)> = Vec::new();
        let mut emit = |event: SinkEvent| {
            if let SinkEvent::Progress { phase, detail } = &event {
                if phase == "analyze" {
                    progress.push(detail.clone());
                }
            }
            if let SinkEvent::Started {
                step:
                    Step::AgentMessage {
                        text, plain, ..
                    },
            } = event
            {
                messages.push((text, plain));
            }
            true
        };
        run(&request, &|| true, &|_, _| true, &mut emit).expect("projectless qa run");
        assert!(
            progress.iter().any(|d| d.contains("1 context spans")),
            "attachment must reach the first user message: {progress:?}"
        );
        let (text, plain) = &messages[0];
        assert!(*plain, "attachment Q&A stays plain");
        assert!(
            !text.contains("Verification status"),
            "no task footer: {text}"
        );
    }

    #[test]
    fn greeting_with_attachment_skips_the_canned_reply() {
        // The canned greeting shortcut runs before pinned context is read, so
        // a greeting + file used to drop the attachment entirely. With files
        // pinned the run must reach the model (here: the offline fallback,
        // which echoes the pin note) instead of the local canned sentence.
        let (_dir, abs) = temp_attachment("deploy_note.txt", "发布步骤：先灰度再全量。\n".as_bytes());
        let request = RunRequest {
            project: None,
            message: "你好".to_owned(),
            pinned_context: vec![abs],
            provider: None,
            permission: Permission::Ask,
            mode: AgentMode::Code,
            fallback_to_local: true,
            max_output_tokens: 512,
            extended_thinking: false,
            session_id: None,
            user_skills_dir: None,
            mcp_servers: Vec::new(),
            agent_instructions: None,
            personas: vec![],
        };
        let mut texts: Vec<String> = Vec::new();
        let mut emit = |event: SinkEvent| {
            if let SinkEvent::Started {
                step: Step::AgentMessage { text, .. },
            } = event
            {
                texts.push(text);
            }
            true
        };
        run(&request, &|| true, &|_, _| true, &mut emit).expect("greeting + attachment run");
        assert_eq!(texts.len(), 1, "exactly one final answer");
        assert!(
            !texts[0].contains("我是 Kodo"),
            "canned greeting must yield to the attachment: {}",
            texts[0]
        );
        assert!(
            texts[0].contains("钉住上下文"),
            "the pinned attachment must surface: {}",
            texts[0]
        );
    }

    #[test]
    fn greeting_without_attachment_keeps_the_canned_reply() {
        // Control for the guard above: no files pinned → instant canned
        // answer, no model/offline round trip.
        let request = RunRequest {
            project: None,
            message: "你好".to_owned(),
            pinned_context: Vec::new(),
            provider: None,
            permission: Permission::Ask,
            mode: AgentMode::Code,
            fallback_to_local: true,
            max_output_tokens: 512,
            extended_thinking: false,
            session_id: None,
            user_skills_dir: None,
            mcp_servers: Vec::new(),
            agent_instructions: None,
            personas: vec![],
        };
        let mut texts: Vec<String> = Vec::new();
        let mut emit = |event: SinkEvent| {
            if let SinkEvent::Started {
                step: Step::AgentMessage { text, .. },
            } = event
            {
                texts.push(text);
            }
            true
        };
        run(&request, &|| true, &|_, _| true, &mut emit).expect("canned greeting run");
        assert_eq!(texts, vec!["你好，我是 Kodo。有什么代码问题需要我帮你处理？".to_owned()]);
    }

    #[test]
    fn binary_attachment_produces_a_descriptor_observation() {
        // png → binary refused → the exists arm emits an `<attachment
        // inlined="false">` descriptor so the model still knows the file is
        // pinned; `notes` additionally reach the offline answer.
        let (_dir, abs) = temp_attachment("shot.png", &[0x89, b'P', b'N', b'G', 0, 0, 0, 0]);
        let request = RunRequest {
            project: None,
            message: "分析并为我解释一下这个文档".to_owned(),
            pinned_context: vec![abs],
            provider: None,
            permission: Permission::Ask,
            mode: AgentMode::Code,
            fallback_to_local: true,
            max_output_tokens: 512,
            extended_thinking: false,
            session_id: None,
            user_skills_dir: None,
            mcp_servers: Vec::new(),
            agent_instructions: None,
            personas: vec![],
        };
        let mut progress: Vec<String> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        let mut emit = |event: SinkEvent| {
            if let SinkEvent::Progress { phase, detail } = &event {
                if phase == "analyze" {
                    progress.push(detail.clone());
                }
            }
            if let SinkEvent::Started {
                step: Step::AgentMessage { text, .. },
            } = event
            {
                texts.push(text);
            }
            true
        };
        run(&request, &|| true, &|_, _| true, &mut emit).expect("binary attachment run");
        assert!(
            progress.iter().any(|d| d.contains("1 context spans")),
            "descriptor must be a pre-observation: {progress:?}"
        );
        assert!(
            texts[0].contains("已附带附件"),
            "the not-inlined note must reach the answer: {}",
            texts[0]
        );
    }

    #[test]
    fn projectless_xlsx_attachment_inlines_extracted_cells() {
        // Layer 2 × Layer 3 together: on the conversational arm a pinned
        // xlsx must extract into a real content observation (钉住上下文 pin),
        // not fall back to the binary path descriptor (已附带附件).
        let xlsx = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/qa.xlsx");
        let abs = xlsx.to_string_lossy().to_string();
        let request = RunRequest {
            project: None,
            message: "分析并为我解释一下这个文档".to_owned(),
            pinned_context: vec![abs],
            provider: None,
            permission: Permission::Ask,
            mode: AgentMode::Code,
            fallback_to_local: true,
            max_output_tokens: 512,
            extended_thinking: false,
            session_id: None,
            user_skills_dir: None,
            mcp_servers: Vec::new(),
            agent_instructions: None,
            personas: vec![],
        };
        let mut progress: Vec<String> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        let mut emit = |event: SinkEvent| {
            if let SinkEvent::Progress { phase, detail } = &event {
                if phase == "analyze" {
                    progress.push(detail.clone());
                }
            }
            if let SinkEvent::Started {
                step: Step::AgentMessage { text, .. },
            } = event
            {
                texts.push(text);
            }
            true
        };
        run(&request, &|| true, &|_, _| true, &mut emit).expect("xlsx attachment run");
        assert!(
            progress.iter().any(|d| d.contains("1 context spans")),
            "extracted cells must be a pre-observation: {progress:?}"
        );
        assert!(
            texts[0].contains("钉住上下文"),
            "successful pin, not a descriptor: {}",
            texts[0]
        );
        assert!(
            !texts[0].contains("已附带附件"),
            "spreadsheet must not fall into the not-inlined arm: {}",
            texts[0]
        );
    }

    #[test]
    fn span_observation_caps_long_output_and_marks_truncation() {
        let span = ContextSpan {
            path: "big.txt".to_owned(),
            start_line: 1,
            end_line: 9,
            reason: "pinned by user".to_owned(),
            score: 50,
            snippet: "x".repeat(5_000),
            truncated: false,
            pinned: true,
            total_lines: 9,
            file_version: String::new(),
            stale: false,
            obs_id: String::new(),
        };
        let obs = span_observation(&span, "pin_0");
        assert!(obs.ok);
        assert!(
            obs.output.contains("result truncated: context block capped"),
            "capped body must say so: {}",
            &obs.output[obs.output.len().saturating_sub(120)..]
        );
        assert!(
            obs.output.chars().count() < 2_600,
            "observation stays within the cap + marker"
        );
    }

    #[test]
    fn span_observation_prefixes_stale_spans_old_version() {
        let span = ContextSpan {
            path: "stale.txt".to_owned(),
            start_line: 1,
            end_line: 2,
            reason: "pinned by user".to_owned(),
            score: 50,
            snippet: "old body".to_owned(),
            truncated: false,
            pinned: true,
            total_lines: 2,
            file_version: String::new(),
            stale: true,
            obs_id: String::new(),
        };
        let obs = span_observation(&span, "pin_1");
        assert!(obs.output.starts_with("[OLD VERSION]"), "{}", obs.output);
    }

    #[test]
    fn attachment_kind_labels_follow_extensions() {
        assert_eq!(attachment_kind_label("/tmp/a.pdf"), "pdf");
        assert_eq!(attachment_kind_label("/tmp/shot.PNG"), "image");
        assert_eq!(attachment_kind_label("/tmp/spec.docx"), "doc");
        assert_eq!(attachment_kind_label("/tmp/deck.pptx"), "ppt");
        assert_eq!(attachment_kind_label("/tmp/budget.xlsx"), "office");
        assert_eq!(attachment_kind_label("/tmp/blob"), "binary");
    }

    #[test]
    fn conversational_run_finishes_on_the_first_prose_answer() {
        // Regression (2026-09-29): this question used to inherit the feature
        // skill's acceptance criteria, bounce "criteria not met" six times,
        // and die as VerificationFailed → 部分完成. With the scripted fake
        // provider answering once, the run must end on that single prose
        // turn — exactly one ModelCall, plain ready+not_run, no footer.
        let dir = std::env::temp_dir().join(format!(
            "kodo-conv-{}-projectless",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let script = dir.join("script.jsonl");
        std::fs::write(&script, "{\"text\":\"数据仓库是集中存储分析数据的系统。\"}\n")
            .expect("script");
        let provider = Provider::new("fake", "fake-key", script.to_string_lossy(), "fake-model");
        let request = RunRequest {
            project: None,
            message: "介绍下什么是数据仓库".to_owned(),
            pinned_context: Vec::new(),
            provider: Some(provider),
            permission: Permission::Ask,
            mode: AgentMode::Code,
            fallback_to_local: true,
            max_output_tokens: 512,
            extended_thinking: false,
            session_id: None,
            user_skills_dir: None,
            mcp_servers: Vec::new(),
            agent_instructions: None,
            personas: vec![],
        };
        let mut kinds: Vec<String> = Vec::new();
        let mut answers: Vec<(String, String, String, bool)> = Vec::new();
        let mut emit = |event: SinkEvent| {
            if let SinkEvent::Started { step } = event {
                kinds.push(format!("{:?}", step.kind()));
                if let Step::AgentMessage {
                    text,
                    delivery,
                    verification,
                    plain,
                    ..
                } = step
                {
                    answers.push((text, delivery, verification, plain));
                }
            }
            true
        };
        run(&request, &|| true, &|_, _| true, &mut emit).expect("conversational run");
        assert_eq!(
            kinds.iter().filter(|k| *k == "ModelCall").count(),
            1,
            "one prose turn must end the run: {kinds:?}"
        );
        assert_eq!(answers.len(), 1, "exactly one final answer");
        let (text, delivery, verification, plain) = &answers[0];
        assert!(*plain, "Q&A renders plain");
        assert_eq!(delivery, "ready");
        assert_eq!(verification, "not_run");
        assert!(text.contains("数据仓库"), "scripted answer lost: {text}");
        assert!(!text.contains("Verification status"), "no task footer: {text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn knowledge_question_with_a_project_stays_conversational() {
        // 常规问题 with a project selected: same channel — the question never
        // becomes a feature task (no skill criteria, no context scan, no git
        // step), and the answer lands as plain ready+not_run (2026-09-29).
        let dir =
            std::env::temp_dir().join(format!("kodo-conv-{}-with-project", std::process::id()));
        let project = dir.join("repo");
        std::fs::create_dir_all(&project).expect("temp project");
        std::fs::write(project.join("README.md"), "# Demo\n").expect("readme");
        let script = dir.join("script.jsonl");
        std::fs::write(&script, "{\"text\":\"数据仓库集中存储分析数据。\"}\n").expect("script");
        let provider = Provider::new("fake", "fake-key", script.to_string_lossy(), "fake-model");
        let request = RunRequest {
            project: Some(project.clone()),
            message: "介绍下什么是数据仓库".to_owned(),
            pinned_context: Vec::new(),
            provider: Some(provider),
            permission: Permission::Ask,
            mode: AgentMode::Code,
            fallback_to_local: true,
            max_output_tokens: 512,
            extended_thinking: false,
            session_id: None,
            user_skills_dir: None,
            mcp_servers: Vec::new(),
            agent_instructions: None,
            personas: vec![],
        };
        let mut kinds: Vec<String> = Vec::new();
        let mut answers: Vec<(String, String, String, bool)> = Vec::new();
        let mut emit = |event: SinkEvent| {
            if let SinkEvent::Started { step } = event {
                kinds.push(format!("{:?}", step.kind()));
                if let Step::AgentMessage {
                    text,
                    delivery,
                    verification,
                    plain,
                    ..
                } = step
                {
                    answers.push((text, delivery, verification, plain));
                }
            }
            true
        };
        run(&request, &|| true, &|_, _| true, &mut emit).expect("with-project q&a run");
        assert_eq!(
            kinds.iter().filter(|k| *k == "ModelCall").count(),
            1,
            "one prose turn must end the run: {kinds:?}"
        );
        // No repo scan / git orientation on the conversational channel.
        assert!(
            kinds
                .iter()
                .all(|k| matches!(k.as_str(), "Reasoning" | "ModelCall" | "AgentMessage")),
            "conversational trace stays clean: {kinds:?}"
        );
        assert_eq!(answers.len(), 1, "exactly one final answer");
        let (text, delivery, verification, plain) = &answers[0];
        assert!(*plain, "Q&A renders plain");
        assert_eq!(delivery, "ready");
        assert_eq!(verification, "not_run");
        assert!(!text.contains("Verification status"), "no task footer: {text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_mode_parses_and_defaults_to_code() {
        assert_eq!(AgentMode::parse("work"), AgentMode::Work);
        assert_eq!(AgentMode::parse("code"), AgentMode::Code);
        assert_eq!(AgentMode::parse("nope"), AgentMode::Code);
        assert_eq!(AgentMode::Code.label(), "code");
    }

    /// Phase 0 second-pass — the screenshot's exact phrase must trigger
    /// the summary branch so the loop gets the larger 128-tool budget.
    #[test]
    fn is_summary_intent_detects_chinese_architecture_query() {
        assert!(is_summary_intent("为我输出该项目的总体架构设计"));
        assert!(is_summary_intent("总结这个仓库"));
        assert!(is_summary_intent("介绍下项目结构"));
        assert!(is_summary_intent("show me the project layout"));
        assert!(is_summary_intent("summarize the codebase"));
        assert!(is_summary_intent("describe the architecture"));
        assert!(is_summary_intent("give me an overview of the folder"));
        assert!(is_summary_intent("print the structure"));
    }

    #[test]
    fn is_summary_intent_rejects_when_mutating_verb_present() {
        assert!(!is_summary_intent("修复搜索框的 bug"));
        assert!(!is_summary_intent("添加一个新页面"));
        assert!(!is_summary_intent("重构 lib.rs"));
        assert!(!is_summary_intent("fix the test failure"));
        assert!(!is_summary_intent("write a README"));
        assert!(!is_summary_intent("delete the obsolete file"));
        assert!(!is_summary_intent("update the config"));
    }
}
