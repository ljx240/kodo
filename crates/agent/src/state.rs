//! Pure agent state machine — unit-testable, no I/O, no multi-agent.
//!
//! The production loop in `lib.rs` feeds [`AgentEvent`]s derived from real
//! provider/tool traffic. Tests feed the same events from fake provider +
//! fake tools, so transitions never depend on the network.

use crate::plan::{AcceptanceEvidence, AcceptanceReport, TaskPlan};
use crate::protocol::ToolResult;

/// Logical states of one agent turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    Understand,
    Plan,
    GatherContext,
    Execute,
    Verify,
    Repair,
    Finish,
    Failed { reason: FailReason },
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    /// A budget hit its cap — explicit partial/failure, never fake success.
    BudgetExhausted,
    /// Finish was requested but acceptance criteria are unmet.
    AcceptanceUnmet,
    /// Unrecoverable internal / tool policy failure.
    Unrecoverable,
}

impl AgentState {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Understand => "Understand",
            Self::Plan => "Plan",
            Self::GatherContext => "GatherContext",
            Self::Execute => "Execute",
            Self::Verify => "Verify",
            Self::Repair => "Repair",
            Self::Finish => "Finish",
            Self::Failed { .. } => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Finish | Self::Failed { .. } | Self::Cancelled)
    }
}

/// Hard budgets for one turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budget {
    pub max_rounds: usize,
    pub max_tool_calls: usize,
    pub max_repairs: usize,
    pub rounds_used: usize,
    pub tool_calls_used: usize,
    pub repairs_used: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_rounds: 6,
            max_tool_calls: 32,
            max_repairs: 3,
            rounds_used: 0,
            tool_calls_used: 0,
            repairs_used: 0,
        }
    }
}

impl Budget {
    pub fn rounds_exhausted(&self) -> bool {
        self.rounds_used >= self.max_rounds
    }

    pub fn tools_exhausted(&self) -> bool {
        self.tool_calls_used >= self.max_tool_calls
    }

    pub fn repairs_exhausted(&self) -> bool {
        self.repairs_used >= self.max_repairs
    }

    /// Auto-trip Failed only for rounds/tools; repair caps are checked on the
    /// repair transition itself (so denial→Plan is not stolen by BudgetExhausted).
    pub fn any_exhausted(&self) -> bool {
        self.rounds_exhausted() || self.tools_exhausted()
    }

    pub fn summary(&self) -> String {
        format!(
            "rounds {}/{} · tools {}/{} · repairs {}/{}",
            self.rounds_used,
            self.max_rounds,
            self.tool_calls_used,
            self.max_tool_calls,
            self.repairs_used,
            self.max_repairs
        )
    }
}

/// Events that drive transitions. Produced by the loop (or fakes in tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    /// User message received.
    TaskReceived,
    /// Structured plan is ready (model JSON or heuristic).
    PlanReady,
    /// Pre-scan / context tools finished.
    ContextGathered,
    /// A model turn requested N tool calls (charged on ToolsFinished).
    ModelRequestedTools { count: usize },
    /// Tool executions finished (including rejections as failed results).
    ToolsFinished { results: Vec<ToolResult> },
    /// Verification command finished.
    VerifyFinished { ok: bool },
    /// Model produced final prose (claim only — not sufficient for Finish).
    ModelClaimedDone,
    /// Enter Repair after failed verify (charges one repair).
    BeginRepair,
    /// Repair work finished; back to Execute.
    RepairApplied,
    /// User requested stop / alive() went false.
    Cancel,
    /// Explicit budget trip (also checked automatically after each event).
    BudgetExceeded,
}

/// Outcome of feeding one event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// State (or internal evidence) updated; `from` → `to`.
    Advanced { from: AgentState, to: AgentState },
    /// Event accepted while staying in the same state (evidence/budget only).
    Stayed { state: AgentState },
    /// Illegal / ignored event — machine does not panic.
    Rejected { state: AgentState, reason: String },
    /// Terminal state: further events are ignored.
    AlreadyTerminal { state: AgentState },
}

