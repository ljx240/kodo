//! Structured task plan the model and state machine share.
//!
//! A plan is **evidence-driven**: subtasks flip to `Done` only from observed
//! [`ToolResult`]s matched against a per-subtask [`EvidenceRequirement`].
//! Acceptance criteria are structured objects; free-text model claims never
//! complete them.

use crate::evidence::{
    evidence_from_tool_result, AcceptanceCriterion, CommandExpectation, EvidenceBag, EvidenceItem,
    EvidenceKind, EvidenceRequirement, SemanticTarget, SubtaskRequirement,
};
use crate::protocol::{ToolErrorCode, ToolName, ToolResult};
use crate::skill::SkillSpec;

/// Kind of work a subtask represents — drives how evidence marks it done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtaskKind {
    /// Search / read / inspect.
    Read,
    /// write_file or other mutation.
    Edit,
    /// Run a command (tests, git, …).
    Command,
    /// Explicit verification gate.
    Verify,
}

/// Lifecycle of one subtask. `Done` requires non-empty evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtaskStatus {
    Pending,
    Running,
    /// Blocked by permission denial / skill gate — may resume later.
    Blocked,
    Done,
    /// Last attempt failed (non-permission error); retryable.
    Failed,
}

impl SubtaskStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    pub fn is_done(self) -> bool {
        matches!(self, Self::Done)
    }
}

/// Structural proof attached to a subtask. Never model prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    /// A successful (or gating) tool result id.
    ToolResultId { id: String, tool: String },
    /// A file the project actually changed.
    ChangedFile { path: String },
    /// A verification run that reported success.
    PassedVerification { command: String },
    /// Typed criterion-specific evidence bag entry.
    Typed(crate::evidence::EvidenceItem),
}

impl Evidence {
    pub fn describe(&self) -> String {
        match self {
            Self::ToolResultId { id, tool } => format!("tool:{tool}#{id}"),
            Self::ChangedFile { path } => format!("file:{path}"),
            Self::PassedVerification { command } => format!("verify:{command}"),
            Self::Typed(item) => format!("typed:{}", item.describe()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subtask {
    pub id: String,
    pub title: String,
    pub kind: SubtaskKind,
    pub status: SubtaskStatus,
    pub evidence: Vec<Evidence>,
    /// What this subtask actually requires — prevents arbitrary Read completion.
    pub requirement: Option<SubtaskRequirement>,
}

impl Subtask {
    pub fn new(id: impl Into<String>, title: impl Into<String>, kind: SubtaskKind) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind,
            status: SubtaskStatus::Pending,
            evidence: Vec::new(),
            requirement: None,
        }
    }

    pub fn with_requirement(mut self, requirement: SubtaskRequirement) -> Self {
        self.requirement = Some(requirement);
        self
    }

    /// Default requirement when none was assigned — kind-aware, still strict
    /// against pre-scan context for non-Read work.
    pub fn effective_requirement(&self) -> SubtaskRequirement {
        if let Some(req) = &self.requirement {
            return req.clone();
        }
        match self.kind {
            SubtaskKind::Read => {
                SubtaskRequirement::contextual(EvidenceRequirement::AnyToolSuccess)
            }
            SubtaskKind::Edit => SubtaskRequirement::strict(EvidenceRequirement::AnyFileChange),
            SubtaskKind::Command => SubtaskRequirement::strict(EvidenceRequirement::AnyToolSuccess),
            SubtaskKind::Verify => {
                SubtaskRequirement::strict(EvidenceRequirement::VerificationPassed)
            }
        }
    }

    pub fn is_done(&self) -> bool {
        self.status.is_done()
    }

    /// Transition to `Done`. Refuses without evidence — `Done` is proof-backed.
    pub fn mark_done(&mut self, evidence: Evidence) -> bool {
        if self.status.is_done() {
            if !self.evidence.contains(&evidence) {
                self.evidence.push(evidence);
            }
            return true;
        }
        self.evidence.push(evidence);
        self.status = SubtaskStatus::Done;
        true
    }

    /// Can this evidence item complete this subtask?
    pub fn accepts(&self, item: &EvidenceItem) -> bool {
        self.effective_requirement().matches(item)
    }
}

/// Plan handed to the model and used for acceptance evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskPlan {
    pub goal: String,
    pub constraints: Vec<String>,
    pub subtasks: Vec<Subtask>,
    pub acceptance_criteria: Vec<String>,
    /// Structured criteria with requirement matchers (source of truth for Finish).
    pub criteria: Vec<AcceptanceCriterion>,
    /// Index into `subtasks` for the active item (len when all done).
    pub current_subtask: usize,
    /// Whether Finish requires a passing verify run.
    pub requires_verify: bool,
    /// Command labels to record as verification evidence on the next
    /// `mark_verify_done` (set by the loop from the real runner outcomes).
    pub verify_commands: Vec<String>,
}

impl TaskPlan {
    /// Heuristic plan when no model planner is available (still structured).
    pub fn from_task(message: &str) -> Self {
        let goal = message
            .trim()
            .lines()
            .next()
            .unwrap_or("task")
            .trim()
            .to_owned();
        let lower = message.to_ascii_lowercase();
        let mutating = [
            "write", "edit", "fix", "create", "update", "修改", "修复", "创建", "更新", "添加",
        ]
        .iter()
        .any(|k| lower.contains(k));
        // Narrow bug-fix detection: "fix tests" / "fix docs" are edit tasks,
        // not bug reproductions.
        let looks_like_bug = lower.contains("bug")
            || lower.contains("panic")
            || lower.contains("crash")
            || lower.contains("报错")
            || lower.contains("失败")
            || lower.contains("exception")
            || lower.contains("500")
            || (lower.contains("fix")
                && !lower.contains("fix tests")
                && !lower.contains("fix test")
                && !lower.contains("fix docs")
                && !lower.contains("fix docs"));

        let mut subtasks = vec![
            Subtask::new("s1", "Gather project context", SubtaskKind::Read).with_requirement(
                SubtaskRequirement::contextual(EvidenceRequirement::AnyToolSuccess),
            ),
        ];
        if mutating {
            if looks_like_bug {
                subtasks.push(
                    Subtask::new(
                        "s1b",
                        "Reproduce the failure with a failing test or command",
                        SubtaskKind::Command,
                    )
                    .with_requirement(SubtaskRequirement::strict(
                        EvidenceRequirement::CommandOutcome {
                            command_contains: String::new(),
                            expectation: CommandExpectation::ExpectedFailure,
                        },
                    )),
                );
                subtasks.push(
                    Subtask::new(
                        "s2",
                        "Locate the root cause in the source",
                        SubtaskKind::Read,
                    )
                    .with_requirement(SubtaskRequirement::strict(
                        EvidenceRequirement::SemanticProof {
                            target: SemanticTarget::RootCause,
                        },
                    )),
                );
            }
            subtasks.push(
                Subtask::new("s3", "Apply the requested changes", SubtaskKind::Edit)
                    .with_requirement(SubtaskRequirement::strict(
                        EvidenceRequirement::AnyFileChange,
                    )),
            );
            subtasks.push(
                Subtask::new("s4", "Verify the project still holds", SubtaskKind::Verify)
                    .with_requirement(SubtaskRequirement::strict(
                        EvidenceRequirement::VerificationPassed,
                    )),
            );
        }

        let mut acceptance_criteria = vec!["Context was gathered from the live project".to_owned()];
        let mut criteria = vec![AcceptanceCriterion::new(
            "c1",
            "Context was gathered from the live project",
            EvidenceRequirement::ContextRead {
                path_contains: String::new(),
            },
        )];
        if mutating {
            acceptance_criteria.push("Requested files were written inside the project".to_owned());
            acceptance_criteria.push("Verification command completed successfully".to_owned());
            criteria.push(AcceptanceCriterion::new(
                "c2",
                "Requested files were written inside the project",
                EvidenceRequirement::AnyFileChange,
            ));
            criteria.push(AcceptanceCriterion::new(
                "c3",
                "Verification command completed successfully",
                EvidenceRequirement::VerificationPassed,
            ));
        } else {
            acceptance_criteria.push("Answer is grounded in gathered tool observations".to_owned());
            criteria.push(AcceptanceCriterion::new(
                "c2",
                "Answer is grounded in gathered tool observations",
                EvidenceRequirement::AnyToolSuccess,
            ));
        }

        Self {
            goal,
            constraints: vec![
                "Paths stay inside the project".to_owned(),
                "No hidden chain-of-thought is stored".to_owned(),
            ],
            subtasks,
            acceptance_criteria,
            criteria,
            current_subtask: 0,
            requires_verify: mutating,
            verify_commands: Vec::new(),
        }
    }

