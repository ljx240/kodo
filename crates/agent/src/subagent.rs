//! Foreground delegation using the existing agent loop and its permission gates.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use kodo_core::session::{SubagentTrace, SubagentTraceStep};

use crate::protocol::{
    ToolArgs, ToolCall, ToolCallId, ToolError, ToolName, ToolRegistry, ToolResult,
};
use crate::{Alive, Approve, Emit, Permission, RunRequest, SinkEvent, Step, StepKind};

const MAX_CHILDREN: usize = 4;
const MAX_STEPS: usize = 48;
const CHILD_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Default)]
pub(crate) struct Scope {
    pub depth: u8,
    pub read_only: bool,
    pub allowed_tools: Option<Vec<String>>,
}

impl Scope {
    pub fn registry(&self, registry: ToolRegistry) -> ToolRegistry {
        registry.filtered(|name| {
            (self.depth == 0 || name != "subagent")
                && self
                    .allowed_tools
                    .as_ref()
                    .is_none_or(|tools| tools.iter().any(|t| t == name))
                && (!self.read_only
                    || (!name.starts_with("mcp__")
                        && !ToolName::parse(name).is_some_and(ToolName::is_mutation)))
        })
    }
}

#[derive(Default)]
pub(crate) struct Delegation {
    pub scope: Scope,
    pub count: usize,
}

pub(crate) struct Outcome {
    pub result: ToolResult,
    pub verification: String,
    pub keep: bool,
}

/// Only the exact command prefix is interpreted; ordinary prose stays unchanged.
pub(crate) fn command(message: &str) -> Option<Result<ToolCall, String>> {
    let rest = message.trim_start().strip_prefix("/subagent")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim();
    let (agent, task) = if let Some(role) = rest.strip_prefix('@') {
        let (name, task) = role.split_once(char::is_whitespace).unwrap_or((role, ""));
        (Some(name.to_owned()), task.trim())
    } else {
        (None, rest)
    };
    if task.is_empty() || task.chars().count() > 8_000 {
        return Some(Err(
            "用法：/subagent [@角色] <任务>，任务长度为 1–8000 字。".into(),
        ));
    }
    Some(Ok(ToolCall {
        id: ToolCallId::new("subagent-command"),
        name: ToolName::Subagent,
        args: ToolArgs::Subagent {
            task: task.to_owned(),
            agent,
            read_only: false,
        },
    }))
}

fn bounded(text: &str, cap: usize) -> String {
    let clean = crate::tools::redact_secrets(text);
    if clean.chars().count() <= cap {
        clean
    } else {
        format!(
            "{}\n…（已截断）",
            clean.chars().take(cap).collect::<String>()
        )
    }
}

fn public_step(step: &Step, status: &str, duration: Option<u64>) -> Option<SubagentTraceStep> {
    let (kind, label, output) = match step {
        Step::Reasoning { summary, .. } => ("reasoning", summary.clone(), String::new()),
        Step::Search { query, detail } => ("search", query.clone(), detail.clone()),
        Step::FileRead { path, detail } => ("fileRead", path.clone(), detail.clone()),
        Step::Command {
            command, output, ..
        } => ("commandExecution", command.clone(), output.clone()),
        Step::ModelCall {
            model,
            input_tokens,
            output_tokens,
            ..
        } => (
            "modelCall",
            model.clone(),
            format!("{input_tokens} → {output_tokens} tokens"),
        ),
        Step::FileChange { changes, .. } => (
            "fileChange",
            changes
                .iter()
                .map(|c| c.path.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            String::new(),
        ),
        // Final output is kept separately; private reasoning and detailed I/O never enter this trace.
        _ => return None,
    };
    Some(SubagentTraceStep {
        kind: kind.into(),
        label: bounded(&label, 240),
        status: status.into(),
        output: bounded(&output, 512),
        duration_ms: duration,
    })
}

fn terminal_status(
    live: bool,
    timed_out: bool,
    error: bool,
    delivery: &str,
    verification: &str,
) -> &'static str {
    if !live {
        "cancelled"
    } else if timed_out {
        "timed_out"
    } else if error {
        "failed"
    } else if delivery == "blocked" || verification == "blocked" {
        "blocked"
    } else if delivery == "ready" && matches!(verification, "passed" | "not_run") {
        "completed"
    } else {
        "failed"
    }
}