/// The pure state machine for a single turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMachine {
    state: AgentState,
    plan: TaskPlan,
    budget: Budget,
    evidence: AcceptanceEvidence,
    last_acceptance: Option<AcceptanceReport>,
    pending_tool_count: usize,
}

impl AgentMachine {
    pub fn new(task: &str, budget: Budget) -> Self {
        Self {
            state: AgentState::Understand,
            plan: TaskPlan::from_task(task),
            budget,
            evidence: AcceptanceEvidence::default(),
            last_acceptance: None,
            pending_tool_count: 0,
        }
    }

    pub fn with_plan(task: &str, plan: TaskPlan, budget: Budget) -> Self {
        let mut m = Self::new(task, budget);
        m.plan = plan;
        m
    }

    pub fn state(&self) -> &AgentState {
        &self.state
    }

    pub fn plan(&self) -> &TaskPlan {
        &self.plan
    }

    pub fn plan_mut(&mut self) -> &mut TaskPlan {
        &mut self.plan
    }

    pub fn budget(&self) -> &Budget {
        &self.budget
    }

    pub fn evidence(&self) -> &AcceptanceEvidence {
        &self.evidence
    }

    pub fn last_acceptance(&self) -> Option<&AcceptanceReport> {
        self.last_acceptance.as_ref()
    }

    /// Non-CoT progress line for Reasoning steps.
    pub fn progress_summary(&self) -> String {
        format!(
            "{} · {} · {}",
            self.plan.progress_summary(self.state.name()),
            self.budget.summary(),
            evidence_summary(&self.evidence)
        )
    }

    /// Feed one event. Budget is re-checked after successful handling.
    pub fn handle(&mut self, event: AgentEvent) -> Transition {
        if self.state.is_terminal() {
            return Transition::AlreadyTerminal { state: self.state.clone() };
        }

        // Cancel wins from any non-terminal state.
        if matches!(event, AgentEvent::Cancel) {
            let from = std::mem::replace(&mut self.state, AgentState::Cancelled);
            return Transition::Advanced { from, to: AgentState::Cancelled };
        }

        let from = self.state.clone();
        let next = match self.step(event) {
            Ok(next) => next,
            Err(reason) => {
                return Transition::Rejected { state: self.state.clone(), reason };
            }
        };

        // Budget trip after applying the event (except when already failing/cancel).
        if !matches!(next, AgentState::Failed { .. } | AgentState::Cancelled | AgentState::Finish)
            && self.budget.any_exhausted()
        {
            let to = AgentState::Failed { reason: FailReason::BudgetExhausted };
            self.state = to.clone();
            return Transition::Advanced { from, to };
        }

        if next == self.state {
            Transition::Stayed { state: self.state.clone() }
        } else {
            self.state = next.clone();
            Transition::Advanced { from, to: next }
        }
    }

