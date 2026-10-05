//! SkillSpec: structured, version-controllable skills that steer the real
//! Planner, Verifier, and tool gate — not a Markdown blob in the prompt.
//!
//! Skill files live at `skills/<name>/SKILL.md` and use section headers the
//! parser understands (`## applicable_task_types`, `## workflow`, …). The eight
//! built-in skills are embedded via `include_str!` so the registry is
//! available with zero filesystem I/O. User skills load from a caller-supplied
//! directory ([`load_user_skills`]) — this crate never resolves paths itself.

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
    /// Work mode: general deliverables; light context, no repo-map depth.
    WorkFocused,
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
            Self::WorkFocused => "work-focused",
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
            "work-focused" => Some(Self::WorkFocused),
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
            // Work deliverables rarely need deep repo packing.
            Self::WorkFocused => ContextBudget {
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
    /// One-line summary (`## description`). Parse falls back to the first
    /// completion criterion; saving requires the explicit section.
    pub description: String,
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
        // External (MCP) tools (`mcp__*` wire names) pass the skill gate
        // unconditionally: `allowed_tools` narrows **builtin** tools only —
        // a skill never blocks a server tool the project configured. They are
        // not free of oversight: every external call still goes through
        // `Permission::needs_approval` (a skill can narrow, never widen).
        if label.starts_with("mcp__") {
            return true;
        }
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

    /// Registry visible to the model: only skill-allowed **builtin** tools are
    /// advertised (and therefore parseable) for this turn. External (MCP)
    /// tools pass through untouched — the allowlist never narrows them (see
    /// `allows_label`).
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

pub(crate) fn section_body<'a>(lines: &[&'a str], header: &str) -> Vec<&'a str> {
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

pub(crate) fn single_line(body: &[&str]) -> String {
    body.iter()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// Truncate on a char boundary (skill text is user-supplied UTF-8).
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    s.chars().take(max).collect()
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

    // Backward-tolerant: older skill files carry no `## description` — fall
    // back to the first completion criterion so the field is never empty.
    let description_raw = single_line(&section_body(&lines, "description"));
    let description = if description_raw.is_empty() {
        completion_criteria
            .first()
            .map(|c| truncate_chars(c.trim(), 120))
            .unwrap_or_default()
    } else {
        description_raw
    };

    let guidance = section_body(&lines, "guidance")
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(SkillSpec {
        name,
        description,
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

/// Embedded built-in skill sources, keyed by name, in registry order.
/// `select()` is first-match, so `workflow` (also applicable to `work` tasks)
/// must stay **after** `work` — otherwise every Work task would silently
/// switch skills. Guarded by the `workflow_registration_keeps_work_selectable`
/// test.
pub(crate) const BUILTIN_SOURCES: [(&str, &str); 8] = [
    ("bug-fix", include_str!("../../../skills/bug-fix/SKILL.md")),
    ("feature", include_str!("../../../skills/feature/SKILL.md")),
    ("test", include_str!("../../../skills/test/SKILL.md")),
    (
        "refactor",
        include_str!("../../../skills/refactor/SKILL.md"),
    ),
    (
        "code-review",
        include_str!("../../../skills/code-review/SKILL.md"),
    ),
    ("docs", include_str!("../../../skills/docs/SKILL.md")),
    ("work", include_str!("../../../skills/work/SKILL.md")),
    (
        "workflow",
        include_str!("../../../skills/workflow/SKILL.md"),
    ),
];

/// Embedded Markdown for a built-in skill — the read-only preview source.
pub fn builtin_markdown(name: &str) -> Option<&'static str> {
    BUILTIN_SOURCES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, src)| *src)
}

impl SkillRegistry {
    /// The eight built-in skills, embedded at compile time.
    pub fn builtin() -> Self {
        let skills = BUILTIN_SOURCES
            .iter()
            .map(|(_, src)| parse_skill(src).expect("built-in SKILL.md must parse"))
            .collect();
        Self { skills }
    }

    pub fn from_skills(skills: Vec<SkillSpec>) -> Self {
        Self { skills }
    }

