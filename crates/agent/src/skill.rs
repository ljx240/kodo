//! SkillSpec: structured, version-controllable skills that steer the real
//! Planner, Verifier, and tool gate — not a Markdown blob in the prompt.
//!
//! Skill files live at `skills/<name>/SKILL.md` and use section headers the
//! parser understands (`## applicable_task_types`, `## workflow`, …). The six
//! built-in skills are embedded via `include_str!` so the registry is
//! available with zero filesystem I/O.

use std::path::Path;

use crate::classify::TaskType;
use crate::context::ContextBudget;
use crate::plan::SubtaskKind;
use crate::protocol::{ToolCallId, ToolError, ToolName, ToolRegistry, ToolResult};
use crate::verify::{VerificationRunner, VerifyCommand, VerifyKind};

/// How a skill wants project context gathered and the plan ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextStrategy {
    /// Bug fixes: reproduce before editing.
    ReproFirst,
    /// Features: acceptance criteria before editing.
    AcceptanceFirst,
    /// Tests: understand conventions, then write tests first.
    TestFirst,
    /// Refactors: smallest coherent diff against current structure.
    MinimalChange,
    /// Reviews: read-only inspection, no mutation.
    ReviewOnly,
    /// Docs: lightweight docs-oriented context, no deep code packing.
    DocsFocused,
}

impl ContextStrategy {
    pub fn label(self) -> &'static str {
        match self {
            Self::ReproFirst => "repro-first",
            Self::AcceptanceFirst => "acceptance-first",
            Self::TestFirst => "test-first",
            Self::MinimalChange => "minimal-change",
            Self::ReviewOnly => "review-only",
            Self::DocsFocused => "docs-focused",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "repro-first" => Some(Self::ReproFirst),
            "acceptance-first" => Some(Self::AcceptanceFirst),
            "test-first" => Some(Self::TestFirst),
            "minimal-change" => Some(Self::MinimalChange),
            "review-only" => Some(Self::ReviewOnly),
            "docs-focused" => Some(Self::DocsFocused),
            _ => None,
        }
    }

    /// Context packing budget preset — a real effect on the context phase.
    pub fn budget(self) -> ContextBudget {
        match self {
            // Docs updates need a light touch: fewer, shorter spans.
            Self::DocsFocused => ContextBudget {
                max_spans: 4,
                ..ContextBudget::default()
            },
            // Reviews target the named files; slightly tighter packing.
            Self::ReviewOnly => ContextBudget {
                max_spans: 6,
                ..ContextBudget::default()
            },
            _ => ContextBudget::default(),
        }
    }
}

/// What the Verifier is allowed (and required) to run for this skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationPolicy {
    /// Run the full inferred build/typecheck/test/lint set.
    Full,
    /// Run only test-kind commands — no gratuitous full builds.
    TestsOnly,
    /// Do not run project verification (docs / read-only review).
    None,
}

impl VerificationPolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::TestsOnly => "tests-only",
            Self::None => "none",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "full" => Some(Self::Full),
            "tests-only" => Some(Self::TestsOnly),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    /// Whether the plan must pass through the Verify gate at all.
    pub fn needs_run(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// One ordered workflow step that becomes a plan subtask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStep {
    pub id: String,
    pub title: String,
    pub kind: SubtaskKind,
}

/// Structured skill specification (the seven contract fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSpec {
    pub name: String,
    pub applicable_task_types: Vec<TaskType>,
    pub context_strategy: ContextStrategy,
    pub allowed_tools: Vec<ToolName>,
    pub workflow: Vec<WorkflowStep>,
    pub completion_criteria: Vec<String>,
    pub verification_policy: VerificationPolicy,
    /// Short human guidance (kept in the file; only compact excerpts reach prompts).
    pub guidance: String,
}

impl SkillSpec {
    /// Skill-level tool allowlist. **Strictly narrower** than Permission:
    /// allowing a tool here never bypasses `Permission::needs_approval`.
    pub fn allows(&self, tool: ToolName) -> bool {
        self.allowed_tools.contains(&tool)
    }

    pub fn allows_label(&self, label: &str) -> bool {
        ToolName::parse(label)
            .map(|t| self.allows(t))
            .unwrap_or(false)
    }

