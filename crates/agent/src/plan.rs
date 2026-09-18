//! Structured task plan the model and state machine share.
//!
//! A plan is **evidence-driven**: subtasks flip `done` only from observed
//! [`ToolResult`]s (or verify outcomes), never from model prose.

use crate::protocol::{ToolErrorCode, ToolResult, ToolName};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subtask {
    pub id: String,
    pub title: String,
    pub kind: SubtaskKind,
    pub done: bool,
}

impl Subtask {
    pub fn new(id: impl Into<String>, title: impl Into<String>, kind: SubtaskKind) -> Self {
        Self { id: id.into(), title: title.into(), kind, done: false }
    }
}

/// Plan handed to the model and used for acceptance evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskPlan {
    pub goal: String,
    pub constraints: Vec<String>,
    pub subtasks: Vec<Subtask>,
    pub acceptance_criteria: Vec<String>,
    /// Index into `subtasks` for the active item (len when all done).
    pub current_subtask: usize,
    /// Whether Finish requires a passing verify run.
    pub requires_verify: bool,
}

impl TaskPlan {
    /// Heuristic plan when no model planner is available (still structured).
    pub fn from_task(message: &str) -> Self {
        let goal = message.trim().lines().next().unwrap_or("task").trim().to_owned();
        let lower = message.to_ascii_lowercase();
        let mutating = matches!(
            ["write", "edit", "fix", "create", "update", "修改", "修复", "创建", "更新", "添加"]
                .iter()
                .any(|k| lower.contains(k)),
            true
        );

        let mut subtasks = vec![Subtask::new("s1", "Gather project context", SubtaskKind::Read)];
        if mutating {
            subtasks.push(Subtask::new("s2", "Apply the requested changes", SubtaskKind::Edit));
            subtasks.push(Subtask::new("s3", "Verify the project still holds", SubtaskKind::Verify));
        }

        let mut acceptance_criteria = vec!["Context was gathered from the live project".to_owned()];
        if mutating {
            acceptance_criteria.push("Requested files were written inside the project".to_owned());
            acceptance_criteria.push("Verification command completed successfully".to_owned());
        } else {
            acceptance_criteria.push("Answer is grounded in gathered tool observations".to_owned());
        }

        Self {
            goal,
            constraints: vec![
                "Paths stay inside the project".to_owned(),
                "No hidden chain-of-thought is stored".to_owned(),
            ],
            subtasks,
            acceptance_criteria,
            current_subtask: 0,
            requires_verify: mutating,
        }
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
            subtasks.push(Subtask::new(id, title, kind));
        }
        if subtasks.is_empty() {
            return None;
        }
        let requires_verify = value
            .get("requires_verify")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| subtasks.iter().any(|s| matches!(s.kind, SubtaskKind::Verify | SubtaskKind::Edit)));
        let current_subtask = value
            .get("current_subtask")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let subtask_count = subtasks.len();
        Some(Self {
            goal,
            constraints,
            subtasks,
            acceptance_criteria,
            current_subtask: current_subtask.min(subtask_count),
            requires_verify,
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
            let mark = if s.done { "x" } else if i == self.current_subtask { ">" } else { " " };
            out.push_str(&format!("  [{mark}] {} · {:?} · {}\n", s.id, s.kind, s.title));
        }
        out.push_str("- acceptance_criteria:\n");
        for c in &self.acceptance_criteria {
            out.push_str(&format!("  - {c}\n"));
        }
        out.push_str(&format!("- current_subtask: {}\n", self.current_subtask));
        out
    }

    /// Short, non-CoT progress line for Reasoning steps.
    pub fn progress_summary(&self, state_name: &str) -> String {
        let total = self.subtasks.len();
        let done = self.subtasks.iter().filter(|s| s.done).count();
        format!("{state_name} · subtasks {done}/{total} · {}", truncate(&self.goal, 60))
    }

    /// Mark subtasks done from tool evidence. Returns how many flipped.
    pub fn absorb_tool_results(&mut self, results: &[ToolResult]) -> usize {
        let mut flipped = 0;
        for result in results {
            if !result.ok {
                continue;
            }
            match result.name.as_str() {
                name if name == ToolName::ReadFile.label() || name == ToolName::Search.label() => {
                    flipped += self.mark_kind_done(SubtaskKind::Read);
                }
                name if name == ToolName::WriteFile.label() => {
                    flipped += self.mark_kind_done(SubtaskKind::Edit);
                }
                name if name == ToolName::RunCommand.label() => {
                    flipped += self.mark_kind_done(SubtaskKind::Command);
                }
                _ => {}
            }
        }
        // Advance cursor to first unfinished.
        self.current_subtask = self
            .subtasks
            .iter()
            .position(|s| !s.done)
            .unwrap_or(self.subtasks.len());
        flipped
    }

    pub fn mark_verify_done(&mut self) {
        for sub in &mut self.subtasks {
            if sub.kind == SubtaskKind::Verify {
                sub.done = true;
            }
        }
        self.current_subtask = self
            .subtasks
            .iter()
            .position(|s| !s.done)
            .unwrap_or(self.subtasks.len());
    }

    fn mark_kind_done(&mut self, kind: SubtaskKind) -> usize {
        let mut n = 0;
        for sub in &mut self.subtasks {
            if sub.kind == kind && !sub.done {
                sub.done = true;
                n += 1;
            }
        }
        n
    }

    /// Structural acceptance evaluation — model text is **not** an input.
    pub fn evaluate_acceptance(&self, evidence: &AcceptanceEvidence) -> AcceptanceReport {
        let mut failures = Vec::new();
        let mut passed = Vec::new();

        // 1) Subtasks must be evidence-done (or there are none pending that require proof).
        let pending: Vec<&Subtask> = self.subtasks.iter().filter(|s| !s.done).collect();
        // Read-only completion: at least one successful observation in evidence.
        if self.subtasks.iter().any(|s| s.kind == SubtaskKind::Read) && !evidence.had_tool_success {
            failures.push("no successful tool observation".to_owned());
        }
        if self.requires_verify {
            match evidence.verify_ok {
                None => failures.push("verification has not run".to_owned()),
                Some(false) => failures.push("verification failed".to_owned()),
                Some(true) => passed.push("verification passed".to_owned()),
            }
        }
        // Edit subtasks need a successful write.
        if self.subtasks.iter().any(|s| s.kind == SubtaskKind::Edit && !s.done) {
            if evidence.files_written.is_empty() {
                failures.push("edit subtask has no write evidence".to_owned());
            } else {
                passed.push(format!("wrote {} file(s)", evidence.files_written.len()));
            }
        } else if self.subtasks.iter().any(|s| s.kind == SubtaskKind::Edit) {
            passed.push("edit subtasks completed".to_owned());
        }

        // Remaining incomplete non-read subtasks without evidence:
        for sub in &pending {
            if sub.kind == SubtaskKind::Read && evidence.had_tool_success {
                continue;
            }
            if !failures.iter().any(|f| f.contains(sub.id.as_str())) {
                failures.push(format!("subtask {} unfinished: {}", sub.id, sub.title));
            }
        }

        // 2) Acceptance criteria must be backed by structural evidence, not claims.
        for criterion in &self.acceptance_criteria {
            if criterion_could_be_structural(criterion) {
                // Structural criteria satisfied if we have tool success / writes / verify as above.
                let ok = evidence.had_tool_success
                    && (!self.requires_verify || evidence.verify_ok == Some(true))
                    && (!self.subtasks.iter().any(|s| s.kind == SubtaskKind::Edit)
                        || !evidence.files_written.is_empty());
                if ok {
                    passed.push(criterion.clone());
                } else {
                    failures.push(format!("unmet criterion: {criterion}"));
                }
            } else {
                // Free-form criteria: require *some* positive evidence package.
                if evidence.had_tool_success
                    && (!self.requires_verify || evidence.verify_ok == Some(true))
                {
                    passed.push(criterion.clone());
                } else {
                    failures.push(format!("unmet criterion: {criterion}"));
                }
            }
        }

        // 3) Model claim alone is recorded but never sufficient.
        if evidence.model_claimed_done && failures.is_empty() && passed.len() == self.acceptance_criteria.len()
        {
            // claim is ignored as proof; already required structural evidence above
        }

        AcceptanceReport {
            ok: failures.is_empty() && !self.acceptance_criteria.is_empty(),
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
}

impl AcceptanceEvidence {
    pub fn absorb_results(&mut self, results: &[ToolResult]) {
        for r in results {
            if r.ok {
                self.had_tool_success = true;
                if r.name == ToolName::WriteFile.label() {
                    self.files_written.push(r.input.clone());
                }
            } else if r.error.as_ref().map(|e| e.code) == Some(ToolErrorCode::PermissionDenied) {
                self.denied_tools.push(format!("{}: {}", r.name, r.input));
            }
        }
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
        ToolResult::success(ToolCallId::new("r1"), ToolName::ReadFile.label(), "README.md", "…")
    }

    fn write_ok(path: &str) -> ToolResult {
        ToolResult::success(ToolCallId::new("w1"), ToolName::WriteFile.label(), path, "wrote")
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
    fn absorb_results_marks_read_subtask_done() {
        let mut plan = TaskPlan::from_task("Summarize the repository");
        assert!(!plan.subtasks[0].done);
        let n = plan.absorb_tool_results(&[read_ok()]);
        assert!(n >= 1);
        assert!(plan.subtasks[0].done);
    }

    #[test]
    fn acceptance_rejects_model_claim_without_evidence() {
        let plan = TaskPlan::from_task("Please update the README file");
        let evidence = AcceptanceEvidence { model_claimed_done: true, ..Default::default() };
        let report = plan.evaluate_acceptance(&evidence);
        assert!(!report.ok, "claim alone must not pass: {:?}", report.failures);
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
    }

    #[test]
    fn progress_summary_has_no_multiline_cot() {
        let plan = TaskPlan::from_task("do a thing");
        let s = plan.progress_summary("Execute");
        assert!(!s.contains('\n'));
        assert!(s.starts_with("Execute"));
    }
}