    /// Builtins first, extra skills appended (project then user in
    /// `build_skill_registry`): `select()`'s first-match fallback always lands
    /// on a builtin; `get(name)` (the chip path) reaches extras too and is
    /// first-match, so earlier extras shadow later ones on a name collision.
    pub fn with_user(mut self, user: Vec<SkillSpec>) -> Self {
        self.skills.extend(user);
        self
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

// ---------------------------------------------------------------------------
// User skill store — flat `<dir>/<name>.md`. The directory is always supplied
// by the caller (`RunRequest.user_skills_dir` / the Tauri `skills_dir()`):
// this crate has no core dependency and must stay hermetic in tests.
// ---------------------------------------------------------------------------

/// Chip ids and file stems: `[A-Za-z0-9_-]{1,64}`.
fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn is_builtin_name(name: &str) -> bool {
    BUILTIN_SOURCES.iter().any(|(n, _)| *n == name)
}

/// Load every usable `*.md` in `dir`. Missing dir → empty. Files are skipped
/// when they fail [`parse_skill`], the stem or the `# Skill:` title violates
/// the name rules, the name collides with a builtin, or a name already loaded
/// (first wins). Entries are sorted by file name so loading is deterministic.
pub fn load_user_skills(dir: &Path) -> Vec<SkillSpec> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();
    paths.sort();
    let mut out: Vec<SkillSpec> = Vec::new();
    for path in paths {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !valid_skill_name(stem) {
            continue;
        }
        let Ok(markdown) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(spec) = parse_skill(&markdown) else {
            continue;
        };
        // Layout invariant: file name = chip id = spec.name (save enforces it;
        // tolerate no drift on load so chip lookup can't miss).
        if spec.name != stem || is_builtin_name(&spec.name) {
            continue;
        }
        if out.iter().any(|s| s.name == spec.name) {
            continue;
        }
        out.push(spec);
    }
    out
}

/// Validate and write one user skill to `dir/<name>.md`. The `# Skill:` title
/// must equal `name` so the chip id, the file name and the parsed name agree.
/// Errors are zh-CN for direct display in the UI.
pub fn save_user_skill(dir: &Path, name: &str, markdown: &str) -> Result<SkillSpec, String> {
    if !valid_skill_name(name) {
        return Err(format!(
            "技能名无效 `{name}`：仅限字母、数字、- 和 _，最多 64 字符"
        ));
    }
    if is_builtin_name(name) {
        return Err(format!("`{name}` 是内置技能名，请换一个名称"));
    }
    let spec = parse_skill(markdown)?;
    if spec.name != name {
        return Err(format!(
            "技能标题 `# Skill: {}` 与名称 `{name}` 不一致",
            spec.name
        ));
    }
    // Save is strict where parse is tolerant: new user skills must carry an
    // explicit `## description` (the editor template includes it).
    if single_line(&section_body(
        &markdown.lines().collect::<Vec<_>>(),
        "description",
    ))
    .is_empty()
    {
        return Err("技能缺少 `## description` 一行描述".to_owned());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{name}.md")), markdown).map_err(|e| e.to_string())?;
    Ok(spec)
}

/// Remove `dir/<name>.md`. Builtins and invalid names are rejected; a missing
/// file is `技能不存在` (the UI already removed the row — not fatal).
pub fn delete_user_skill(dir: &Path, name: &str) -> Result<(), String> {
    if !valid_skill_name(name) {
        return Err(format!("技能名无效 `{name}`"));
    }
    if is_builtin_name(name) {
        return Err(format!("内置技能 `{name}` 不可删除"));
    }
    let path = dir.join(format!("{name}.md"));
    if !path.exists() {
        return Err("技能不存在".to_owned());
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())
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
    fn builtin_registry_covers_all_task_types() {
        let reg = registry();
        for kind in TaskType::ALL {
            let skill = reg
                .select(kind)
                .unwrap_or_else(|| panic!("no skill for {kind}"));
            assert!(skill.applicable_task_types.contains(&kind));
        }
        assert_eq!(reg.skills().len(), 8);
    }

    #[test]
    fn workflow_registration_keeps_work_selectable() {
        // `select()` is first-match and both work + workflow declare the Work
        // task type: workflow must be registered after work or every plain
        // Work task would silently switch skills.
        let reg = registry();
        assert_eq!(reg.select(TaskType::Work).unwrap().name, "work");
        assert!(reg.get("workflow").is_some());
        assert!(builtin_markdown("workflow").is_some_and(|s| s.contains("# Skill: workflow")));
        assert!(builtin_markdown("nope").is_none());
    }

    #[test]
    fn workflow_allowlist_is_exactly_the_work_row_of_the_mode_matrix() {
        // Same invariant as the work skill: the workflow skill must not be
        // load-bearing for work-mode tool availability (DESIGN.md §11).
        let skill = registry()
            .get("workflow")
            .expect("workflow builtin")
            .clone();
        let skill_names: std::collections::BTreeSet<&str> =
            skill.allowed_tools.iter().map(|t| t.label()).collect();
        let matrix = ToolRegistry::for_mode(crate::AgentMode::Work, true);
        let matrix_names: std::collections::BTreeSet<&str> =
            matrix.definitions().iter().map(|d| d.name).collect();
        assert_eq!(skill_names, matrix_names);
        assert_eq!(skill.verification_policy, VerificationPolicy::None);
    }

    #[test]
    fn workflow_plan_is_deliverable_bar_without_verify() {
        use crate::plan::TaskPlan;
        let reg = registry();
        let skill = reg.get("workflow").expect("workflow builtin");
        let plan = TaskPlan::from_skill(skill, "批量整理这批文件");
        assert!(plan.deliverable_bar, "work-family skill carries the bar");
        assert!(!plan.requires_verify, "verification_policy none");
    }

    #[test]
    fn builtin_skills_parse_with_expected_shape() {
        let reg = registry();
        for skill in reg.skills() {
            assert!(!skill.name.is_empty());
            assert!(!skill.description.is_empty());
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
    fn skill_bugfix_reproduction_requires_fingerprint_not_any_nonzero() {
        use crate::evidence::{CommandExpectation, EvidenceRequirement};
        use crate::plan::TaskPlan;

        let skill = skill_of(TaskType::BugFix);
        let plan = TaskPlan::from_skill(&skill, "fix the panic when opening a project");
        let first = &plan.subtasks[0];
        match &first.effective_requirement().evidence {
            EvidenceRequirement::CommandOutcome {
                expectation: CommandExpectation::Reproduction(fe),
                ..
            } => {
                assert!(
                    fe.capture_on_first_failure || fe.has_specific_fingerprint(),
                    "repro step must use FailureExpectation, got {:?}",
                    fe
                );
            }
            other => panic!("bug-fix s1 must be Reproduction expectation, got {other:?}"),
        }
        // Completion criterion for reproduction is SemanticProof, not bare failure.
        let repro_criterion = plan
            .criteria
            .iter()
            .find(|c| c.description.to_ascii_lowercase().contains("reproduc"))
            .expect("reproduction criterion");
        assert!(
            matches!(
                repro_criterion.requirement,
                EvidenceRequirement::SemanticProof { .. }
            ),
            "got {:?}",
            repro_criterion.requirement
        );
        // Repro and verify use separate ledger fields (never mixed at init).
        assert!(plan.repro_commands.is_empty() && plan.verify_commands.is_empty());
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
    fn work_skill_allows_files_but_never_symbol_tools_or_verify() {
        let skill = skill_of(TaskType::Work);
        for tool in [
            ToolName::WriteFile,
            ToolName::CreateFile,
            ToolName::ApplyPatch,
            ToolName::ReplaceRange,
            ToolName::DeleteFile,
            ToolName::RunCommand,
            ToolName::Search,
            ToolName::ReadFile,
        ] {
            assert!(skill.allows(tool), "work must allow {}", tool.label());
        }
        for tool in [
            ToolName::FindSymbol,
            ToolName::FindReferences,
            ToolName::ReadRange,
        ] {
            assert!(!skill.allows(tool), "work must not allow {}", tool.label());
        }
        assert_eq!(skill.verification_policy, VerificationPolicy::None);
        let reg = skill.filter_registry(ToolRegistry::standard());
        assert!(reg.find("write_file").is_some());
        assert!(reg.find("find_symbol").is_none());
        assert!(reg.find("find_references").is_none());
    }

    #[test]
    fn work_allowlist_is_exactly_the_work_row_of_the_mode_matrix() {
        // The work skill must not be load-bearing for the matrix (DESIGN.md
        // §11): if these drift, work-mode tool availability would depend on
        // skill contents instead of the explicit ToolRegistry::for_mode row.
        let skill = skill_of(TaskType::Work);
        let skill_names: std::collections::BTreeSet<&str> =
            skill.allowed_tools.iter().map(|t| t.label()).collect();
        let matrix = ToolRegistry::for_mode(crate::AgentMode::Work, true);
        let matrix_names: std::collections::BTreeSet<&str> =
            matrix.definitions().iter().map(|d| d.name).collect();
        assert_eq!(skill_names, matrix_names);
    }

    #[test]
    fn code_skills_advertise_symbol_tools() {
        // The system prompt tells the coding agent to navigate with
        // find_symbol / find_references / read_range — the code skills'
        // allowlists must actually keep them in the advertised registry.
        for kind in [
            TaskType::BugFix,
            TaskType::Feature,
            TaskType::Test,
            TaskType::Refactor,
            TaskType::CodeReview,
            TaskType::Docs,
        ] {
            let skill = skill_of(kind);
            for tool in [
                ToolName::FindSymbol,
                ToolName::FindReferences,
                ToolName::ReadRange,
                ToolName::ListFiles,
            ] {
                assert!(
                    skill.allows(tool),
                    "{kind} must allow {} for code navigation",
                    tool.label()
                );
            }
            let reg = skill.filter_registry(ToolRegistry::standard());
            assert!(
                reg.find("find_symbol").is_some(),
                "{kind} keeps find_symbol"
            );
            assert!(reg.find("read_range").is_some(), "{kind} keeps read_range");
        }
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
    fn gate_always_passes_external_mcp_labels() {
        // `mcp__*` is outside the builtin allowlist by design: external tools
        // are never narrowed by a skill — Permission still governs them.
        let skill = skill_of(TaskType::CodeReview);
        assert!(!skill.allowed_tools.is_empty(), "fixture must have an allowlist");
        assert!(skill.allows_label("mcp__fake__echo"));
        assert!(skill
            .gate(ToolCallId::new("m1"), "mcp__fake__echo", "{}")
            .is_none());
        // Unknown non-external labels still fall through to rejection.
        assert!(!skill.allows_label("drop_database"));
        // filter_registry keeps externals while narrowing builtins.
        let reg = skill.filter_registry(
            crate::protocol::ToolRegistry::standard().with_externals(vec![
                crate::protocol::ExternalToolDef {
                    name: "mcp__fake__echo".to_owned(),
                    description: "echo".to_owned(),
                    input_schema: serde_json::json!({"type": "object"}),
                },
            ]),
        );
        assert_eq!(reg.external_definitions().len(), 1);
        assert!(reg.find_external("mcp__fake__echo").is_some());
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

    // -----------------------------------------------------------------------
    // User skill store
    // -----------------------------------------------------------------------

    /// Fresh temp dir per test (name-scoped so parallel tests never collide).
    fn temp_store(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kodo_user_skills_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    const USER_MD: &str = "# Skill: my-notes\n\
        ## description\nSummarize materials into short notes.\n\
        ## applicable_task_types\n- work\n\
        ## context_strategy\nwork-focused\n\
        ## allowed_tools\n- read_file\n\
        ## verification_policy\nnone\n\
        ## workflow\n- s1 | read | Read the input\n\
        ## completion_criteria\n- The notes are complete\n\
        ## guidance\nKeep it short.\n";

    /// Same skill without `## description` (the pre-description on-disk shape).
    const USER_MD_NO_DESC: &str = "# Skill: my-notes\n\
        ## applicable_task_types\n- work\n\
        ## context_strategy\nwork-focused\n\
        ## allowed_tools\n- read_file\n\
        ## verification_policy\nnone\n\
        ## workflow\n- s1 | read | Read the input\n\
        ## completion_criteria\n- The notes are complete\n\
        ## guidance\nKeep it short.\n";

    #[test]
    fn description_falls_back_to_first_criterion() {
        let spec = parse_skill(USER_MD_NO_DESC).expect("legacy shape still parses");
        assert_eq!(spec.description, "The notes are complete");
        let spec = parse_skill(USER_MD).expect("explicit description");
        assert_eq!(spec.description, "Summarize materials into short notes.");
        // Long criterion truncates to 120 chars.
        let mut long_criterion = String::from("- ");
        long_criterion.push_str(&"x".repeat(200));
        let long = USER_MD_NO_DESC.replace("- The notes are complete", &long_criterion);
        let spec = parse_skill(&long).expect("long criterion");
        assert_eq!(spec.description.chars().count(), 120);
    }

    #[test]
    fn save_user_skill_requires_explicit_description() {
        let dir = temp_store("desc");
        let err = save_user_skill(&dir, "my-notes", USER_MD_NO_DESC)
            .unwrap_err()
            .to_string();
        assert!(err.contains("description"), "got: {err}");
        // Legacy file on disk still loads (parse is tolerant).
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("my-notes.md"), USER_MD_NO_DESC).expect("write");
        let loaded = load_user_skills(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].description, "The notes are complete");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_user_skill_round_trips_and_enforces_invariants() {
        let dir = temp_store("roundtrip");
        let spec = save_user_skill(&dir, "my-notes", USER_MD).expect("save");
        assert_eq!(spec.name, "my-notes");
        assert!(dir.join("my-notes.md").exists());
        let loaded = load_user_skills(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "my-notes");

        // Bad charset / builtin collision / title mismatch are refused.
        assert!(save_user_skill(&dir, "bad name!", USER_MD).is_err());
        assert!(save_user_skill(&dir, "work", USER_MD)
            .unwrap_err()
            .contains("内置"));
        let wrong_title = USER_MD.replace("my-notes", "other-name");
        assert!(save_user_skill(&dir, "my-notes", &wrong_title)
            .unwrap_err()
            .contains("不一致"));
        assert!(save_user_skill(&dir, "nope", "not a skill").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_user_skills_skips_unusable_files() {
        let dir = temp_store("skip");
        std::fs::create_dir_all(&dir).expect("dir");
        // File name = chip id = `# Skill:` title (the save() invariant).
        std::fs::write(dir.join("my-notes.md"), USER_MD).expect("good");
        std::fs::write(dir.join("broken.md"), "no title").expect("broken");
        // Stem ≠ title: chip id would miss on lookup — skipped.
        std::fs::write(dir.join("other-name.md"), USER_MD).expect("mismatch");
        // Builtin collision (stem AND title = builtin name): must never
        // shadow an embedded skill.
        std::fs::write(dir.join("work.md"), USER_MD.replace("my-notes", "work")).expect("builtin");
        // Bad stem charset.
        std::fs::write(dir.join("bad name.md"), USER_MD).expect("bad stem");
        // Non-markdown files ignored.
        std::fs::write(dir.join("readme.txt"), USER_MD).expect("txt");

        let loaded = load_user_skills(&dir);
        assert_eq!(loaded.len(), 1, "only my-notes.md loads: {:?}", loaded);
        assert_eq!(loaded[0].name, "my-notes");

        assert!(load_user_skills(&dir.join("missing")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_user_skill_removes_only_user_files() {
        let dir = temp_store("delete");
        save_user_skill(&dir, "my-notes", USER_MD).expect("save");
        assert!(delete_user_skill(&dir, "my-notes").is_ok());
        assert!(!dir.join("my-notes.md").exists());
        assert!(delete_user_skill(&dir, "my-notes")
            .unwrap_err()
            .contains("不存在"));
        assert!(delete_user_skill(&dir, "work")
            .unwrap_err()
            .contains("内置"));
        assert!(delete_user_skill(&dir, "bad name").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_user_appends_after_builtins_without_hijacking_select() {
        let user = parse_skill(USER_MD).expect("user spec");
        let reg = SkillRegistry::builtin().with_user(vec![user]);
        assert_eq!(reg.skills().len(), 9);
        // select() first-match still lands on builtins…
        assert_eq!(reg.select(TaskType::Work).unwrap().name, "work");
        // …while get() reaches the user skill (the chip path).
        assert!(reg.get("my-notes").is_some());
        assert!(SkillRegistry::builtin().get("my-notes").is_none());
    }

    #[test]
    fn earlier_extras_shadow_later_ones_on_name_collision() {
        // build_skill_registry appends project before user: the project copy
        // wins `get()` (chip path) deterministically.
        let project = parse_skill(
            &USER_MD.replace("Summarize materials into short notes.", "Project copy."),
        )
        .expect("project spec");
        let user = parse_skill(USER_MD).expect("user spec");
        let reg = SkillRegistry::builtin().with_user(vec![project]).with_user(vec![user]);
        let hit = reg.get("my-notes").expect("collision resolves");
        assert_eq!(hit.description, "Project copy.");
    }
}