    /// Skill-driven plan: workflow → subtasks with requirements, completion
    /// criteria → structured acceptance criteria.
    pub fn from_skill(skill: &SkillSpec, message: &str) -> Self {
        let mut plan = Self::from_task(message);
        if !skill.workflow.is_empty() {
            plan.subtasks = skill
                .workflow
                .iter()
                .map(|w| {
                    let requirement = requirement_for_skill_step(&w.title, w.kind);
                    Subtask::new(w.id.clone(), w.title.clone(), w.kind)
                        .with_requirement(requirement)
                })
                .collect();
            plan.current_subtask = 0;
        }
        if !skill.completion_criteria.is_empty() {
            plan.acceptance_criteria = skill.completion_criteria.clone();
            plan.criteria = skill
                .completion_criteria
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    AcceptanceCriterion::new(
                        format!("skill_c{}", i + 1),
                        c.clone(),
                        infer_criterion_req(c),
                    )
                })
                .collect();
        }
        plan.requires_verify = skill.verification_policy.needs_run();
        plan.constraints = vec![
            "Paths stay inside the project".to_owned(),
            "No hidden chain-of-thought is stored".to_owned(),
            format!(
                "Skill `{}` rules apply; they never bypass Permission approval",
                skill.name
            ),
            "Only tool/verification evidence can mark work done".to_owned(),
        ];
        plan
    }

    /// Attempt to parse a model-provided plan JSON. Falls back to `None` on bad input.
    pub fn parse_model_json(text: &str) -> Option<Self> {
        let body = text.trim();
        let body = body
            .strip_prefix("```json")
            .or_else(|| body.strip_prefix("```"))
            .unwrap_or(body)
            .trim()
            .strip_suffix("```")
            .unwrap_or(body)
            .trim();
        let value: serde_json::Value = serde_json::from_str(body).ok()?;
        let goal = value.get("goal")?.as_str()?.to_owned();
        let constraints = string_list(value.get("constraints"));
        let acceptance_criteria = string_list(value.get("acceptance_criteria"));
        if acceptance_criteria.is_empty() {
            return None;
        }
        let mut subtasks = Vec::new();
        for (index, item) in value.get("subtasks")?.as_array()?.iter().enumerate() {
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .or_else(|| item.as_str())?
                .to_owned();
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("s{}", index + 1));
            let kind = match item.get("kind").and_then(|v| v.as_str()).unwrap_or("") {
                "edit" => SubtaskKind::Edit,
                "command" => SubtaskKind::Command,
                "verify" => SubtaskKind::Verify,
                _ => SubtaskKind::Read,
            };
            let requirement = item
                .get("requirement")
                .and_then(|v| v.as_str())
                .map(requirement_from_label)
                .or_else(|| Some(requirement_for_skill_step(&title, kind)))
                .unwrap();
            subtasks.push(Subtask::new(id, title, kind).with_requirement(requirement));
        }
        if subtasks.is_empty() {
            return None;
        }
        let requires_verify = value
            .get("requires_verify")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| {
                subtasks
                    .iter()
                    .any(|s| matches!(s.kind, SubtaskKind::Verify | SubtaskKind::Edit))
            });
        let current_subtask = value
            .get("current_subtask")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let subtask_count = subtasks.len();
        let criteria = acceptance_criteria
            .iter()
            .enumerate()
            .map(|(i, c)| {
                AcceptanceCriterion::new(format!("m_c{}", i + 1), c.clone(), infer_criterion_req(c))
            })
            .collect();
        Some(Self {
            goal,
            constraints,
            subtasks,
            acceptance_criteria,
            criteria,
            current_subtask: current_subtask.min(subtask_count),
            requires_verify,
            verify_commands: Vec::new(),
        })
    }

    /// Prompt fragment: structured plan for the model (no hidden reasoning).
    pub fn to_prompt_block(&self) -> String {
        let mut out = String::from("## Current plan\n");
        out.push_str(&format!("- goal: {}\n", self.goal));
        for c in &self.constraints {
            out.push_str(&format!("- constraint: {c}\n"));
        }
        out.push_str("- subtasks:\n");
        for (i, s) in self.subtasks.iter().enumerate() {
            let mark = if s.is_done() {
                "x"
            } else if i == self.current_subtask {
                ">"
            } else {
                " "
            };
            out.push_str(&format!(
                "  [{mark}] {} · {:?} · {} · {}\n",
                s.id,
                s.kind,
                s.status.label(),
                s.title
            ));
            if s.is_done() {
                for ev in &s.evidence {
                    out.push_str(&format!("      evidence: {}\n", ev.describe()));
                }
            }
        }
        out.push_str("- acceptance_criteria:\n");
        for c in &self.criteria {
            let mark = if c.is_met() { "x" } else { " " };
            out.push_str(&format!(
                "  [{mark}] {} · {} · need={}\n",
                c.id,
                c.description,
                c.requirement.label()
            ));
        }
        if self.criteria.is_empty() {
            for c in &self.acceptance_criteria {
                out.push_str(&format!("  - {c}\n"));
            }
        }
        out.push_str(&format!("- current_subtask: {}\n", self.current_subtask));
        out
    }

    /// Structured, non-CoT progress for UI.
    pub fn progress_payload(&self, state_name: &str, active: Option<&str>) -> String {
        let total = self.subtasks.len();
        let done = self.subtasks.iter().filter(|s| s.is_done()).count();
        let current = self
            .subtasks
            .get(self.current_subtask)
            .map(|s| s.title.as_str())
            .unwrap_or("—");
        format!(
            "phase={state_name} · subtask={current} · {done}/{total} · active={} · goal={}",
            active.unwrap_or("—"),
            truncate(&self.goal, 48)
        )
    }

    /// Short, non-CoT progress line for Reasoning steps.
    pub fn progress_summary(&self, state_name: &str) -> String {
        let total = self.subtasks.len();
        let done = self.subtasks.iter().filter(|s| s.is_done()).count();
        format!(
            "{state_name} · subtasks {done}/{total} · {}",
            truncate(&self.goal, 60)
        )
    }

    /// Advance the active subtask to `Running` (called when the model requests tools).
    pub fn mark_current_running(&mut self) {
        let idx = self
            .current_subtask
            .min(self.subtasks.len().saturating_sub(1));
        if let Some(sub) = self.subtasks.get_mut(idx) {
            if matches!(
                sub.status,
                SubtaskStatus::Pending | SubtaskStatus::Blocked | SubtaskStatus::Failed
            ) {
                sub.status = SubtaskStatus::Running;
            }
        }
    }

    /// Mark subtasks done from tool evidence. Returns how many newly flipped.
    ///
    /// Only the **first unfinished** subtask of a matching kind flips per
    /// result, so multi-step workflows (reproduce → regression → …) advance
    /// one at a time. Failures set `Blocked` (permission) or `Failed` (error).
    pub fn absorb_tool_results(&mut self, results: &[ToolResult]) -> usize {
        let mut flipped = 0;
        for result in results {
            let expectation = self
                .subtasks
                .iter()
                .find(|s| !s.is_done() && matches!(s.kind, SubtaskKind::Command))
                .and_then(|s| match &s.effective_requirement().evidence {
                    EvidenceRequirement::CommandOutcome { expectation, .. } => {
                        Some(expectation.clone())
                    }
                    _ => None,
                });
            let maybe_item = evidence_from_tool_result(result, expectation.as_ref());
            let kind_for_result = match ToolName::parse(&result.name) {
                Some(ToolName::ReadFile) | Some(ToolName::Search) => Some(SubtaskKind::Read),
                Some(name) if name.is_mutation() => Some(SubtaskKind::Edit),
                Some(ToolName::RunCommand) => Some(SubtaskKind::Command),
                _ => None,
            };

            if let Some(item) = maybe_item.clone() {
                flipped += self.mark_first_accepting(&item);
                if result.ok
                    && ToolName::parse(&result.name)
                        .map(|n| n.is_mutation())
                        .unwrap_or(false)
                {
                    // also record file-change evidence
                }
            } else if result.ok {
                if let Some(kind) = kind_for_result {
                    let synthetic = EvidenceItem::new(
                        format!("ev_{}", result.id),
                        result.name.clone(),
                        match kind {
                            SubtaskKind::Edit => EvidenceKind::FileChanged {
                                path: result.input.clone(),
                            },
                            SubtaskKind::Command => EvidenceKind::CommandSucceeded {
                                command: result.input.clone(),
                                exit_code: 0,
                            },
                            _ => EvidenceKind::FileRead {
                                path: result.input.clone(),
                            },
                        },
                    );
                    flipped += self.mark_first_accepting(&synthetic);
                }
            } else if let Some(kind) = kind_for_result {
                let permission = result
                    .error
                    .as_ref()
                    .map(|e| e.code == ToolErrorCode::PermissionDenied)
                    .unwrap_or(false);
                // Expected-failure evidence (if produced) already counted; do not
                // then mark the same Command subtask Failed.
                let accepted_as_expected = maybe_item
                    .as_ref()
                    .map(|i| {
                        matches!(i.kind, EvidenceKind::CommandFailedAsExpected { .. })
                            && self.subtasks.iter().any(|s| {
                                s.is_done()
                                    && s.evidence
                                        .iter()
                                        .any(|e| matches!(e, Evidence::Typed(t) if t.id == i.id))
                            })
                    })
                    .unwrap_or(false);
                if !accepted_as_expected {
                    self.mark_kind_throttled(
                        kind,
                        if permission {
                            SubtaskStatus::Blocked
                        } else {
                            SubtaskStatus::Failed
                        },
                    );
                }
            }
        }
        self.advance_cursor();
        flipped
    }

    /// Flip the first unfinished subtask whose requirement accepts `item`.
    /// Prefer the most specific matching kind (Edit FileChanged over Read).
    fn mark_first_accepting(&mut self, item: &EvidenceItem) -> usize {
        let specificity = |s: &Subtask| -> u8 {
            let req = s.effective_requirement();
            match &req.evidence {
                EvidenceRequirement::AnyFileChange
                | EvidenceRequirement::FileChanged { .. }
                | EvidenceRequirement::VerificationPassed
                | EvidenceRequirement::CommandOutcome { .. }
                | EvidenceRequirement::ToolSucceeded { .. }
                | EvidenceRequirement::SemanticProof { .. }
                | EvidenceRequirement::RequiresExplicitEvidence => 3,
                EvidenceRequirement::SearchHit { .. }
                | EvidenceRequirement::FileInspected { .. }
                | EvidenceRequirement::DiffReviewed { .. } => 2,
                _ => 1,
            }
        };
        let mut candidates: Vec<usize> = self
            .subtasks
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.is_done() && s.accepts(item))
            .map(|(i, _)| i)
            .collect();
        candidates.sort_by_key(|&i| std::cmp::Reverse(specificity(&self.subtasks[i])));
        let Some(index) = candidates.into_iter().next() else {
            return 0;
        };
        let sub = &mut self.subtasks[index];
        let legacy = match &item.kind {
            EvidenceKind::FileChanged { path } | EvidenceKind::PatchApplied { path } => {
                Evidence::ChangedFile { path: path.clone() }
            }
            EvidenceKind::TestPassed { command }
            | EvidenceKind::BuildPassed { command }
            | EvidenceKind::LintPassed { command } => Evidence::PassedVerification {
                command: command.clone(),
            },
            _ => Evidence::ToolResultId {
                id: item.id.clone(),
                tool: item.source.clone(),
            },
        };
        sub.mark_done(legacy);
        sub.mark_done(Evidence::Typed(item.clone()));
        1
    }

    /// Record that verification succeeded, attaching evidence to Verify subtasks.
    pub fn mark_verify_done(&mut self) {
        let command = if self.verify_commands.is_empty() {
            "project verification".to_owned()
        } else {
            self.verify_commands.join(" && ")
        };
        for sub in &mut self.subtasks {
            if sub.kind == SubtaskKind::Verify && !sub.is_done() {
                sub.mark_done(Evidence::PassedVerification {
                    command: command.clone(),
                });
            }
        }
        for criterion in &mut self.criteria {
            if matches!(
                criterion.requirement,
                EvidenceRequirement::VerificationPassed
                    | EvidenceRequirement::SemanticProof {
                        target: SemanticTarget::RegressionPrevented
                    }
            ) {
                let item = EvidenceItem::new(
                    format!("verify_{command}"),
                    "verify",
                    if command.contains("test") {
                        EvidenceKind::TestPassed {
                            command: command.clone(),
                        }
                    } else if command.contains("lint") || command.contains("clippy") {
                        EvidenceKind::LintPassed {
                            command: command.clone(),
                        }
                    } else {
                        EvidenceKind::BuildPassed {
                            command: command.clone(),
                        }
                    },
                );
                criterion.absorb(&item);
            }
        }
        self.verify_commands.clear();
        self.advance_cursor();
    }

    fn advance_cursor(&mut self) {
        self.current_subtask = self
            .subtasks
            .iter()
            .position(|s| !s.is_done())
            .unwrap_or(self.subtasks.len());
    }

    /// Set status on the first unfinished subtask of `kind` (failure paths).
    fn mark_kind_throttled(&mut self, kind: SubtaskKind, status: SubtaskStatus) {
        if let Some(sub) = self
            .subtasks
            .iter_mut()
            .find(|s| s.kind == kind && !s.is_done() && s.status != SubtaskStatus::Blocked)
        {
            sub.status = status;
        }
    }

    /// Absorb typed evidence into structured criteria.
    pub fn absorb_evidence_item(&mut self, item: &EvidenceItem) {
        for criterion in &mut self.criteria {
            criterion.absorb(item);
        }
        flipped_unused(item);
    }

    /// Completion gate for Finish / `model_claimed_done` evaluation.
    ///
    /// Passes only when **all** hold:
    /// - every required criterion is `Met`
    /// - required verification passed
    /// - no unresolved criterion
    /// - no unresolved critical failure
    ///
    /// `model_claimed_done` never creates evidence — it only triggers this
    /// evaluation. Model prose is never converted into TestPassed/FileChanged.
    pub fn evaluate_acceptance(&self, evidence: &AcceptanceEvidence) -> AcceptanceReport {
        let mut failures = Vec::new();
        let mut passed = Vec::new();

        // Claim flags are evaluation triggers, not observations.
        let _claimed = evidence.model_claimed_done;

        for s in &self.subtasks {
            if s.is_done() && s.evidence.is_empty() {
                failures.push(format!("subtask {} marked done without evidence", s.id));
            }
        }

        if self.requires_verify {
            match evidence.verify_ok {
                None => failures.push("verification has not run".to_owned()),
                Some(false) => failures.push("verification failed".to_owned()),
                Some(true) => passed.push("verification passed".to_owned()),
            }
        } else if evidence.verify_ok == Some(false) {
            // Unresolved critical failure even when the skill skips verify.
            failures.push("verification failed".to_owned());
        }

        // Evaluate structured criteria when present (strict), else legacy strings.
        if !self.criteria.is_empty() {
            for criterion in &self.criteria {
                let mut met = criterion.is_met()
                    || evidence
                        .bag
                        .satisfies_for(Some(&criterion.id), &criterion.requirement)
                    || (matches!(
                        criterion.requirement,
                        EvidenceRequirement::AnyFileChange
                            | EvidenceRequirement::SemanticProof {
                                target: SemanticTarget::BehaviorImplemented
                            }
                    ) && !evidence.files_written.is_empty())
                    || (matches!(criterion.requirement, EvidenceRequirement::AnyToolSuccess)
                        && evidence.had_tool_success)
                    || (matches!(
                        criterion.requirement,
                        EvidenceRequirement::VerificationPassed
                            | EvidenceRequirement::SemanticProof {
                                target: SemanticTarget::RegressionPrevented
                            }
                    ) && evidence.verify_ok == Some(true))
                    || (matches!(
                        criterion.requirement,
                        EvidenceRequirement::ContextRead { .. }
                    ) && evidence.had_tool_success);
                // Unresolved free-form criteria never fall back to tool success.
                if criterion.requirement.is_unresolved() {
                    met = criterion.is_met();
                }
                // Criterion can also be met by subtask-typed evidence (binding-aware).
                if !met {
                    for sub in &self.subtasks {
                        if !sub.is_done() {
                            continue;
                        }
                        for ev in &sub.evidence {
                            if let Evidence::Typed(item) = ev {
                                if criterion.accepts(item) {
                                    met = true;
                                    break;
                                }
                            }
                        }
                        if met {
                            break;
                        }
                    }
                }
                // Expected-failure / semantic reproduction evidence from the bag.
                if !met
                    && matches!(
                        criterion.requirement,
                        EvidenceRequirement::CommandOutcome { .. }
                            | EvidenceRequirement::SemanticProof {
                                target: SemanticTarget::Reproduction
                            }
                    )
                {
                    for item in &evidence.bag.items {
                        if item.binding_allows(&criterion.id) && criterion.requirement.matches(item)
                        {
                            met = true;
                            break;
                        }
                    }
                }
                if met {
                    passed.push(criterion.description.clone());
                } else {
                    let label = if criterion.is_unresolved() {
                        format!(
                            "unresolved criterion {}: {} (need {})",
                            criterion.id,
                            criterion.description,
                            criterion.requirement.label()
                        )
                    } else {
                        format!(
                            "unmet criterion {}: {} (need {})",
                            criterion.id,
                            criterion.description,
                            criterion.requirement.label()
                        )
                    };
                    failures.push(label);
                }
            }
            // Completion gate: no unresolved criterion may open Finish.
            for criterion in &self.criteria {
                if criterion.is_unresolved()
                    && !failures.iter().any(|f| f.contains(criterion.id.as_str()))
                {
                    failures.push(format!(
                        "unresolved criterion {}: free-form evidence required",
                        criterion.id
                    ));
                }
            }
        } else {
            for criterion in &self.acceptance_criteria {
                let ok = evidence.had_tool_success
                    && (!self.requires_verify || evidence.verify_ok == Some(true))
                    && (!self.subtasks.iter().any(|s| s.kind == SubtaskKind::Edit)
                        || !evidence.files_written.is_empty());
                if ok {
                    passed.push(criterion.clone());
                } else {
                    failures.push(format!("unmet criterion: {criterion}"));
                }
            }
        }

        // Unfinished strict subtasks block Finish. Contextual Read may complete
        // via pre-scan observations; Edit/Command/Verify stay evidence-backed.
        // Optional review checks ("optionally run…") never block Finish.
        for sub in self.subtasks.iter().filter(|s| !s.is_done()) {
            let req = sub.effective_requirement();
            let contextual_read = sub.kind == SubtaskKind::Read
                && req.allow_context_as_evidence
                && evidence.had_tool_success;
            if contextual_read {
                continue;
            }
            if sub.kind == SubtaskKind::Verify && evidence.verify_ok == Some(true) {
                continue;
            }
            let optional = sub.title.to_ascii_lowercase().contains("optionally")
                || sub.title.to_ascii_lowercase().contains("optional");
            if optional {
                continue;
            }
            if !failures.iter().any(|f| f.contains(sub.id.as_str())) {
                failures.push(format!("subtask {} unfinished: {}", sub.id, sub.title));
            }
        }

        AcceptanceReport {
            ok: failures.is_empty()
                && !self.acceptance_criteria.is_empty()
                && self
                    .criteria
                    .iter()
                    .all(|c| c.is_met() || !c.is_unresolved()),
            passed,
            failures,
        }
    }
}