    /// Inner transition; `Err` = rejected (stay put).
    fn step(&mut self, event: AgentEvent) -> Result<AgentState, String> {
        use AgentEvent::*;
        use AgentState as S;

        match (&self.state.clone(), event) {
            // --- Understand ---
            (S::Understand, TaskReceived) => Ok(S::Understand),
            (S::Understand, PlanReady) => Ok(S::Plan),

            // --- Plan ---
            (S::Plan, ContextGathered) => Ok(S::GatherContext),
            // Allow direct jump if context was gathered before plan locked in.
            (S::Plan, PlanReady) => Ok(S::Plan),

            // --- GatherContext ---
            (S::GatherContext, ModelRequestedTools { count }) => {
                if self.budget.tools_exhausted() {
                    return Ok(S::Failed { reason: FailReason::BudgetExhausted });
                }
                self.pending_tool_count = count;
                Ok(S::Execute)
            }
            (S::GatherContext, ModelClaimedDone) => {
                // Read-only path: context alone may satisfy acceptance.
                self.evidence.model_claimed_done = true;
                self.try_finish(S::GatherContext)
            }
            (S::GatherContext, ToolsFinished { results }) => {
                self.absorb(results);
                self.after_tools(S::GatherContext)
            }

            // --- Execute ---
            (S::Execute, ModelRequestedTools { count }) => {
                if self.budget.tools_exhausted() {
                    return Ok(S::Failed { reason: FailReason::BudgetExhausted });
                }
                self.pending_tool_count = count;
                Ok(S::Execute)
            }
            (S::Execute, ToolsFinished { results }) => {
                self.absorb(results);
                self.after_tools(S::Execute)
            }
            (S::Execute, ModelClaimedDone) => {
                self.evidence.model_claimed_done = true;
                self.try_finish(S::Execute)
            }
            (S::Execute, VerifyFinished { ok }) => {
                // Direct verify without explicit Verify state (loop shortcut).
                self.evidence.verify_ok = Some(ok);
                if ok {
                    self.plan.mark_verify_done();
                    self.try_finish(S::Execute)
                } else {
                    self.begin_repair(S::Execute)
                }
            }

            // --- Verify ---
            (S::Verify, VerifyFinished { ok }) => {
                self.evidence.verify_ok = Some(ok);
                if ok {
                    self.plan.mark_verify_done();
                    self.try_finish(S::Verify)
                } else {
                    self.begin_repair(S::Verify)
                }
            }
            (S::Verify, ModelRequestedTools { .. }) => {
                // Unexpected mid-verify tools — allow but stay.
                Ok(S::Verify)
            }
            (S::Verify, ToolsFinished { results }) => {
                self.absorb(results);
                Ok(S::Verify)
            }
            (S::Verify, ModelClaimedDone) => {
                // Claim while still needing verify → not Finish.
                self.evidence.model_claimed_done = true;
                Err("cannot Finish from Verify before VerifyFinished".to_owned())
            }

            // --- Repair ---
            (S::Repair, RepairApplied) => Ok(S::Execute),
            (S::Repair, ModelRequestedTools { count }) => {
                self.pending_tool_count = count;
                Ok(S::Execute)
            }
            (S::Repair, ToolsFinished { results }) => {
                self.absorb(results);
                Ok(S::Repair)
            }
            (S::Repair, ModelClaimedDone) => {
                self.evidence.model_claimed_done = true;
                Err("cannot Finish from Repair; re-verify first".to_owned())
            }
            (S::Repair, VerifyFinished { ok }) => {
                self.evidence.verify_ok = Some(ok);
                if ok {
                    self.plan.mark_verify_done();
                    self.try_finish(S::Repair)
                } else {
                    self.begin_repair(S::Repair)
                }
            }

            // --- Budget / explicit fail ---
            (_, BudgetExceeded) => Ok(S::Failed { reason: FailReason::BudgetExhausted }),

            // --- Terminal-adjacent rejections ---
            (S::Finish | S::Failed { .. } | S::Cancelled, _) => {
                unreachable!("terminal handled above")
            }

            (state, event) => Err(format!("illegal transition {:?} + {:?}", state.name(), event)),
        }
    }

    fn absorb(&mut self, results: Vec<ToolResult>) {
        self.budget.tool_calls_used = self
            .budget
            .tool_calls_used
            .saturating_add(results.len())
            .max(self.budget.tool_calls_used);
        // Charge at least pending_tool_count if results shorter (rejections collapsed).
        if results.is_empty() && self.pending_tool_count > 0 {
            self.budget.tool_calls_used =
                self.budget.tool_calls_used.saturating_add(self.pending_tool_count);
        }
        self.plan.absorb_tool_results(&results);
        self.evidence.absorb_results(&results);
        self.pending_tool_count = 0;
    }

    fn after_tools(&mut self, from: AgentState) -> Result<AgentState, String> {
        // Tool denial → replan if repairs remain, else fail safely.
        if !self.evidence.denied_tools.is_empty() {
            if !self.budget.repairs_exhausted() {
                self.budget.repairs_used += 1;
                // Back to Plan so the loop can re-plan around the denial.
                return Ok(AgentState::Plan);
            }
            return Ok(AgentState::Failed { reason: FailReason::Unrecoverable });
        }

        if self.plan.requires_verify && self.evidence.verify_ok != Some(true) {
            // Enter Verify once there is something to check (or a prior fail
            // still needs a clean re-run after edits).
            if !self.evidence.files_written.is_empty() {
                return Ok(AgentState::Verify);
            }
            return Ok(from);
        }

        if self.evidence.verify_ok == Some(true) {
            self.plan.mark_verify_done();
            return self.try_finish(from);
        }

        // Stay in Execute for more model turns unless budget says otherwise.
        Ok(from)
    }