fn child_request(
    request: &RunRequest,
    task: &str,
    agent: &str,
    read_only: bool,
) -> Result<RunRequest, String> {
    let mut child = request.clone();
    child.message = task.into();
    child.conversation_history.clear();
    child.session_id = None;
    child.local_trace = false;
    child.fallback_to_local = false;
    child.allow_verify_commands = false;
    child.budget_family = None;
    // Decision assistance has a separate disclosure/approval scope; it is never implicitly delegated.
    child.decision = None;
    if read_only {
        child.permission = Permission::Ask;
    }
    if agent != "executor" {
        let user_dir = request
            .user_skills_dir
            .as_deref()
            .and_then(|dir| dir.parent())
            .map(|dir| dir.join("agents"));
        let spec = crate::agents::resolve_spec(
            user_dir
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("/nonexistent-kodo-agents")),
            request.project.as_deref(),
            agent,
        )?;
        // Role instructions may narrow behavior, but cannot switch model or permission.
        child.personas.push(crate::PersonaBlock {
            name: spec.name,
            instructions: spec.instructions,
            defaults: Default::default(),
            variables: spec.variables,
            examples: spec.examples,
        });
    }
    Ok(child)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute(
    call: &ToolCall,
    request: &RunRequest,
    registry: &ToolRegistry,
    delegation: &mut Delegation,
    inherited_read_only: bool,
    alive: &Alive,
    approve: &Approve,
    emit: &mut Emit,
) -> Outcome {
    let fail = |message: String| Outcome {
        result: ToolResult::failure(
            call.id.clone(),
            "subagent",
            call.args.label(),
            ToolError::execution(message),
        ),
        verification: "not_run".into(),
        keep: alive(),
    };
    if !alive() {
        return fail("父任务已停止，未启动子任务。".into());
    }
    if delegation.scope.depth >= 1 {
        return fail("子 Agent 不可继续委派（最大深度 1）。".into());
    }
    if delegation.count >= MAX_CHILDREN {
        return fail("本轮子任务已达到上限 4。".into());
    }
    let ToolArgs::Subagent {
        task,
        agent,
        read_only,
    } = &call.args
    else {
        return fail("subagent 参数类型无效。".into());
    };
    if task.trim().is_empty() || task.chars().count() > 8_000 {
        return fail("子任务长度必须为 1–8000 字。".into());
    }
    if request.project.is_none() {
        return fail("请先选择项目，再执行子任务。".into());
    }
    if !request
        .provider
        .as_ref()
        .is_some_and(|p| !p.api_key.trim().is_empty() && !p.resolved_model_id().trim().is_empty())
    {
        return fail("子任务需要已配置的模型；未执行任何子任务。".into());
    }
    let agent = agent.as_deref().unwrap_or("executor");
    let read_only = *read_only
        || inherited_read_only
        || delegation.scope.read_only
        || agent.ends_with("reviewer");
    let child = match child_request(request, task, agent, read_only) {
        Ok(child) => child,
        Err(error) => return fail(error),
    };
    delegation.count += 1;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut trace = SubagentTrace {
        id: format!(
            "subagent-{}-{}",
            kodo_core::session::now(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ),
        parent_id: request.session_id.clone().unwrap_or_else(|| "root".into()),
        agent: bounded(agent, 80),
        task: bounded(task, 8_000),
        status: "running".into(),
        output: String::new(),
        input_tokens: 0,
        output_tokens: 0,
        steps: Vec::new(),
        omitted_steps: 0,
    };
    let began = Instant::now();
    let sink_live = Cell::new(emit(SinkEvent::Subagent {
        trace: trace.clone(),
        duration_ms: 0,
    }));
    let child_alive = || alive() && sink_live.get() && began.elapsed() < CHILD_TIMEOUT;
    let scope = Scope {
        depth: delegation.scope.depth + 1,
        read_only,
        allowed_tools: Some(
            registry
                .definitions()
                .iter()
                .map(|d| d.name.to_owned())
                .chain(
                    registry
                        .external_definitions()
                        .iter()
                        .map(|d| d.name.clone()),
                )
                .collect(),
        ),
    };
    // Approval payloads are executable data at the existing boundary, so keep them byte-identical.
    let child_approve = |kind, detail: &str| {
        if read_only
            && match kind {
                StepKind::Command => {
                    crate::tools::classify_command_risk(detail)
                        != crate::tools::CommandRisk::ReadOnly
                }
                StepKind::FileChange | StepKind::ExternalTool | StepKind::SandboxEscalation => true,
                _ => false,
            }
        {
            return false;
        }
        approve(kind, detail)
    };
    let mut delivery = String::new();
    let mut verification = "not_run".to_owned();
    let mut open_step = None;
    let mut collect = |event: SinkEvent| {
        match event {
            SinkEvent::Started { step } => {
                if let Some(step) = public_step(&step, "running", None) {
                    if trace.steps.len() < MAX_STEPS {
                        open_step = Some(trace.steps.len());
                        trace.steps.push(step);
                    } else {
                        open_step = None;
                        trace.omitted_steps += 1;
                    }
                }
            }
            SinkEvent::Finished {
                step,
                duration_ms,
                denied,
            } => {
                if let Step::ModelCall {
                    input_tokens,
                    output_tokens,
                    ..
                } = &step
                {
                    trace.input_tokens = trace.input_tokens.saturating_add(*input_tokens as u64);
                    trace.output_tokens = trace.output_tokens.saturating_add(*output_tokens as u64);
                }
                if let Step::AgentMessage {
                    text,
                    delivery: d,
                    verification: v,
                    ..
                } = &step
                {
                    trace.output = bounded(text, 8_000);
                    delivery = d.clone();
                    verification = v.clone();
                }
                let failed = denied
                    || matches!(&step, Step::Command { exit_code: Some(code), .. } if *code != 0)
                    || matches!(&step, Step::ModelCall { finish_reason, .. } if matches!(finish_reason.as_str(), "error" | "cancelled"));
                if let Some(step) = public_step(
                    &step,
                    if failed { "failed" } else { "done" },
                    Some(duration_ms),
                ) {
                    if let Some(index) = open_step.take() {
                        trace.steps[index] = step;
                    } else if trace.steps.len() < MAX_STEPS {
                        trace.steps.push(step);
                    }
                }
            }
            SinkEvent::AgentMessage {
                text,
                delivery: d,
                verification: v,
                ..
            } => {
                trace.output = bounded(&text, 8_000);
                delivery = d;
                verification = v;
            }
            SinkEvent::AskUser { question, .. } => {
                trace.output = bounded(&question, 8_000);
                delivery = "blocked".into();
            }
            _ => return child_alive(),
        }
        let keep = emit(SinkEvent::Subagent {
            trace: trace.clone(),
            duration_ms: began.elapsed().as_millis() as u64,
        });
        sink_live.set(keep);
        child_alive()
    };
    let result = crate::run_scoped(&child, &child_alive, &child_approve, &mut collect, &scope);
    drop(collect);
    trace.status = terminal_status(
        alive() && sink_live.get(),
        began.elapsed() >= CHILD_TIMEOUT,
        result.is_err(),
        &delivery,
        &verification,
    )
    .into();
    if let Err(error) = result {
        trace.output = bounded(&format!("{error}\n{}", trace.output), 8_000);
    }
    for step in &mut trace.steps {
        if step.status == "running" {
            step.status = if trace.status == "cancelled" {
                "stopped"
            } else {
                "interrupted"
            }
            .into();
        }
    }
    if trace.output.is_empty() {
        if trace.status == "completed" {
            trace.status = "failed".into();
        }
        trace.output = format!("子任务结束：{}；未返回完整结果。", trace.status);
    }
    let ok = trace.status == "completed";
    let output = format!(
        "子任务 {} · {}\n状态：{}\n{}",
        trace.id, agent, trace.status, trace.output
    );
    let recorded = emit(SinkEvent::Subagent {
        trace,
        duration_ms: began.elapsed().as_millis() as u64,
    });
    let tool = if ok {
        ToolResult::success(call.id.clone(), "subagent", call.args.label(), output)
    } else {
        ToolResult::failure(
            call.id.clone(),
            "subagent",
            call.args.label(),
            ToolError::execution(output),
        )
    };
    Outcome {
        result: tool,
        verification,
        keep: alive() && sink_live.get() && recorded,
    }
}

pub(crate) fn run_command(
    command: Result<ToolCall, String>,
    request: &RunRequest,
    alive: &Alive,
    approve: &Approve,
    emit: &mut Emit,
) -> Result<(), String> {
    if request.project.is_none() {
        return crate::emit_knowledge_terminal(
            "请先选择项目，再执行子任务。".into(),
            "blocked",
            "",
            alive,
            emit,
        );
    }
    let call = match command {
        Ok(call) => call,
        Err(message) => return crate::emit_knowledge_terminal(message, "blocked", "", alive, emit),
    };
    let ToolArgs::Subagent { task, .. } = &call.args else {
        unreachable!()
    };
    let (chips, _) = crate::split_skill_chip(&request.message);
    let skills = crate::build_skill_registry(
        request.user_skills_dir.as_deref(),
        request.project.as_deref(),
        &request.skill_roots,
    );
    let skill = crate::resolve_skill(&skills, &chips, request.mode).or_else(|| {
        skills.select(if request.mode == crate::AgentMode::Work {
            crate::TaskType::Work
        } else {
            crate::classify_task(task)
        })
    });
    let mcp = crate::mcp::McpClients::deferred(&request.mcp_servers);
    let registry = crate::run_registry(request, skill, &mcp);
    if !registry.contains_name("subagent") {
        return crate::emit_knowledge_terminal(
            "当前项目或技能不允许委派子任务。".into(),
            "blocked",
            "",
            alive,
            emit,
        );
    }
    let mut changeset = request
        .project
        .as_deref()
        .map(crate::TurnChangeSet::capture_baseline)
        .unwrap_or_default();
    let before = request
        .project
        .as_deref()
        .map(|p| changeset.capture_command_files(p));
    let mut outcome = execute(
        &call,
        request,
        &registry,
        &mut Delegation::default(),
        skill.is_some_and(|s| s.context_strategy == crate::ContextStrategy::ReviewOnly),
        alive,
        approve,
        emit,
    );
    if let (Some(project), Some(before)) = (request.project.as_deref(), before) {
        crate::record_opaque_changes(
            project,
            before,
            &mut outcome.result,
            &mut changeset,
            alive,
            emit,
            request.local_trace,
        );
        crate::persist_changeset(project, request.session_id.as_deref(), &changeset)?;
    }
    if !outcome.keep || !alive() {
        return Ok(());
    }
    crate::simple_step(
        Step::AgentMessage {
            text: outcome.result.output,
            checks: Vec::new(),
            delivery: if outcome.result.ok { "ready" } else { "failed" }.into(),
            verification: outcome.verification,
            plain: false,
            failure_reason: if outcome.result.ok {
                ""
            } else {
                "unrecoverable"
            }
            .into(),
        },
        alive,
        emit,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{parse_model_turn, ModelTurn, ToolInvocation};
    use serde_json::json;

    struct Fixture {
        root: std::path::PathBuf,
        script: std::path::PathBuf,
    }
    impl Fixture {
        fn new(replies: Vec<serde_json::Value>) -> Self {
            static ID: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "kodo-subagent-{}-{}",
                std::process::id(),
                ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("README.md"), "Synthetic routing evidence.\n").unwrap();
            let script = root.with_extension("responses");
            std::fs::write(
                &script,
                replies
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n",
            )
            .unwrap();
            Self { root, script }
        }
        fn request(&self) -> RunRequest {
            let mut request = crate::tests::temp_request(&self.root, Permission::Full);
            request.mode = crate::AgentMode::Work;
            request.provider = Some(crate::Provider::new(
                "fake",
                "synthetic-key",
                self.script.to_string_lossy(),
                "fake-model",
            ));
            request
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
            let _ = std::fs::remove_file(&self.script);
        }
    }
    fn call(task: &str) -> ToolCall {
        ToolCall {
            id: ToolCallId::new("delegate-1"),
            name: ToolName::Subagent,
            args: ToolArgs::Subagent {
                task: task.into(),
                agent: None,
                read_only: false,
            },
        }
    }
    fn reply(text: &str) -> serde_json::Value {
        json!({"text":text})
    }
    fn tool(name: &str, args: serde_json::Value) -> serde_json::Value {
        reply(&json!({"tool_calls":[{"id":"child-tool","name":name,"arguments":args}]}).to_string())
    }

    #[test]
    fn slash_and_wire_validate_tasks_and_optional_arguments() {
        assert!(command("/subagents task").is_none());
        assert!(command("ordinary /subagent task").is_none());
        assert!(command("/subagent").unwrap().is_err());
        assert!(command("/subagent @reviewer").unwrap().is_err());
        let command = command(" /subagent @reviewer 检查路由\n保留证据")
            .unwrap()
            .unwrap();
        assert!(
            matches!(command.args, ToolArgs::Subagent { agent: Some(ref a), ref task, .. } if a == "reviewer" && task.contains("保留证据"))
        );
        let registry = ToolRegistry::standard();
        let ModelTurn::Tools { calls } = parse_model_turn(
            r#"{"tool_calls":[{"name":"subagent","arguments":{"task":"inspect","read_only":true}}]}"#,
            &registry,
        ) else {
            panic!("tools")
        };
        assert!(matches!(
            &calls[0],
            ToolInvocation::Ready(ToolCall {
                args: ToolArgs::Subagent {
                    read_only: true,
                    ..
                },
                ..
            })
        ));
        for args in [
            json!({"task":""}),
            json!({"task":"x","read_only":"false"}),
            json!({"task":"x".repeat(8001)}),
        ] {
            assert!(matches!(
                registry.accept(ToolCallId::new("invalid"), "subagent", &args),
                ToolInvocation::Rejected(_)
            ));
        }
    }

    #[test]
    fn scope_can_only_narrow_tools_and_cannot_expose_nested_delegation() {
        let scope = Scope {
            depth: 1,
            read_only: true,
            allowed_tools: Some(vec![
                "read_file".into(),
                "write_file".into(),
                "subagent".into(),
            ]),
        };
        let registry = scope.registry(ToolRegistry::standard());
        assert!(registry.contains_name("read_file"));
        for tool in ["write_file", "subagent", "run_command", "websearch"] {
            assert!(!registry.contains_name(tool));
        }
    }

    #[test]
    fn automatic_delegation_policy_reaches_both_protocols_only_when_available() {
        let fixture = Fixture::new(vec![]);
        let mut request = fixture.request();
        let skills = crate::SkillRegistry::builtin();
        let mcp = crate::mcp::McpClients::default();
        for (mode, skill_name) in [
            (crate::AgentMode::Code, "feature"),
            (crate::AgentMode::Work, "work"),
        ] {
            request.mode = mode;
            let mut skill = skills.get(skill_name).unwrap().clone();
            let registry = crate::run_registry(&request, Some(&skill), &mcp);
            let schemas = crate::tool_schemas(&registry);
            let native = schemas.iter().find(|s| s.name == "subagent").unwrap();
            let fallback = crate::protocol::protocol_instructions(&registry);
            let system = crate::system_prompt(request.project.as_deref(), mode, &registry, "");
            assert!(
                system.find("Task orchestration").unwrap() < system.find("Tool protocol").unwrap()
            );
            for policy in [
                "Automatically decide",
                "user does not need",
                "simple questions",
                "avoid delegation",
                "sequentially",
            ] {
                assert!(native.description.contains(policy), "{policy}");
                assert!(fallback.contains(policy), "{policy}");
            }
            skill
                .allowed_tools
                .retain(|tool| *tool != ToolName::Subagent);
            let child_scope = Scope {
                depth: 1,
                ..Scope::default()
            };
            let mut projectless = request.clone();
            projectless.project = None;
            for restricted in [
                child_scope.registry(registry),
                crate::run_registry(&request, Some(&skill), &mcp),
                crate::run_registry(&projectless, None, &mcp),
            ] {
                assert!(!crate::tool_schemas(&restricted)
                    .iter()
                    .any(|s| s.name == "subagent"));
                assert!(!crate::protocol::protocol_instructions(&restricted)
                    .contains("Automatically decide"));
                assert!(
                    !crate::system_prompt(request.project.as_deref(), mode, &restricted, "")
                        .contains("Task orchestration")
                );
            }
        }
    }

    #[test]
    fn child_request_isolates_history_and_keeps_security_and_context() {
        let fixture = Fixture::new(vec![]);
        let mut parent = fixture.request();
        parent.conversation_history = vec![("assistant".into(), "parent private history".into())];
        parent.pinned_context = vec!["README.md".into()];
        parent.local_trace = true;
        parent.session_id = Some("parent-session".into());
        parent.allow_verify_commands = true;
        let child = child_request(&parent, "child task", "reviewer", true).unwrap();
        assert!(child.conversation_history.is_empty());
        assert!(child.session_id.is_none());
        assert!(!child.local_trace && !child.fallback_to_local && !child.allow_verify_commands);
        assert_eq!(child.project, parent.project);
        assert_eq!(child.pinned_context, parent.pinned_context);
        assert_eq!(child.permission, Permission::Ask);
        assert_eq!(
            child.sandbox.as_ref().unwrap().mode(),
            parent.sandbox.as_ref().unwrap().mode()
        );
        assert_eq!(
            child.provider.as_ref().unwrap().model,
            parent.provider.as_ref().unwrap().model
        );
        assert_eq!(child.personas.last().unwrap().name, "reviewer");
        assert!(child.personas.last().unwrap().defaults.model.is_none());
        assert!(child_request(&parent, "task", "../missing", false).is_err());
    }

    #[test]
    fn executes_real_child_tools_and_keeps_final_answer_inside_child_trace() {
        let fixture = Fixture::new(vec![
            tool("read_file", json!({"path":"README.md"})),
            reply("已读取路由资料，结论已核对。"),
        ]);
        let request = fixture.request();
        let mut traces = Vec::new();
        let outcome = execute(
            &call("介绍下什么是路由"),
            &request,
            &ToolRegistry::standard(),
            &mut Delegation::default(),
            false,
            &|| true,
            &|_, _| true,
            &mut |event| {
                if let SinkEvent::Subagent { trace, .. } = event {
                    traces.push(trace);
                } else {
                    panic!("child leaked a parent event");
                }
                true
            },
        );
        assert!(outcome.result.ok, "{}", outcome.result.output);
        let final_trace = traces.last().unwrap();
        assert_eq!(final_trace.status, "completed");
        assert!(final_trace
            .steps
            .iter()
            .any(|s| s.kind == "fileRead" && s.output.contains("Synthetic")));
        assert!(traces.iter().all(|t| t.id == final_trace.id));
        assert!(std::fs::read_to_string(&fixture.script).unwrap().is_empty());
    }

    #[test]
    fn read_only_child_cannot_write_even_with_allowing_approver() {
        let fixture = Fixture::new(vec![
            tool(
                "write_file",
                json!({"path":"must-not-exist.txt","content":"bad"}),
            ),
            tool("run_command", json!({"command":"touch must-not-exist.txt"})),
            reply("未修改文件。"),
        ]);
        let request = fixture.request();
        let mut snapshots = Vec::new();
        let _ = execute(
            &call("介绍下什么是路由"),
            &request,
            &ToolRegistry::standard(),
            &mut Delegation::default(),
            true,
            &|| true,
            &|_, _| panic!("read-only child dispatched a forbidden effect"),
            &mut |event| {
                if let SinkEvent::Subagent { trace, .. } = event {
                    snapshots.push(trace);
                }
                true
            },
        );
        assert!(!fixture.root.join("must-not-exist.txt").exists());
        assert!(snapshots
            .last()
            .unwrap()
            .steps
            .iter()
            .all(|s| s.kind != "fileChange"));
    }

    #[test]
    fn cancellation_and_sink_failure_prevent_provider_dispatch() {
        let fixture = Fixture::new(vec![reply("must remain unread")]);
        for stopped in [true, false] {
            let result = execute(
                &call("介绍下什么是路由"),
                &fixture.request(),
                &ToolRegistry::standard(),
                &mut Delegation::default(),
                false,
                &|| !stopped,
                &|_, _| true,
                &mut |_| false,
            );
            assert!(!result.result.ok && !result.keep);
        }
        assert!(std::fs::read_to_string(&fixture.script)
            .unwrap()
            .contains("must remain unread"));
    }

    #[test]
    fn limits_and_provider_failures_are_explicit_and_preserve_partial_status() {
        let fixture = Fixture::new(vec![json!({"error":"synthetic failure", "status":401})]);
        for mut delegation in [
            Delegation {
                count: 4,
                ..Default::default()
            },
            Delegation {
                scope: Scope {
                    depth: 1,
                    ..Default::default()
                },
                count: 0,
            },
        ] {
            let result = execute(
                &call("task"),
                &fixture.request(),
                &ToolRegistry::standard(),
                &mut delegation,
                false,
                &|| true,
                &|_, _| true,
                &mut |_| true,
            );
            assert!(!result.result.ok);
        }
        let result = execute(
            &call("介绍下什么是路由"),
            &fixture.request(),
            &ToolRegistry::standard(),
            &mut Delegation::default(),
            false,
            &|| true,
            &|_, _| true,
            &mut |_| true,
        );
        assert!(!result.result.ok);
        assert!(result.result.output.contains("failed"));
        assert_eq!(
            terminal_status(true, false, false, "partial", "passed"),
            "failed"
        );
        assert_eq!(
            terminal_status(true, false, false, "ready", "failed"),
            "failed"
        );
        assert_eq!(
            terminal_status(true, false, false, "blocked", "not_run"),
            "blocked"
        );
        assert_eq!(
            terminal_status(true, true, false, "ready", "passed"),
            "timed_out"
        );
        assert_eq!(
            terminal_status(false, false, false, "ready", "passed"),
            "cancelled"
        );
    }

    #[test]
    fn private_reasoning_is_not_archived_and_public_text_is_bounded() {
        assert!(public_step(
            &Step::Thinking {
                content: "private reasoning".into(),
                phase: "execute".into(),
                source: "native".into()
            },
            "done",
            Some(1)
        )
        .is_none());
        let step = public_step(
            &Step::Command {
                command: "probe".into(),
                cwd: ".".into(),
                output: "x".repeat(10000),
                exit_code: Some(0),
            },
            "done",
            Some(1),
        )
        .unwrap();
        assert!(step.output.chars().count() < 550);
        assert!(!bounded("api_key=sk-synthetic-secret-value-123456789", 512)
            .contains("sk-synthetic-secret-value-123456789"));
    }

    #[test]
    fn parent_continues_after_child_result_and_slash_runs_the_same_child() {
        let fixture = Fixture::new(vec![
            tool("subagent", json!({"task":"介绍下什么是路由"})),
            reply("child evidence"),
            reply("parent synthesis"),
        ]);
        let mut request = fixture.request();
        request.message = "介绍下什么是路由".into();
        let mut answers = Vec::new();
        let mut traces = Vec::new();
        crate::run(&request, &|| true, &|_, _| true, &mut |event| {
            match event {
                SinkEvent::Subagent { trace, .. } => traces.push(trace),
                SinkEvent::Finished {
                    step: Step::AgentMessage { text, .. },
                    ..
                } => answers.push(text),
                _ => {}
            }
            true
        })
        .unwrap();
        assert_eq!(answers, vec!["parent synthesis"]);
        assert_eq!(traces.last().unwrap().output, "child evidence");

        let fixture = Fixture::new(vec![reply("slash child answer")]);
        let mut request = fixture.request();
        request.message = "/subagent 介绍下什么是路由".into();
        let mut answer = String::new();
        crate::run(&request, &|| true, &|_, _| true, &mut |event| {
            if let SinkEvent::Finished {
                step: Step::AgentMessage { text, .. },
                ..
            } = event
            {
                answer = text;
            }
            true
        })
        .unwrap();
        assert!(answer.contains("slash child answer"), "{answer}");
    }

    #[test]
    fn ordinary_task_can_delegate_real_reads_before_parent_synthesis() {
        let fixture = Fixture::new(vec![
            tool(
                "subagent",
                json!({"task":"README.md 的输入格式有什么含义？请读取文件后回答。", "read_only":true}),
            ),
            tool("read_file", json!({"path":"README.md"})),
            reply("child evidence"),
            reply("parent synthesis"),
        ]);
        let mut request = fixture.request();
        request.message =
            "输入格式与校验规则有什么区别？请分别解释输入格式的边界与规则约束。".into();
        let mut traces = Vec::new();
        let mut answers = Vec::new();
        crate::run(&request, &|| true, &|_, _| true, &mut |event| {
            match event {
                SinkEvent::Subagent { trace, .. } => traces.push(trace),
                SinkEvent::Finished {
                    step: Step::AgentMessage { text, .. },
                    ..
                } => answers.push(text),
                _ => {}
            }
            true
        })
        .unwrap();
        let trace = traces.last().unwrap();
        assert_eq!(trace.status, "completed", "{}", trace.output);
        assert!(trace
            .steps
            .iter()
            .any(|s| s.kind == "fileRead" && s.label == "README.md"));
        assert_eq!(trace.output, "child evidence");
        assert_eq!(answers, vec!["parent synthesis"]);
    }

    #[test]
    fn parent_finishes_remaining_work_after_delegated_changes_and_verification() {
        let fixture = Fixture::new(vec![
            tool("read_file", json!({"path":"input.txt"})),
            tool("read_file", json!({"path":"README.md"})),
            tool(
                "subagent",
                json!({"task":"新增 child.txt，内容为 child-output。"}),
            ),
            tool("read_file", json!({"path":"input.txt"})),
            tool("read_file", json!({"path":"README.md"})),
            tool(
                "write_file",
                json!({"path":"child.txt","content":"child-output"}),
            ),
            tool(
                "write_file",
                json!({"path":"parent.txt","content":"parent-output"}),
            ),
            reply("parent-synthesis: child.txt 与 parent.txt 已实际创建。"),
        ]);
        std::fs::write(fixture.root.join("input.txt"), "synthetic input\n").unwrap();
        let mut request = fixture.request();
        request.mode = crate::AgentMode::Code;
        request.message = "新增两个产物 child.txt 与 parent.txt，完成后总结两项结果。".into();
        let mut final_message = None;
        let mut traces = Vec::new();
        crate::run(&request, &|| true, &|_, _| true, &mut |event| {
            match event {
                SinkEvent::Subagent { trace, .. } => traces.push(trace),
                SinkEvent::Finished {
                    step: Step::AgentMessage { text, delivery, .. },
                    ..
                } => final_message = Some((text, delivery)),
                _ => {}
            }
            true
        })
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("child.txt")).unwrap(),
            "child-output"
        );
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("parent.txt")).unwrap(),
            "parent-output"
        );
        assert_eq!(traces.last().unwrap().status, "completed");
        let (text, delivery) = final_message.unwrap();
        assert_eq!(delivery, "ready", "{text}");
        assert!(text.contains("parent-synthesis"), "{text}");
    }

    #[test]
    fn read_only_explanation_of_readme_does_not_require_document_writes() {
        for mode in [crate::AgentMode::Code, crate::AgentMode::Work] {
            let fixture = Fixture::new(vec![
                tool("read_file", json!({"path":"README.md"})),
                reply("两项差异已根据 README.md 读取结果解释；未执行代码、未修改文件。"),
            ]);
            let mut request = fixture.request();
            request.mode = mode;
            let mut traces = Vec::new();
            let outcome = execute(
                &call(
                    "只读复核 README.md 的内部实现与规范有何不一致？说明 catch 分支写入 failed 记录的控制流，不要分析其他模块的内部实现。输出为两条结论，不执行代码，不要修改任何文件。",
                ),
                &request,
                &ToolRegistry::standard(),
                &mut Delegation::default(),
                true,
                &|| true,
                &|kind, _| {
                    assert_ne!(kind, StepKind::FileChange);
                    true
                },
                &mut |event| {
                    if let SinkEvent::Subagent { trace, .. } = event {
                        traces.push(trace);
                    }
                    true
                },
            );
            let trace = traces.last().unwrap();
            assert!(outcome.result.ok, "{}", trace.output);
            assert_eq!(trace.status, "completed");
            assert!(trace
                .steps
                .iter()
                .any(|s| s.kind == "fileRead" && s.label == "README.md"));
            assert!(trace.steps.iter().all(|s| s.kind != "fileChange"));
        }
    }

    #[test]
    fn read_only_child_keeps_explicit_writing_and_review_contracts() {
        use crate::AgentMode::{Code, Work};
        for (mode, task) in [
            (Code, "将 report.txt 保存"),
            (Work, "将 report.txt 保存"),
            (Code, "保存一份文档"),
            (Code, "保存到 report.txt"),
            (Work, "保存到 report.txt"),
            (Code, "说明 README.md 并保存到 report.md"),
            (Work, "说明 README.md 并保存到 report.md"),
            (Code, "【技能：docs】\n解释 README.md，不要修改文件。"),
            (Code, "Review the code in README.md"),
        ] {
            let fixture = Fixture::new(vec![
                tool("read_file", json!({"path":"README.md"})),
                tool("read_file", json!({"path":"README.md"})),
                reply("Only an explanation, without a saved document or a review report."),
            ]);
            let mut request = fixture.request();
            request.mode = mode;
            let outcome = execute(
                &call(task),
                &request,
                &ToolRegistry::standard(),
                &mut Delegation::default(),
                true,
                &|| true,
                &|kind, _| {
                    assert_ne!(kind, StepKind::FileChange);
                    true
                },
                &mut |_| true,
            );
            assert!(!outcome.result.ok, "{mode:?}: {task}");
            assert!(!fixture.root.join("report.md").exists());
        }
    }

    #[test]
    fn simple_question_answers_directly_without_a_child() {
        let fixture = Fixture::new(vec![reply("count 表示数量，total 表示总和。")]);
        let mut request = fixture.request();
        request.message = "count 和 total 有什么区别？".into();
        let mut child_count = 0;
        let mut answer = String::new();
        crate::run(&request, &|| true, &|_, _| true, &mut |event| {
            match event {
                SinkEvent::Subagent { .. } => child_count += 1,
                SinkEvent::Finished {
                    step: Step::AgentMessage { text, .. },
                    ..
                } => answer = text,
                _ => {}
            }
            true
        })
        .unwrap();
        assert_eq!(child_count, 0);
        assert!(answer.contains("count 表示数量"), "{answer}");
    }

    #[test]
    fn unpinned_script_task_keeps_command_tools_and_cancels_its_process() {
        let fixture = Fixture::new(vec![tool("run_command", json!({"command":"sh cancel.sh"}))]);
        std::fs::write(
            fixture.root.join("cancel.sh"),
            "printf started > started.txt\nsleep 10\nprintf forbidden > must-not-exist.txt\n",
        )
        .unwrap();
        let mut request = fixture.request();
        request.mode = crate::AgentMode::Code;
        request.message = "/subagent 请执行这个合成取消测试：读取 README.md 和 cancel.sh，然后通过命令工具运行 sh cancel.sh。不要修改脚本。".into();
        let mut traces = Vec::new();
        let alive = || !fixture.root.join("started.txt").exists();
        crate::run(&request, &alive, &|_, _| true, &mut |event| {
            if let SinkEvent::Subagent { trace, .. } = event {
                traces.push(trace);
            }
            true
        })
        .unwrap();
        assert!(fixture.root.join("started.txt").exists());
        assert!(!fixture.root.join("must-not-exist.txt").exists());
        let trace = traces.last().unwrap();
        assert_eq!(trace.status, "cancelled", "{}", trace.output);
        assert!(trace
            .steps
            .iter()
            .any(|step| step.kind == "commandExecution" && step.label == "sh cancel.sh"));
    }

    #[test]
    fn accepted_child_tools_produce_a_completion_summary_without_a_model_final() {
        for verify in [true, false] {
            let fixture = Fixture::new(vec![
                tool("read_file", json!({"path":"input.txt"})),
                tool("read_file", json!({"path":"README.md"})),
                tool(
                    "write_file",
                    json!({"path":"result.json","content":r#"{"count":3,"total":21}"#}),
                ),
                reply("MUST_NOT_CALL_AFTER_ACCEPTANCE"),
            ]);
            std::fs::write(fixture.root.join("input.txt"), "3\n7\n11\n").unwrap();
            if verify {
                std::fs::write(
                    fixture.root.join("package.json"),
                    r#"{"private":true,"scripts":{"test":"printf 'SYNTHETIC_CHECK_OK\n'"}}"#,
                )
                .unwrap();
            }
            let mut request = fixture.request();
            request.mode = crate::AgentMode::Code;
            request.message = "/subagent 新增汇总产物 result.json，计算数量和总和。".into();
            let mut traces = Vec::new();
            let mut final_message = None;
            crate::run(&request, &|| true, &|_, _| true, &mut |event| {
                match event {
                    SinkEvent::Subagent { trace, .. } => traces.push(trace),
                    SinkEvent::Finished {
                        step:
                            Step::AgentMessage {
                                text,
                                delivery,
                                verification,
                                ..
                            },
                        ..
                    } => {
                        final_message = Some((text, delivery, verification));
                    }
                    _ => {}
                }
                true
            })
            .unwrap();
            let trace = traces.last().unwrap();
            let (text, delivery, verification) = final_message.unwrap();
            assert_eq!(trace.status, "completed", "{verify}: {trace:?}");
            assert_eq!(delivery, "ready", "{verify}: {text}");
            assert_eq!(verification, if verify { "passed" } else { "not_run" });
            assert!(text.contains("result.json"), "{text}");
            for contradiction in [
                "任务尚未完成",
                "未产出可用回答",
                "MUST_NOT_CALL_AFTER_ACCEPTANCE",
            ] {
                assert!(!text.contains(contradiction), "{verify}: {text}");
            }
            if verify {
                assert!(text.contains("npm run test"), "{text}");
            } else {
                assert!(
                    text.contains("未运行验证") || text.contains("未经验证"),
                    "{text}"
                );
            }
        }
    }

    #[test]
    fn delegated_writes_enter_parent_diff_and_safe_undo() {
        let fixture = Fixture::new(vec![
            tool(
                "create_file",
                json!({"path":"child.txt","content":"child-created\n"}),
            ),
            reply("已创建文件。"),
        ]);
        let request = fixture.request();
        let mut changeset = crate::TurnChangeSet::capture_baseline(&fixture.root);
        let mut wrote = false;
        let results = crate::run_invocations(
            vec![ToolInvocation::Ready(call("介绍下什么是路由"))],
            &fixture.root,
            &request,
            None,
            &mut crate::mcp::McpClients::default(),
            &|| true,
            &|_, _| true,
            &mut |_| true,
            &mut Vec::new(),
            &mut wrote,
            &mut changeset,
            &mut crate::dedup::TurnDedup::new(),
            &mut Delegation::default(),
        )
        .unwrap()
        .unwrap();
        assert!(wrote, "parent must know child writes");
        assert!(results[0].changed_files.iter().any(|p| p == "child.txt"));
        assert!(changeset.kodo_touched.contains("child.txt"));
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("child.txt")).unwrap(),
            "child-created\n"
        );
        let report = changeset.undo_kodo_changes(&fixture.root);
        assert!(report.is_ok());
        assert!(!fixture.root.join("child.txt").exists());
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("README.md")).unwrap(),
            "Synthetic routing evidence.\n"
        );
    }

    #[test]
    fn forged_child_calls_still_fail_at_execution_boundary() {
        let fixture = Fixture::new(vec![]);
        let mut delegation = Delegation {
            scope: Scope {
                depth: 1,
                read_only: true,
                allowed_tools: None,
            },
            count: 0,
        };
        let calls = vec![
            ToolInvocation::Ready(ToolCall {
                id: ToolCallId::new("w"),
                name: ToolName::WriteFile,
                args: ToolArgs::WriteFile {
                    path: "forbidden.txt".into(),
                    content: "bad".into(),
                },
            }),
            ToolInvocation::Ready(ToolCall {
                id: ToolCallId::new("c"),
                name: ToolName::RunCommand,
                args: ToolArgs::RunCommand {
                    command: "touch forbidden.txt".into(),
                },
            }),
            ToolInvocation::Ready(call("nested task")),
        ];
        let results = crate::run_invocations(
            calls,
            &fixture.root,
            &fixture.request(),
            None,
            &mut crate::mcp::McpClients::default(),
            &|| true,
            &|_, _| panic!("must reject before approval"),
            &mut |_| true,
            &mut Vec::new(),
            &mut false,
            &mut crate::TurnChangeSet::default(),
            &mut crate::dedup::TurnDedup::new(),
            &mut delegation,
        )
        .unwrap()
        .unwrap();
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| !r.ok));
        assert!(!fixture.root.join("forbidden.txt").exists());
    }
}