fn criterion_could_be_structural(c: &str) -> bool {
    let lower = c.to_ascii_lowercase();
    lower.contains("written")
        || lower.contains("write")
        || lower.contains("verif")
        || lower.contains("test")
        || lower.contains("pass")
        || lower.contains("gather")
        || lower.contains("observ")
        || lower.contains("写入")
        || lower.contains("验证")
        || lower.contains("测试")
}

fn flipped_unused(_item: &EvidenceItem) {}

#[allow(unused)]
fn _unused_criterion_helper(c: &str) -> bool {
    criterion_could_be_structural(c)
}

/// Skill step title/kind → strict evidence requirement.
pub fn requirement_for_skill_step(title: &str, kind: SubtaskKind) -> SubtaskRequirement {
    let lower = title.to_ascii_lowercase();
    if lower.contains("reproduc") || lower.contains("复现") || lower.contains("failing test") {
        // Subtask-level: expectation-aware failure outcome (not a success wildcard).
        return SubtaskRequirement::strict(EvidenceRequirement::CommandOutcome {
            command_contains: String::new(),
            expectation: CommandExpectation::ExpectedFailureSignature {
                signature: "fail".into(),
            },
        });
    }
    // Root cause needs a real search hit — not any FileRead/command.
    if lower.contains("root cause") || lower.contains("定位") {
        return SubtaskRequirement::strict(EvidenceRequirement::SemanticProof {
            target: SemanticTarget::RootCause,
        });
    }
    if lower.contains("verif") || lower.contains("regression") || lower.contains("验证") {
        return SubtaskRequirement::strict(EvidenceRequirement::VerificationPassed);
    }
    if lower.contains("extract") && (lower.contains("criteria") || lower.contains("验收")) {
        // Structured plan JSON refine counts as tool-backed planning evidence.
        // Arbitrary Read alone does not (covered by evidence matchers).
        return SubtaskRequirement::strict(EvidenceRequirement::AnyToolSuccess);
    }
    if lower.contains("review") {
        return SubtaskRequirement::strict(EvidenceRequirement::DiffReviewed {
            path_contains: String::new(),
        });
    }
    if lower.contains("confirm") && (lower.contains("api") || lower.contains("contract")) {
        return SubtaskRequirement::strict(EvidenceRequirement::SearchHit {
            query_contains: String::new(),
        });
    }
    match kind {
        SubtaskKind::Edit => SubtaskRequirement::strict(EvidenceRequirement::AnyFileChange),
        SubtaskKind::Verify => SubtaskRequirement::strict(EvidenceRequirement::VerificationPassed),
        SubtaskKind::Command => SubtaskRequirement::strict(EvidenceRequirement::AnyToolSuccess),
        SubtaskKind::Read => {
            // Pre-scan context is not proof of extraction/root-cause.
            if lower.contains("context") || lower.contains("gather") || lower.contains("scan") {
                SubtaskRequirement::contextual(EvidenceRequirement::AnyToolSuccess)
            } else {
                // Locate/inspect: an explicit file read or search hit — never
                // git status / arbitrary successful commands.
                SubtaskRequirement::strict(EvidenceRequirement::FileInspected {
                    path_contains: String::new(),
                })
            }
        }
    }
}