    fn try_finish(&mut self, from: AgentState) -> Result<AgentState, String> {
        self.budget.rounds_used = self.budget.rounds_used.saturating_add(1);
        let report = self.plan.evaluate_acceptance(&self.evidence);
        self.last_acceptance = Some(report.clone());
        if report.ok {
            Ok(AgentState::Finish)
        } else if self.budget.rounds_exhausted() {
            Ok(AgentState::Failed { reason: FailReason::BudgetExhausted })
        } else {
            // Not enough evidence — stay so the loop can gather more.
            // If we were forced to finish-like claim, caller may BudgetExceeded.
            let _ = from;
            Ok(from)
        }
    }

    fn begin_repair(&mut self, from: AgentState) -> Result<AgentState, String> {
        if self.budget.repairs_exhausted() {
            return Ok(AgentState::Failed { reason: FailReason::BudgetExhausted });
        }
        self.budget.repairs_used += 1;
        let _ = from;
        Ok(AgentState::Repair)
    }
}

fn evidence_summary(e: &AcceptanceEvidence) -> String {
    format!(
        "writes {} · verify {} · denied {}",
        e.files_written.len(),
        match e.verify_ok {
            Some(true) => "pass",
            Some(false) => "fail",
            None => "—",
        },
        e.denied_tools.len()
    )
}