    /// Gate one tool call. Returns a structured failure for disallowed tools —
    /// called **before** the Permission approval path, never instead of it.
    pub fn gate(&self, id: ToolCallId, tool_label: &str, input: &str) -> Option<ToolResult> {
        if self.allows_label(tool_label) {
            return None;
        }
        Some(ToolResult::failure(
            id,
            tool_label,
            input,
            ToolError::permission_denied(format!(
                "skill `{}` does not allow tool `{tool_label}`",
                self.name
            )),
        ))
    }

    /// Registry visible to the model: only skill-allowed tools are advertised
    /// (and therefore parseable) for this turn.
    pub fn filter_registry(&self, registry: ToolRegistry) -> ToolRegistry {
        registry.filtered(|name| self.allows_label(name))
    }

    /// Verification commands this skill permits for the project.
    /// `None` policy → empty (the Verifier must not run a full compile for docs).
    pub fn verification_commands(
        &self,
        runner: &VerificationRunner,
        project: &Path,
    ) -> Vec<VerifyCommand> {
        let inferred = runner.infer(project);
        match self.verification_policy {
            VerificationPolicy::Full => inferred,
            VerificationPolicy::TestsOnly => inferred
                .into_iter()
                .filter(|c| c.kind == VerifyKind::Test)
                .collect(),
            VerificationPolicy::None => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Parser — section-based Markdown, no extra dependencies.
// ---------------------------------------------------------------------------

fn section_body<'a>(lines: &[&'a str], header: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in lines {
        let trimmed = line.trim();
        if let Some(h) = trimmed.strip_prefix("## ") {
            inside = h.trim().eq_ignore_ascii_case(header);
            continue;
        }
        if inside {
            out.push(*line);
        }
    }
    out
}

fn bullets(body: &[&str]) -> Vec<String> {
    body.iter()
        .filter_map(|l| {
            let t = l.trim();
            t.strip_prefix("- ").map(|s| s.trim().to_owned())
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn single_line(body: &[&str]) -> String {
    body.iter()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn parse_workflow(body: &[&str]) -> Result<Vec<WorkflowStep>, String> {
    let mut steps = Vec::new();
    for raw in bullets(body) {
        // Format: `id | kind | title`
        let parts: Vec<&str> = raw.splitn(3, '|').map(str::trim).collect();
        if parts.len() != 3 {
            return Err(format!(
                "workflow step must be `id | kind | title`, got: {raw}"
            ));
        }
        let kind = match parts[1].to_ascii_lowercase().as_str() {
            "read" => SubtaskKind::Read,
            "edit" => SubtaskKind::Edit,
            "command" => SubtaskKind::Command,
            "verify" => SubtaskKind::Verify,
            other => return Err(format!("unknown workflow kind `{other}`")),
        };
        steps.push(WorkflowStep {
            id: parts[0].to_owned(),
            title: parts[2].to_owned(),
            kind,
        });
    }
    Ok(steps)
}

/// Parse one SKILL.md into a [`SkillSpec`].
pub fn parse_skill(markdown: &str) -> Result<SkillSpec, String> {
    let lines: Vec<&str> = markdown.lines().collect();
    let name = lines
        .iter()
        .find_map(|l| l.trim().strip_prefix("# Skill:"))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("missing `# Skill: <name>` title")?
        .to_owned();

    let task_types: Vec<TaskType> = bullets(&section_body(&lines, "applicable_task_types"))
        .into_iter()
        .map(|raw| TaskType::parse(&raw).ok_or_else(|| format!("unknown task_type `{raw}`")))
        .collect::<Result<_, _>>()?;
    if task_types.is_empty() {
        return Err(format!("skill `{name}` has no applicable_task_types"));
    }

    let strategy_raw = single_line(&section_body(&lines, "context_strategy"));
    let context_strategy = ContextStrategy::parse(&strategy_raw)
        .ok_or_else(|| format!("unknown context_strategy `{strategy_raw}`"))?;

    let allowed_tools: Vec<ToolName> = bullets(&section_body(&lines, "allowed_tools"))
        .into_iter()
        .map(|raw| ToolName::parse(&raw).ok_or_else(|| format!("unknown tool `{raw}`")))
        .collect::<Result<_, _>>()?;

    let policy_raw = single_line(&section_body(&lines, "verification_policy"));
    let verification_policy = VerificationPolicy::parse(&policy_raw)
        .ok_or_else(|| format!("unknown verification_policy `{policy_raw}`"))?;

    let workflow = parse_workflow(&section_body(&lines, "workflow"))?;
    if workflow.is_empty() {
        return Err(format!("skill `{name}` has an empty workflow"));
    }

    let completion_criteria = bullets(&section_body(&lines, "completion_criteria"));
    if completion_criteria.is_empty() {
        return Err(format!("skill `{name}` has no completion_criteria"));
    }

    let guidance = section_body(&lines, "guidance")
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(SkillSpec {
        name,
        applicable_task_types: task_types,
        context_strategy,
        allowed_tools,
        workflow,
        completion_criteria,
        verification_policy,
        guidance,
    })
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Selects the active skill for a classified task type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRegistry {
    skills: Vec<SkillSpec>,
}

impl SkillRegistry {
    /// The six built-in skills, embedded at compile time.
    pub fn builtin() -> Self {
        const SOURCES: [&str; 6] = [
            include_str!("../../../skills/bug-fix/SKILL.md"),
            include_str!("../../../skills/feature/SKILL.md"),
            include_str!("../../../skills/test/SKILL.md"),
            include_str!("../../../skills/refactor/SKILL.md"),
            include_str!("../../../skills/code-review/SKILL.md"),
            include_str!("../../../skills/docs/SKILL.md"),
        ];
        let skills = SOURCES
            .iter()
            .map(|src| parse_skill(src).expect("built-in SKILL.md must parse"))
            .collect();
        Self { skills }
    }

    pub fn from_skills(skills: Vec<SkillSpec>) -> Self {
        Self { skills }
    }

    /// Pick the skill whose `applicable_task_types` contains `task_type`.
    pub fn select(&self, task_type: TaskType) -> Option<&SkillSpec> {
        self.skills
            .iter()
            .find(|s| s.applicable_task_types.contains(&task_type))
    }

    pub fn get(&self, name: &str) -> Option<&SkillSpec> {
        self.skills.iter().find(|s| s.name == name)
    }

    pub fn skills(&self) -> &[SkillSpec] {
        &self.skills
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> SkillRegistry {
        SkillRegistry::builtin()
    }

    /// Owned skill for a task type (avoids borrowing a temporary registry).
    fn skill_of(kind: TaskType) -> SkillSpec {
        registry()
            .select(kind)
            .unwrap_or_else(|| panic!("no skill for {kind}"))
            .clone()
    }

    #[test]
    fn builtin_registry_covers_all_six_task_types() {
        let reg = registry();
        for kind in TaskType::ALL {
            let skill = reg
                .select(kind)
                .unwrap_or_else(|| panic!("no skill for {kind}"));
            assert!(skill.applicable_task_types.contains(&kind));
        }
        assert_eq!(reg.skills().len(), 6);
    }

    #[test]
    fn builtin_skills_parse_with_expected_shape() {
        let reg = registry();
        for skill in reg.skills() {
            assert!(!skill.name.is_empty());
            assert!(!skill.allowed_tools.is_empty());
            assert!(!skill.workflow.is_empty());
            assert!(!skill.completion_criteria.is_empty());
            assert!(!skill.guidance.is_empty());
            let ids: Vec<_> = skill.workflow.iter().map(|w| w.id.as_str()).collect();
            let unique: std::collections::HashSet<_> = ids.iter().copied().collect();
            assert_eq!(
                ids.len(),
                unique.len(),
                "workflow ids unique in {}",
                skill.name
            );
        }
    }

    #[test]
    fn code_review_skill_is_read_only() {
        let skill = skill_of(TaskType::CodeReview);
        for mutation in [
            ToolName::WriteFile,
            ToolName::ApplyPatch,
            ToolName::ReplaceRange,
            ToolName::CreateFile,
            ToolName::DeleteFile,
        ] {
            assert!(
                !skill.allows(mutation),
                "code-review must not allow {}",
                mutation.label()
            );
        }
        assert!(skill.allows(ToolName::ReadFile));
        assert_eq!(skill.verification_policy, VerificationPolicy::None);
    }

    #[test]
    fn docs_skill_forbids_shell_and_full_compile() {
        let skill = skill_of(TaskType::Docs);
        assert!(!skill.allows(ToolName::RunCommand));
        assert_eq!(skill.verification_policy, VerificationPolicy::None);
        assert!(!skill.verification_policy.needs_run());
    }

    #[test]
    fn gate_rejects_disallowed_tool_with_permission_denied_code() {
        let skill = skill_of(TaskType::CodeReview);
        let denied = skill
            .gate(ToolCallId::new("g1"), "write_file", "README.md")
            .expect("must deny");
        assert!(!denied.ok);
        assert_eq!(
            denied.error.as_ref().map(|e| e.code),
            Some(crate::protocol::ToolErrorCode::PermissionDenied)
        );
        assert!(denied.error.unwrap().message.contains("code-review"));
        assert!(skill
            .gate(ToolCallId::new("g2"), "read_file", "src/lib.rs")
            .is_none());
    }

    #[test]
    fn skill_allowlist_never_bypasses_permission_approval() {
        // Skill allows the write…
        let skill = skill_of(TaskType::BugFix);
        assert!(skill.allows(ToolName::WriteFile));
        assert!(skill
            .gate(ToolCallId::new("p"), "write_file", "a.rs")
            .is_none());
        // …but Ask mode still requires human approval for file changes.
        let ask = crate::Permission::Ask;
        assert!(ask.needs_approval(crate::StepKind::FileChange, Some("write a.rs")));
        let auto = crate::Permission::Auto;
        assert!(auto.needs_approval(crate::StepKind::FileChange, Some("write a.rs")));
    }

    #[test]
    fn filtered_registry_hides_disallowed_tools_from_the_model() {
        let skill = skill_of(TaskType::Docs);
        let reg = skill.filter_registry(ToolRegistry::standard());
        assert!(reg.find("write_file").is_some());
        assert!(reg.find("run_command").is_none());
        assert!(reg.find("delete_file").is_none());
        // Parse-level rejection as defense in depth.
        let inv = reg.accept(
            ToolCallId::new("x"),
            "run_command",
            &serde_json::json!({"command": "cargo test"}),
        );
        assert!(matches!(inv, crate::protocol::ToolInvocation::Rejected(_)));
    }

    #[test]
    fn verification_commands_respect_policy() {
        // Project markers: cargo present so Full/TestsOnly have something to pick.
        let dir = std::env::temp_dir().join(format!("kodo_skill_verify_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname=\"t\"\nversion=\"0.1.0\"\n",
        )
        .expect("cargo toml");
        std::fs::create_dir_all(dir.join("src")).expect("src");
        std::fs::write(dir.join("src/lib.rs"), "").expect("lib");

        let runner = VerificationRunner::new(1_000);

        let docs = skill_of(TaskType::Docs);
        let cmds = docs.verification_commands(&runner, &dir);
        assert!(
            cmds.is_empty(),
            "docs must not run a full compile: {cmds:?}"
        );

        let review = skill_of(TaskType::CodeReview);
        assert!(review.verification_commands(&runner, &dir).is_empty());

        let bug = skill_of(TaskType::BugFix);
        let full = bug.verification_commands(&runner, &dir);
        assert!(!full.is_empty(), "bug-fix Full should infer cargo commands");

        let test_skill = skill_of(TaskType::Test);
        let tests_only = test_skill.verification_commands(&runner, &dir);
        assert!(!tests_only.is_empty());
        assert!(tests_only.iter().all(|c| c.kind == VerifyKind::Test));
        assert!(
            !tests_only.iter().any(|c| c.kind == VerifyKind::Build),
            "tests-only must not run cargo check"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_skill_rejects_malformed_input() {
        assert!(parse_skill("no title").is_err());
        assert!(parse_skill(
            "# Skill: x\n## applicable_task_types\n- nope\n## context_strategy\nrepro-first\n"
        )
        .is_err());
        assert!(parse_skill(
            "# Skill: x\n## applicable_task_types\n- docs\n## context_strategy\nbogus\n"
        )
        .is_err());
    }

    #[test]
    fn context_strategy_budget_is_smaller_for_docs() {
        let docs = skill_of(TaskType::Docs);
        let bug = skill_of(TaskType::BugFix);
        assert!(docs.context_strategy.budget().max_spans < bug.context_strategy.budget().max_spans);
    }
}