/// Free-text label from model JSON → requirement.
pub fn requirement_from_label(label: &str) -> SubtaskRequirement {
    requirement_for_skill_step(label, SubtaskKind::Read)
}

/// Free-text acceptance criterion → requirement.
///
/// Unknown free-form text becomes [`EvidenceRequirement::RequiresExplicitEvidence`]
/// (fail closed) — never a silent [`EvidenceRequirement::AnyToolSuccess`] pass.
pub fn infer_criterion_req(description: &str) -> EvidenceRequirement {
    let lower = description.to_ascii_lowercase();
    // Grounded/tool-observation language must win over generic "review".
    if lower.contains("grounded")
        || lower.contains("observ")
        || lower.contains("context")
        || lower.contains("gather")
        || lower.contains("cite")
        || lower.contains("evidence")
        || lower.contains("criteria")
        || lower.contains("acceptance")
        || lower.contains("验收")
    {
        return EvidenceRequirement::AnyToolSuccess;
    }
    if lower.contains("reproduc")
        || lower.contains("failure was reproduced")
        || lower.contains("复现")
    {
        return EvidenceRequirement::SemanticProof {
            target: SemanticTarget::Reproduction,
        };
    }
    if lower.contains("regression") {
        return EvidenceRequirement::SemanticProof {
            target: SemanticTarget::RegressionPrevented,
        };
    }
    if lower.contains("root cause") || lower.contains("定位") {
        return EvidenceRequirement::SemanticProof {
            target: SemanticTarget::RootCause,
        };
    }
    if lower.contains("verif")
        || lower.contains("test")
        || lower.contains("验证")
        || lower.contains("测试")
        || lower.contains("intact")
    {
        return EvidenceRequirement::VerificationPassed;
    }
    // Negation / absence language before write-shaped patterns:
    // "No project files were modified" must not become a change requirement.
    if lower.contains("modified") {
        return EvidenceRequirement::AnyToolSuccess;
    }
    if lower.contains("changed file")
        || lower.contains("writ") // write / written / writing
        || lower.contains("修复")
        || lower.contains("写入")
        || lower.contains("project files")
        || lower.contains("restructuring")
        || lower.contains("behavior implemented")
    {
        return EvidenceRequirement::SemanticProof {
            target: SemanticTarget::BehaviorImplemented,
        };
    }
    if lower.contains("diff") || lower.contains("review") {
        return EvidenceRequirement::DiffReviewed {
            path_contains: String::new(),
        };
    }
    EvidenceRequirement::RequiresExplicitEvidence
}