// ---------------------------------------------------------------------------
// Tests: fake provider + fake tools, deterministic — no network.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ToolArgs, ToolCall, ToolCallId, ToolError, ToolName, ToolRegistry};
    use std::collections::VecDeque;

    /// Scripted model replies (JSON tool protocol or final prose).
    enum FakeReply {
        Tools(serde_json::Value),
        Plan(serde_json::Value),
        Final(String),
    }

    struct FakeProvider {
        script: VecDeque<FakeReply>,
    }

    impl FakeProvider {
        fn new(script: Vec<FakeReply>) -> Self {
            Self { script: script.into() }
        }

        fn next(&mut self) -> FakeReply {
            self.script
                .pop_front()
                .unwrap_or(FakeReply::Final("done".into()))
        }
    }

    /// Fake tools: map tool name → fixed result behavior.
    struct FakeTools {
        write_ok: bool,
        verify_ok: bool,
        deny_write: bool,
        registry: ToolRegistry,
    }

    impl FakeTools {
        fn ok() -> Self {
            Self {
                write_ok: true,
                verify_ok: true,
                deny_write: false,
                registry: ToolRegistry::standard(),
            }
        }

        fn verify_fails() -> Self {
            Self { verify_ok: false, ..Self::ok() }
        }

        fn deny_writes() -> Self {
            Self { deny_write: true, ..Self::ok() }
        }

        fn execute(&self, call: &ToolCall) -> ToolResult {
            match (&call.name, &call.args) {
                (ToolName::WriteFile, ToolArgs::WriteFile { path, .. }) => {
                    if self.deny_write {
                        ToolResult::failure(
                            call.id.clone(),
                            ToolName::WriteFile.label(),
                            path.clone(),
                            ToolError::permission_denied("denied by fake"),
                        )
                    } else if self.write_ok {
                        ToolResult::success(
                            call.id.clone(),
                            ToolName::WriteFile.label(),
                            path.clone(),
                            "wrote",
                        )
                    } else {
                        ToolResult::failure(
                            call.id.clone(),
                            ToolName::WriteFile.label(),
                            path.clone(),
                            ToolError::execution("disk full"),
                        )
                    }
                }
                (ToolName::RunCommand, ToolArgs::RunCommand { command }) => {
                    if self.verify_ok {
                        ToolResult::success(
                            call.id.clone(),
                            ToolName::RunCommand.label(),
                            command.clone(),
                            "tests passed",
                        )
                    } else {
                        ToolResult::failure(
                            call.id.clone(),
                            ToolName::RunCommand.label(),
                            command.clone(),
                            ToolError::execution("test failed"),
                        )
                    }
                }
                (name, args) => ToolResult::success(
                    call.id.clone(),
                    name.label(),
                    args.label(),
                    "ok",
                ),
            }
        }
    }

    fn parse_tools_json(
        json: &serde_json::Value,
    ) -> Option<Vec<crate::protocol::ToolInvocation>> {
        let text = json.to_string();
        let turn = crate::protocol::parse_model_turn(&text, &ToolRegistry::standard());
        match turn {
            crate::protocol::ModelTurn::Tools { calls } if !calls.is_empty() => Some(calls),
            _ => None,
        }
    }

    fn tools_json(calls: serde_json::Value) -> FakeReply {
        FakeReply::Tools(serde_json::json!({ "tool_calls": calls }))
    }

    // ---- Required evaluation scenarios ----

    #[test]
    fn simple_read_only_task_finishes_directly() {
        let mut m = AgentMachine::new(
            "Summarize the repository layout for me",
            Budget::default(),
        );
        m.handle(AgentEvent::TaskReceived);
        m.handle(AgentEvent::PlanReady);
        m.handle(AgentEvent::ContextGathered);
        // Simulate successful pre-scan tool results (fake tools).
        m.handle(AgentEvent::ToolsFinished {
            results: vec![ToolResult::success(
                ToolCallId::new("pre"),
                ToolName::Search.label(),
                "src",
                "found files",
            )],
        });
        assert_eq!(m.state(), &AgentState::GatherContext, "still gathering");

        // Fake provider final reply — claim only; acceptance is structural.
        m.handle(AgentEvent::ModelClaimedDone);
        assert_eq!(
            m.state(),
            &AgentState::Finish,
            "read-only should Finish, plan={:?} acc={:?}",
            m.plan(),
            m.last_acceptance()
        );
    }

    #[test]
    fn edit_task_execute_verify_finish() {
        let mut m = AgentMachine::new("Please update README with badges", Budget::default());
        let mut p = FakeProvider::new(vec![tools_json(serde_json::json!([
            {"id":"1","name":"write_file","arguments":{"path":"README.md","content":"# x\n"}}
        ]))]);
        let tools = FakeTools::ok();

        m.handle(AgentEvent::TaskReceived);
        m.handle(AgentEvent::PlanReady);
        m.handle(AgentEvent::ContextGathered);
        assert_eq!(m.state(), &AgentState::GatherContext);

        // Model asks tools → Execute
        let reply = p.next();
        let FakeReply::Tools(json) = reply else { panic!("expected tools") };
        let calls = parse_tools_json(&json).expect("calls");
        m.handle(AgentEvent::ModelRequestedTools { count: calls.len() });
        assert_eq!(m.state(), &AgentState::Execute);

        let results: Vec<ToolResult> = calls
            .iter()
            .map(|inv| match inv {
                crate::protocol::ToolInvocation::Ready(c) => tools.execute(c),
                crate::protocol::ToolInvocation::Rejected(r) => ToolResult::from_rejection(r),
            })
            .collect();
        m.handle(AgentEvent::ToolsFinished { results });
        assert_eq!(m.state(), &AgentState::Verify, "writes require verify");

        m.handle(AgentEvent::VerifyFinished { ok: true });
        assert_eq!(m.state(), &AgentState::Finish, "failures={:?}", m.last_acceptance());
        assert!(m.last_acceptance().map(|r| r.ok).unwrap_or(false));
    }

    #[test]
    fn failed_tests_enter_repair() {
        let mut m = AgentMachine::new("Please fix the failing unit tests", Budget::default());
        m.handle(AgentEvent::TaskReceived);
        m.handle(AgentEvent::PlanReady);
        m.handle(AgentEvent::ContextGathered);
        m.handle(AgentEvent::ModelRequestedTools { count: 1 });
        m.handle(AgentEvent::ToolsFinished {
            results: vec![ToolResult::success(
                ToolCallId::new("w"),
                ToolName::WriteFile.label(),
                "src/lib.rs",
                "wrote",
            )],
        });
        assert_eq!(m.state(), &AgentState::Verify);

        m.handle(AgentEvent::VerifyFinished { ok: false });
        assert_eq!(m.state(), &AgentState::Repair);
        assert_eq!(m.budget().repairs_used, 1);

        // Repair → Execute
        m.handle(AgentEvent::RepairApplied);
        assert_eq!(m.state(), &AgentState::Execute);
    }

    #[test]
    fn repair_limit_yields_failed_budget() {
        let budget = Budget { max_repairs: 2, ..Budget::default() };
        let mut m = AgentMachine::new("Please update the README file", budget);
        m.handle(AgentEvent::TaskReceived);
        m.handle(AgentEvent::PlanReady);
        m.handle(AgentEvent::ContextGathered);

        for _ in 0..8 {
            if m.state().is_terminal() {
                break;
            }
            match m.state().clone() {
                AgentState::Repair => {
                    m.handle(AgentEvent::RepairApplied);
                }
                AgentState::Verify => {
                    m.handle(AgentEvent::VerifyFinished { ok: false });
                }
                AgentState::GatherContext | AgentState::Execute | AgentState::Plan => {
                    if m.state() == &AgentState::Plan {
                        m.handle(AgentEvent::ContextGathered);
                    }
                    m.handle(AgentEvent::ModelRequestedTools { count: 1 });
                    m.handle(AgentEvent::ToolsFinished {
                        results: vec![ToolResult::success(
                            ToolCallId::new("w"),
                            ToolName::WriteFile.label(),
                            "README.md",
                            "wrote",
                        )],
                    });
                }
                _ => break,
            }
        }

        assert_eq!(
            m.state(),
            &AgentState::Failed { reason: FailReason::BudgetExhausted },
            "state={:?} repairs={}",
            m.state(),
            m.budget().repairs_used
        );
        assert!(m.budget().repairs_used >= 2);
    }

    #[test]
    fn user_stop_cancels_from_any_active_state() {
        for setup in [
            AgentState::Understand,
            AgentState::Plan,
            AgentState::GatherContext,
            AgentState::Execute,
            AgentState::Verify,
            AgentState::Repair,
        ] {
            // Editing task so Verify/Repair setups are reachable.
            let mut m = AgentMachine::new("Please update the file", Budget::default());
            match setup {
                AgentState::Understand => {}
                AgentState::Plan => {
                    m.handle(AgentEvent::TaskReceived);
                    m.handle(AgentEvent::PlanReady);
                }
                AgentState::GatherContext => {
                    m.handle(AgentEvent::TaskReceived);
                    m.handle(AgentEvent::PlanReady);
                    m.handle(AgentEvent::ContextGathered);
                }
                AgentState::Execute => {
                    m.handle(AgentEvent::TaskReceived);
                    m.handle(AgentEvent::PlanReady);
                    m.handle(AgentEvent::ContextGathered);
                    m.handle(AgentEvent::ModelRequestedTools { count: 1 });
                }
                AgentState::Verify => {
                    m.handle(AgentEvent::TaskReceived);
                    m.handle(AgentEvent::PlanReady);
                    m.handle(AgentEvent::ContextGathered);
                    m.handle(AgentEvent::ModelRequestedTools { count: 1 });
                    m.handle(AgentEvent::ToolsFinished {
                        results: vec![ToolResult::success(
                            ToolCallId::new("w"),
                            ToolName::WriteFile.label(),
                            "a.md",
                            "x",
                        )],
                    });
                }
                AgentState::Repair => {
                    m.handle(AgentEvent::TaskReceived);
                    m.handle(AgentEvent::PlanReady);
                    m.handle(AgentEvent::ContextGathered);
                    m.handle(AgentEvent::ModelRequestedTools { count: 1 });
                    m.handle(AgentEvent::ToolsFinished {
                        results: vec![ToolResult::success(
                            ToolCallId::new("w"),
                            ToolName::WriteFile.label(),
                            "a.md",
                            "x",
                        )],
                    });
                    m.handle(AgentEvent::VerifyFinished { ok: false });
                }
                _ => unreachable!(),
            }
            assert_eq!(m.state(), &setup, "setup reached for {setup:?}");
            let t = m.handle(AgentEvent::Cancel);
            assert!(
                matches!(t, Transition::Advanced { to: AgentState::Cancelled, .. }),
                "cancel from {setup:?} → {t:?}"
            );
            assert_eq!(m.state(), &AgentState::Cancelled);
            // Terminal: further events ignored safely
            assert!(matches!(
                m.handle(AgentEvent::ModelClaimedDone),
                Transition::AlreadyTerminal { .. }
            ));
        }
    }

    #[test]
    fn tool_denial_triggers_replan_not_fake_success() {
        let mut m = AgentMachine::new("Please update README", Budget::default());
        m.handle(AgentEvent::TaskReceived);
        m.handle(AgentEvent::PlanReady);
        m.handle(AgentEvent::ContextGathered);
        m.handle(AgentEvent::ModelRequestedTools { count: 1 });
        m.handle(AgentEvent::ToolsFinished {
            results: vec![ToolResult::failure(
                ToolCallId::new("d"),
                ToolName::WriteFile.label(),
                "README.md",
                ToolError::permission_denied("user denied"),
            )],
        });

        assert_eq!(m.state(), &AgentState::Plan, "denial → re-plan");
        assert_eq!(m.budget().repairs_used, 1);
        assert_ne!(m.state(), &AgentState::Finish);

        // Burn remaining repairs on further denials → Failed.
        for _ in 0..5 {
            if m.state().is_terminal() {
                break;
            }
            if m.state() == &AgentState::Plan {
                m.handle(AgentEvent::ContextGathered);
            }
            m.handle(AgentEvent::ModelRequestedTools { count: 1 });
            m.handle(AgentEvent::ToolsFinished {
                results: vec![ToolResult::failure(
                    ToolCallId::new("d2"),
                    ToolName::WriteFile.label(),
                    "README.md",
                    ToolError::permission_denied("user denied"),
                )],
            });
        }
        assert_eq!(m.state(), &AgentState::Failed { reason: FailReason::Unrecoverable });
    }

    #[test]
    fn model_cannot_finish_without_acceptance_evidence() {
        let mut m = AgentMachine::new("Please update README with badges", Budget::default());
        m.handle(AgentEvent::TaskReceived);
        m.handle(AgentEvent::PlanReady);
        m.handle(AgentEvent::ContextGathered);
        // No writes, no verify — model claims done.
        m.handle(AgentEvent::ModelClaimedDone);
        assert_ne!(m.state(), &AgentState::Finish, "claim without evidence must not Finish");
    }

    #[test]
    fn tool_budget_exhaustion_fails_explicitly() {
        let budget = Budget { max_tool_calls: 2, ..Budget::default() };
        let mut m = AgentMachine::new("touch many files", budget);
        m.handle(AgentEvent::TaskReceived);
        m.handle(AgentEvent::PlanReady);
        m.handle(AgentEvent::ContextGathered);
        m.handle(AgentEvent::ModelRequestedTools { count: 2 });
        let t = m.handle(AgentEvent::ToolsFinished {
            results: vec![
                ToolResult::success(ToolCallId::new("1"), ToolName::ReadFile.label(), "a", "x"),
                ToolResult::success(ToolCallId::new("2"), ToolName::ReadFile.label(), "b", "y"),
            ],
        });
        assert!(
            matches!(
                t,
                Transition::Advanced {
                    to: AgentState::Failed { reason: FailReason::BudgetExhausted },
                    ..
                }
            ),
            "got {t:?}"
        );
        assert_eq!(m.state(), &AgentState::Failed { reason: FailReason::BudgetExhausted });
    }

    #[test]
    fn round_budget_exhaustion_fails_explicitly() {
        let budget = Budget { max_rounds: 0, ..Budget::default() };
        let mut m = AgentMachine::new("read only", budget);
        m.handle(AgentEvent::TaskReceived);
        m.handle(AgentEvent::PlanReady);
        m.handle(AgentEvent::ContextGathered);
        m.handle(AgentEvent::ToolsFinished {
            results: vec![ToolResult::success(
                ToolCallId::new("r"),
                ToolName::Search.label(),
                "x",
                "y",
            )],
        });
        // rounds_used is 0; max 0 → any_exhausted after absorb? rounds_used still 0, max 0 → 0>=0 true
        assert_eq!(
            m.state(),
            &AgentState::Failed { reason: FailReason::BudgetExhausted }
        );
    }

    #[test]
    fn illegal_transition_is_rejected_not_panicking() {
        let mut m = AgentMachine::new("t", Budget::default());
        // Cannot ContextGathered from Understand
        let t = m.handle(AgentEvent::ContextGathered);
        assert!(matches!(t, Transition::Rejected { .. }), "{t:?}");
        assert_eq!(m.state(), &AgentState::Understand);
    }

    #[test]
    fn progress_summary_exposes_plan_without_cot() {
        let m = AgentMachine::new("update the docs", Budget::default());
        let s = m.progress_summary();
        assert!(s.contains("Understand"));
        assert!(s.contains("subtasks"));
        assert!(!s.contains('\n'));
        // No pseudo-CoT markers
        assert!(!s.to_lowercase().contains("chain of thought"));
        assert!(!s.contains("thinking:"));
    }

    #[test]
    fn full_edit_happy_path_with_fake_provider_and_tools() {
        let mut m = AgentMachine::new("Please update README and run tests", Budget::default());
        let mut p = FakeProvider::new(vec![tools_json(serde_json::json!([
            {"id":"w1","name":"write_file","arguments":{"path":"README.md","content":"ok\n"}}
        ]))]);
        let tools = FakeTools::ok();

        m.handle(AgentEvent::TaskReceived);
        m.handle(AgentEvent::PlanReady);
        m.handle(AgentEvent::ContextGathered);

        let reply = p.next();
        let FakeReply::Tools(json) = reply else { panic!() };
        let calls = parse_tools_json(&json).unwrap();
        m.handle(AgentEvent::ModelRequestedTools { count: calls.len() });
        let results: Vec<ToolResult> = calls
            .iter()
            .map(|inv| match inv {
                crate::protocol::ToolInvocation::Ready(c) => tools.execute(c),
                crate::protocol::ToolInvocation::Rejected(r) => ToolResult::from_rejection(r),
            })
            .collect();
        m.handle(AgentEvent::ToolsFinished { results });
        assert_eq!(m.state(), &AgentState::Verify);
        m.handle(AgentEvent::VerifyFinished { ok: tools.verify_ok });
        assert_eq!(m.state(), &AgentState::Finish);
    }

    #[test]
    fn failed_verify_with_fake_tools_then_repair_then_success() {
        let mut m = AgentMachine::new("fix tests", Budget::default());
        m.handle(AgentEvent::TaskReceived);
        m.handle(AgentEvent::PlanReady);
        m.handle(AgentEvent::ContextGathered);
        m.handle(AgentEvent::ModelRequestedTools { count: 1 });
        m.handle(AgentEvent::ToolsFinished {
            results: vec![ToolResult::success(
                ToolCallId::new("w"),
                ToolName::WriteFile.label(),
                "lib.rs",
                "x",
            )],
        });
        let fail_tools = FakeTools::verify_fails();
        assert!(!fail_tools.verify_ok);
        m.handle(AgentEvent::VerifyFinished { ok: false });
        assert_eq!(m.state(), &AgentState::Repair);
        m.handle(AgentEvent::RepairApplied);
        m.handle(AgentEvent::ModelRequestedTools { count: 1 });
        m.handle(AgentEvent::ToolsFinished {
            results: vec![ToolResult::success(
                ToolCallId::new("w2"),
                ToolName::WriteFile.label(),
                "lib.rs",
                "y",
            )],
        });
        m.handle(AgentEvent::VerifyFinished { ok: true });
        assert_eq!(m.state(), &AgentState::Finish);
    }
}