fn string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Inputs for acceptance evaluation (from tools, never from prose alone).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AcceptanceEvidence {
    pub had_tool_success: bool,
    pub files_written: Vec<String>,
    pub verify_ok: Option<bool>,
    pub model_claimed_done: bool,
    pub denied_tools: Vec<String>,
    /// Typed evidence bag for criterion matchers.
    pub bag: EvidenceBag,
}

impl AcceptanceEvidence {
    pub fn absorb_results(&mut self, results: &[ToolResult]) {
        for r in results {
            if r.ok {
                self.had_tool_success = true;
                if ToolName::parse(&r.name)
                    .map(|n| n.is_mutation())
                    .unwrap_or(false)
                {
                    self.files_written.push(r.input.clone());
                }
            } else if r.error.as_ref().map(|e| e.code) == Some(ToolErrorCode::PermissionDenied) {
                self.denied_tools.push(format!("{}: {}", r.name, r.input));
            }
            self.bag.absorb_tool_result(r, None);
        }
    }

    pub fn absorb_results_with_expectation(
        &mut self,
        results: &[ToolResult],
        expectation: Option<&CommandExpectation>,
    ) {
        for r in results {
            if r.ok {
                self.had_tool_success = true;
                if ToolName::parse(&r.name)
                    .map(|n| n.is_mutation())
                    .unwrap_or(false)
                {
                    self.files_written.push(r.input.clone());
                }
            } else if r.error.as_ref().map(|e| e.code) == Some(ToolErrorCode::PermissionDenied) {
                self.denied_tools.push(format!("{}: {}", r.name, r.input));
            }
            self.bag.absorb_tool_result(r, expectation);
        }
    }

    pub fn mark_verify(&mut self, ok: bool, commands: Vec<String>) {
        self.verify_ok = Some(ok);
        self.bag.mark_verify(ok, commands);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceReport {
    pub ok: bool,
    pub passed: Vec<String>,
    pub failures: Vec<String>,
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ToolCallId;

    fn read_ok() -> ToolResult {
        ToolResult::success(
            ToolCallId::new("r1"),
            ToolName::ReadFile.label(),
            "README.md",
            "…",
        )
    }

    fn write_ok(path: &str) -> ToolResult {
        ToolResult::success(
            ToolCallId::new("w1"),
            ToolName::WriteFile.label(),
            path,
            "wrote",
        )
    }

    #[test]
    fn plan_from_read_only_task_has_no_edit() {
        let plan = TaskPlan::from_task("Summarize the repository layout");
        assert!(!plan.requires_verify);
        assert!(plan.subtasks.iter().all(|s| s.kind == SubtaskKind::Read));
        assert!(!plan.acceptance_criteria.is_empty());
        assert!(!plan.goal.is_empty());
    }

    #[test]
    fn plan_from_edit_task_requires_verify() {
        let plan = TaskPlan::from_task("Please update README and fix docs");
        assert!(plan.requires_verify);
        assert!(plan.subtasks.iter().any(|s| s.kind == SubtaskKind::Edit));
        assert!(plan.subtasks.iter().any(|s| s.kind == SubtaskKind::Verify));
    }

    #[test]
    fn absorb_results_marks_read_subtask_done_with_evidence() {
        let mut plan = TaskPlan::from_task("Summarize the repository");
        assert!(!plan.subtasks[0].is_done());
        assert_eq!(plan.subtasks[0].status, SubtaskStatus::Pending);
        let n = plan.absorb_tool_results(&[read_ok()]);
        assert!(n >= 1);
        assert!(plan.subtasks[0].is_done());
        assert!(!plan.subtasks[0].evidence.is_empty());
        assert!(
            matches!(
                plan.subtasks[0].evidence[0],
                Evidence::ToolResultId { ref id, .. } if id == "r1" || id == "ev_r1"
            ) || matches!(plan.subtasks[0].evidence[0], Evidence::Typed(_)),
            "evidence={:?}",
            plan.subtasks[0].evidence
        );
    }

    #[test]
    fn done_subtask_cannot_lack_evidence() {
        let mut plan = TaskPlan::from_task("do a thing");
        // Simulate a corrupt done state and ensure acceptance rejects it.
        plan.subtasks[0].status = SubtaskStatus::Done;
        plan.subtasks[0].evidence.clear();
        let evidence = AcceptanceEvidence {
            had_tool_success: true,
            ..Default::default()
        };
        let report = plan.evaluate_acceptance(&evidence);
        assert!(!report.ok);
        assert!(report
            .failures
            .iter()
            .any(|f| f.contains("without evidence")));
        // Recovering with real evidence restores the invariant.
        plan.subtasks[0].mark_done(Evidence::ToolResultId {
            id: "x".into(),
            tool: "search".into(),
        });
        assert!(!plan.subtasks[0].evidence.is_empty());
    }

    #[test]
    fn workflow_advances_one_subtask_of_same_kind_at_a_time() {
        let mut plan = TaskPlan::from_task("x");
        plan.subtasks = vec![
            Subtask::new("s1", "repro", SubtaskKind::Command),
            Subtask::new("s2", "regression", SubtaskKind::Command),
        ];
        let first = ToolResult::success(
            ToolCallId::new("c1"),
            ToolName::RunCommand.label(),
            "cargo test repro",
            "fail",
        );
        plan.absorb_tool_results(&[first]);
        assert!(plan.subtasks[0].is_done());
        assert!(!plan.subtasks[1].is_done());
        let second = ToolResult::success(
            ToolCallId::new("c2"),
            ToolName::RunCommand.label(),
            "cargo test regression",
            "ok",
        );
        plan.absorb_tool_results(&[second]);
        assert!(plan.subtasks[1].is_done());
    }

    #[test]
    fn permission_denial_blocks_and_execution_error_fails() {
        let mut plan = TaskPlan::from_task("update");
        plan.subtasks = vec![Subtask::new("s1", "edit", SubtaskKind::Edit)];
        plan.absorb_tool_results(&[ToolResult::failure(
            ToolCallId::new("d1"),
            ToolName::WriteFile.label(),
            "a.md",
            crate::protocol::ToolError::permission_denied("nope"),
        )]);
        assert_eq!(plan.subtasks[0].status, SubtaskStatus::Blocked);

        plan.subtasks[0].status = SubtaskStatus::Pending;
        plan.absorb_tool_results(&[ToolResult::failure(
            ToolCallId::new("d2"),
            ToolName::WriteFile.label(),
            "a.md",
            crate::protocol::ToolError::execution("disk full"),
        )]);
        assert_eq!(plan.subtasks[0].status, SubtaskStatus::Failed);

        // Retry with a success → Done with evidence.
        plan.absorb_tool_results(&[write_ok("a.md")]);
        assert!(plan.subtasks[0].is_done());
        assert!(plan.subtasks[0]
            .evidence
            .iter()
            .any(|e| matches!(e, Evidence::ChangedFile { .. })));
    }

    #[test]
    fn apply_patch_success_counts_as_edit_with_changed_file_evidence() {
        let mut plan = TaskPlan::from_task("update code");
        plan.subtasks = vec![Subtask::new("s1", "edit", SubtaskKind::Edit)];
        plan.absorb_tool_results(&[ToolResult::success(
            ToolCallId::new("p1"),
            ToolName::ApplyPatch.label(),
            "src/lib.rs",
            "patched",
        )]);
        assert!(plan.subtasks[0].is_done());
        assert!(plan.subtasks[0]
            .evidence
            .iter()
            .any(|e| matches!(e, Evidence::ChangedFile { path } if path == "src/lib.rs")));
    }

    #[test]
    fn verify_done_records_passed_verification_evidence() {
        let mut plan = TaskPlan::from_task("update");
        plan.subtasks = vec![Subtask::new("v", "verify", SubtaskKind::Verify)];
        plan.verify_commands.push("cargo test --workspace".into());
        plan.mark_verify_done();
        assert!(plan.subtasks[0].is_done());
        assert!(plan.subtasks[0].evidence.iter().any(
            |e| matches!(e, Evidence::PassedVerification { command } if command.contains("cargo test"))
        ));
        assert!(plan.verify_commands.is_empty());
    }

    #[test]
    fn mark_current_running_moves_pending_to_running() {
        let mut plan = TaskPlan::from_task("update");
        assert_eq!(plan.subtasks[0].status, SubtaskStatus::Pending);
        plan.mark_current_running();
        assert_eq!(plan.subtasks[0].status, SubtaskStatus::Running);
        // Done subtasks stay done.
        plan.subtasks[0].mark_done(Evidence::ToolResultId {
            id: "t".into(),
            tool: "search".into(),
        });
        plan.current_subtask = 0;
        plan.mark_current_running();
        assert!(plan.subtasks[0].is_done());
    }

    #[test]
    fn from_skill_shapes_plan_from_spec() {
        let skill = crate::skill::SkillRegistry::builtin()
            .select(crate::classify::TaskType::BugFix)
            .expect("bug-fix skill")
            .clone();
        let plan = TaskPlan::from_skill(&skill, "Fix the panic when opening a project");
        assert_eq!(plan.subtasks.len(), skill.workflow.len());
        assert_eq!(
            plan.subtasks[0].kind,
            SubtaskKind::Command,
            "reproduce first"
        );
        assert_eq!(plan.acceptance_criteria, skill.completion_criteria);
        assert!(plan.requires_verify, "bug-fix policy is full");
        assert!(plan.constraints.iter().any(|c| c.contains("Permission")));
        assert!(plan.subtasks.iter().all(|s| !s.is_done()));
    }

    #[test]
    fn acceptance_rejects_model_claim_without_evidence() {
        let plan = TaskPlan::from_task("Please update the README file");
        let evidence = AcceptanceEvidence {
            model_claimed_done: true,
            ..Default::default()
        };
        let report = plan.evaluate_acceptance(&evidence);
        assert!(
            !report.ok,
            "claim alone must not pass: {:?}",
            report.failures
        );
    }

    #[test]
    fn acceptance_passes_edit_when_write_and_verify_ok() {
        let mut plan = TaskPlan::from_task("Please update the README file");
        plan.absorb_tool_results(&[write_ok("README.md")]);
        plan.mark_verify_done();
        let evidence = AcceptanceEvidence {
            had_tool_success: true,
            files_written: vec!["README.md".into()],
            verify_ok: Some(true),
            model_claimed_done: true,
            ..Default::default()
        };
        let report = plan.evaluate_acceptance(&evidence);
        assert!(report.ok, "expected pass, failures={:?}", report.failures);
    }

    #[test]
    fn acceptance_fails_when_verify_failed() {
        let mut plan = TaskPlan::from_task("Please update the README file");
        plan.absorb_tool_results(&[write_ok("README.md")]);
        let evidence = AcceptanceEvidence {
            had_tool_success: true,
            files_written: vec!["README.md".into()],
            verify_ok: Some(false),
            ..Default::default()
        };
        let report = plan.evaluate_acceptance(&evidence);
        assert!(!report.ok);
        assert!(report.failures.iter().any(|f| f.contains("verification")));
    }

    #[test]
    fn parse_model_json_roundtrip() {
        let json = r#"{
            "goal": "ship fix",
            "constraints": ["no coc"],
            "subtasks": [
                {"id":"a","title":"read","kind":"read"},
                {"id":"b","title":"edit","kind":"edit"}
            ],
            "acceptance_criteria": ["file written", "tests pass"],
            "requires_verify": true
        }"#;
        let plan = TaskPlan::parse_model_json(json).expect("parse");
        assert_eq!(plan.goal, "ship fix");
        assert_eq!(plan.subtasks.len(), 2);
        assert!(plan.requires_verify);
        assert_eq!(plan.acceptance_criteria.len(), 2);
        assert!(plan
            .subtasks
            .iter()
            .all(|s| s.status == SubtaskStatus::Pending));
    }

    #[test]
    fn progress_summary_has_no_multiline_cot() {
        let plan = TaskPlan::from_task("do a thing");
        let s = plan.progress_summary("Execute");
        assert!(!s.contains('\n'));
        assert!(s.starts_with("Execute"));
    }

    // -----------------------------------------------------------------------
    // semantic_completion — CompletionGate false-completion regressions
    // -----------------------------------------------------------------------

    #[test]
    fn semantic_completion_model_done_plus_unrelated_tool_cannot_finish() {
        let mut plan = TaskPlan::from_task("Please update README with badges");
        plan.absorb_tool_results(&[ToolResult::success(
            ToolCallId::new("g"),
            ToolName::RunCommand.label(),
            "git status --short",
            " M other.rs",
        )]);
        let evidence = AcceptanceEvidence {
            had_tool_success: true,
            model_claimed_done: true,
            ..Default::default()
        };
        let report = plan.evaluate_acceptance(&evidence);
        assert!(
            !report.ok,
            "claim + unrelated tool must not open the gate: {:?}",
            report.failures
        );
    }

    #[test]
    fn semantic_completion_tests_pass_with_unresolved_functional_cannot_verify() {
        // Free-form functional criterion stays unresolved even when verify passes.
        let mut plan = TaskPlan::from_task("Please update README with badges");
        plan.criteria.push(AcceptanceCriterion::new(
            "m_cX",
            "Ship the delightful polish users expect",
            infer_criterion_req("Ship the delightful polish users expect"),
        ));
        assert!(
            plan.criteria
                .iter()
                .find(|c| c.id == "m_cX")
                .map(|c| c.requirement.is_unresolved())
                .unwrap_or(false),
            "unknown free-form must be RequiresExplicitEvidence"
        );
        plan.absorb_tool_results(&[write_ok("README.md")]);
        plan.mark_verify_done();
        let mut evidence = AcceptanceEvidence {
            had_tool_success: true,
            files_written: vec!["README.md".into()],
            verify_ok: Some(true),
            model_claimed_done: true,
            ..Default::default()
        };
        evidence.bag.mark_verify(true, vec!["cargo test".into()]);
        let report = plan.evaluate_acceptance(&evidence);
        assert!(
            !report.ok,
            "unresolved functional criterion must block Finish despite verify: {:?}",
            report.failures
        );
        assert!(report
            .failures
            .iter()
            .any(|f| f.contains("unresolved") || f.contains("m_cX")));
    }

    #[test]
    fn semantic_completion_bound_evidence_is_not_reusable_across_criteria() {
        let mut plan = TaskPlan::from_task("verify twice");
        plan.criteria = vec![
            AcceptanceCriterion::new("c1", "first check", EvidenceRequirement::VerificationPassed),
            AcceptanceCriterion::new(
                "c2",
                "second check",
                EvidenceRequirement::VerificationPassed,
            ),
        ];
        let bound = EvidenceItem::new(
            "ev_bound",
            "verify",
            EvidenceKind::TestPassed {
                command: "cargo test".into(),
            },
        )
        .bound_to("c1");
        plan.absorb_evidence_item(&bound);
        assert!(plan.criteria[0].is_met());
        assert!(
            !plan.criteria[1].is_met(),
            "c2 must not steal c1-bound evidence"
        );
        // Reusable sentinel is explicit.
        let shared = EvidenceItem::new(
            "ev_shared",
            "verify",
            EvidenceKind::TestPassed {
                command: "cargo test".into(),
            },
        )
        .reusable();
        assert!(plan.criteria[1].absorb(&shared));
        assert!(plan.criteria[1].is_met());
    }

    #[test]
    fn semantic_completion_model_claim_is_not_evidence() {
        let plan = TaskPlan::from_task("Please update the README file");
        // Claim alone must never inject TestPassed/FileChanged into the bag.
        let evidence = AcceptanceEvidence {
            model_claimed_done: true,
            ..Default::default()
        };
        // Claim alone must never inject TestPassed/FileChanged into the bag.
        assert!(evidence.bag.items.is_empty());
        assert!(evidence.files_written.is_empty());
        let report = plan.evaluate_acceptance(&evidence);
        assert!(!report.ok);
        assert!(!report.passed.iter().any(|p| p.contains("test")));
    }

    #[test]
    fn semantic_completion_root_cause_criterion_rejects_read_and_git_status() {
        let root = infer_criterion_req("root cause located in source");
        assert!(matches!(
            root,
            EvidenceRequirement::SemanticProof {
                target: SemanticTarget::RootCause
            }
        ));
        let mut criterion = AcceptanceCriterion::new("rc", "root cause located", root);
        let read = EvidenceItem::new(
            "r",
            "read_file",
            EvidenceKind::FileRead {
                path: "src/lib.rs".into(),
            },
        );
        let git = EvidenceItem::new(
            "g",
            "run_command",
            EvidenceKind::CommandSucceeded {
                command: "git status --short".into(),
                exit_code: 0,
            },
        );
        assert!(!criterion.absorb(&read));
        assert!(!criterion.absorb(&git));
        assert!(!criterion.is_met());
    }
}
