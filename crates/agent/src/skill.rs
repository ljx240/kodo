//! SkillSpec: structured, version-controllable skills that steer the real
//! Planner, Verifier, and tool gate — not a Markdown blob in the prompt.
//!
//! Skill files live at `skills/<name>/SKILL.md` and use section headers the
//! parser understands (`## applicable_task_types`, `## workflow`, …). The nine
//! built-in skills are embedded via `include_str!` so the registry is
//! available with zero filesystem I/O.

use std::path::{Path, PathBuf};

use kodo_core::skill_paths::{SkillRoot, SkillScope};

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
    /// Work / docs / summaries / Q&A: light docs-style context, no
    /// reproduction or acceptance-gated build/test pipeline.
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
            // Reviews target the named files; slightly tighter packing.
            Self::ReviewOnly => ContextBudget {
                max_spans: 6,
                ..ContextBudget::default()
            },
            // Work / Q&A / summaries: docs-light, no repro or build refs.
            Self::WorkFocused => ContextBudget {
                max_spans: 4,
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
    pub allowed_external_tools: Vec<String>,
    pub workflow: Vec<WorkflowStep>,
    pub completion_criteria: Vec<String>,
    pub verification_policy: VerificationPolicy,
    /// Plan-level gate: the run must end with a final assistant message
    /// before the loop falls back to `model_gap`. Read-only review skills
    /// (`code-review`) opt in so that a long reasoning pass that lands
    /// with an empty `text` block gets one extra user-ask instead of a
    /// hard `provider_error`. Default `false` — one-shot semantics
    /// preserved for conversational / work / bug-fix runs.
    pub requires_final_answer: bool,
    /// Short human-readable description (from the SKILL.md `## description`
    /// section). Optional — older built-ins (bug-fix, feature, …) didn't
    /// ship one. Reaches the system prompt via `skill_prompt_block`.
    pub description: String,
    /// Short human guidance (kept in the file; only compact excerpts reach prompts).
    pub guidance: String,
    /// Loaded from the Agent Skills convention (frontmatter `SKILL.md`)
    /// rather than Kodo's section format. These skills contribute guidance
    /// and an optional tool allowlist; they never replace Kodo's plan,
    /// verification policy, or permission gates.
    pub guidance_only: bool,
    /// Root the skill came from (`builtin`, `user`, `project`, `claude`,
    /// `plugin:codex`, …). Empty for skills parsed directly.
    pub source: String,
}

impl SkillSpec {
    /// Skill-level tool allowlist. **Strictly narrower** than Permission:
    /// allowing a tool here never bypasses `Permission::needs_approval`.
    pub fn allows(&self, tool: ToolName) -> bool {
        self.allowed_tools.contains(&tool)
    }

    pub fn allows_label(&self, label: &str) -> bool {
        // An external skill that declares no tool list is documentation, not
        // a contract: it must not accidentally hide every tool from the
        // model. A declared list still narrows (and Permission still gates).
        if self.guidance_only
            && self.allowed_tools.is_empty()
            && self.allowed_external_tools.is_empty()
        {
            return true;
        }
        ToolName::parse(label)
            .map(|t| self.allows(t))
            .unwrap_or_else(|| {
                self.context_strategy != ContextStrategy::ReviewOnly
                    && label.starts_with("mcp__")
                    && self.allowed_external_tools.iter().any(|pattern| {
                        pattern
                            .strip_suffix('*')
                            .map_or(pattern == label, |prefix| label.starts_with(prefix))
                    })
            })
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

    /// `true` only when the skill deliberately verifies nothing (docs, review).
    /// Distinct from "this project has no manifests to infer from" — callers
    /// must not treat the two the same way.
    pub fn verifies_nothing(&self) -> bool {
        matches!(self.verification_policy, VerificationPolicy::None)
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

pub fn section_body<'a>(lines: &[&'a str], header: &str) -> Vec<&'a str> {
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

pub fn single_line(body: &[&str]) -> String {
    body.iter()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// Parse a single-line `true` / `false` flag. Empty input → `default`;
/// anything else must be exactly `true` or `false` (case-insensitive) to
/// return `Some`. `None` signals a parse error so the caller can fail
/// loudly instead of silently flipping a skill-level gate.
fn parse_bool_flag(raw: &str, default: bool) -> Option<bool> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(default);
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    }
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

    let allowed_external_tools = bullets(&section_body(&lines, "allowed_external_tools"));
    if allowed_external_tools.iter().any(|name| {
        !name.starts_with("mcp__") || name[..name.len().saturating_sub(1)].contains('*')
    }) {
        return Err(
            "external allowlist must use mcp__ wire names or a trailing prefix wildcard".into(),
        );
    }

    let policy_raw = single_line(&section_body(&lines, "verification_policy"));
    let verification_policy = VerificationPolicy::parse(&policy_raw)
        .ok_or_else(|| format!("unknown verification_policy `{policy_raw}`"))?;

    let requires_final_answer = parse_bool_flag(
        &single_line(&section_body(&lines, "requires_final_answer")),
        false,
    )
    .ok_or_else(|| "`requires_final_answer` must be `true` or `false` (single line)".to_owned())?;

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

    let description = single_line(&section_body(&lines, "description"));

    // No explicit `## description` — fall back to the first completion
    // criterion so the system prompt still has a one-line summary.
    let description = if description.is_empty() {
        completion_criteria.first().cloned().unwrap_or_default()
    } else {
        description
    };

    Ok(SkillSpec {
        name,
        applicable_task_types: task_types,
        context_strategy,
        allowed_tools,
        allowed_external_tools,
        workflow,
        completion_criteria,
        verification_policy,
        requires_final_answer,
        description,
        guidance,
        guidance_only: false,
        source: String::new(),
    })
}

// ---------------------------------------------------------------------------
// Agent Skills adapter — frontmatter `SKILL.md`, no extra dependencies.
// ---------------------------------------------------------------------------

/// Anything larger is not a skill body; skip the file instead of feeding a
/// multi-megabyte prompt.
pub const MAX_SKILL_BYTES: u64 = 64 * 1024;
const MAX_GUIDANCE_CHARS: usize = 32 * 1024;
const MAX_DESCRIPTION_CHARS: usize = 500;
const MAX_NAME_CHARS: usize = 64;
/// How deep a root is walked for grouped layouts (`skills/ai/<name>/SKILL.md`).
/// A directory with its own `SKILL.md` is a leaf and is never descended.
const MAX_SKILL_SCAN_DEPTH: usize = 3;

/// How a skill file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillFormat {
    /// Kodo's section format (`# Skill: name`, `## workflow`, …).
    Kodo,
    /// Agent Skills frontmatter (`--- name: … ---`) plus a Markdown body.
    Agent,
}

impl SkillFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Kodo => "kodo",
            Self::Agent => "agent",
        }
    }
}

/// A loaded skill plus the file it came from — what the 插件 page lists.
/// The runtime only reads `spec`; `markdown` and `path` drive the preview,
/// `read_only` drives the edit gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRecord {
    pub spec: SkillSpec,
    pub markdown: String,
    pub source: String,
    pub scope: SkillScope,
    pub path: PathBuf,
    pub read_only: bool,
    pub format: SkillFormat,
}

/// `--- … ---` at the very top of the file. Returns `(frontmatter, body)`.
fn split_frontmatter(markdown: &str) -> Option<(&str, &str)> {
    let first_end = markdown.find('\n')?;
    if markdown[..first_end].trim_end() != "---" {
        return None;
    }
    let mut start = first_end + 1;
    while start <= markdown.len() {
        let end = markdown[start..]
            .find('\n')
            .map_or(markdown.len(), |offset| start + offset);
        if markdown[start..end].trim_end() == "---" {
            let body_start = if end < markdown.len() { end + 1 } else { end };
            return Some((&markdown[first_end + 1..start], &markdown[body_start..]));
        }
        if end == markdown.len() {
            break;
        }
        start = end + 1;
    }
    None
}

fn unquote(raw: &str) -> String {
    let raw = raw.trim();
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        let quoted = (bytes[0] == b'"' && bytes[raw.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[raw.len() - 1] == b'\'');
        if quoted {
            return raw[1..raw.len() - 1].trim().to_owned();
        }
    }
    raw.to_owned()
}

/// Minimal YAML subset: scalar `key: value`, `- item` lists, and folded
/// (`>`) or literal (`|`) blocks. Nested maps are parsed as their flattened
/// keys and simply never read — no dependency, no pretence of full YAML.
fn frontmatter_fields(front: &str) -> Vec<(String, String)> {
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut last_key: Option<String> = None;
    for line in front.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            if let Some(key) = last_key.as_ref() {
                if let Some((_, value)) = fields.iter_mut().find(|(name, _)| name == key) {
                    if !value.is_empty() {
                        value.push('\n');
                    }
                    value.push_str(&unquote(item.trim()));
                }
            }
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if indented {
            // Folded scalar continuation (`description: >` + indented lines).
            if let Some(key) = last_key.as_ref() {
                if let Some((_, value)) = fields.iter_mut().find(|(name, _)| name == key) {
                    if matches!(value.as_str(), ">" | "|" | ">-" | "|-") {
                        value.clear();
                    }
                    if !value.is_empty() {
                        value.push(' ');
                    }
                    value.push_str(trimmed);
                }
            }
            continue;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim().to_ascii_lowercase().replace('-', "_");
            if key.is_empty() {
                continue;
            }
            fields.push((key.clone(), unquote(value.trim())));
            last_key = Some(key);
        }
    }
    fields
}

fn field<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(name, value)| name == key && !value.is_empty())
        .map(|(_, value)| value.as_str())
}

/// Comma-, bracket- and newline-separated list entries.
fn field_list(fields: &[(String, String)], key: &str) -> Vec<String> {
    fields
        .iter()
        .filter(|(name, _)| name == key)
        .flat_map(|(_, value)| {
            let value = value.trim();
            let value = value
                .strip_prefix('[')
                .and_then(|inner| inner.strip_suffix(']'))
                .unwrap_or(value);
            value
                .split(|c| c == ',' || c == '\n')
                .map(|part| unquote(part.trim()))
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// Keep the chip id in the same alphabet `split_skill_chip` accepts, so an
/// external skill named `PDF Tools` still round-trips through the Composer.
fn sanitize_skill_name(raw: &str) -> Option<String> {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_NAME_CHARS).collect())
}

fn agent_tool_names(raw: &str) -> Vec<ToolName> {
    // Claude-style scoping (`Bash(git:*)`) keeps the tool name before `(`.
    let base = raw.split('(').next().unwrap_or(raw).trim();
    match base.to_ascii_lowercase().as_str() {
        "read" | "read_file" | "readfile" => vec![ToolName::ReadFile],
        "grep" | "search" => vec![ToolName::Search],
        "glob" | "list_files" | "ls" => vec![ToolName::ListFiles],
        "write" | "write_file" | "writefile" | "create" | "create_file" => {
            vec![ToolName::WriteFile, ToolName::CreateFile]
        }
        "edit" | "multiedit" | "multi_edit" | "apply_patch" | "patch" => {
            vec![ToolName::ApplyPatch, ToolName::ReplaceRange]
        }
        "bash" | "run_command" | "command" | "shell" => vec![ToolName::RunCommand],
        "websearch" | "web_search" | "webfetch" | "web_fetch" => vec![ToolName::WebSearch],
        _ => Vec::new(),
    }
}

fn agent_external_tool(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if !raw.starts_with("mcp__") {
        return None;
    }
    if let Some(prefix) = raw.strip_suffix('*') {
        return (!prefix.contains('*')).then(|| raw.to_owned());
    }
    // `mcp__github` names a server; Kodo's gate matches tool prefixes.
    if raw["mcp__".len()..].contains("__") {
        Some(raw.to_owned())
    } else {
        Some(format!("{raw}__*"))
    }
}

fn description_from_body(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with("```")
                && !line.starts_with("- ")
                && !line.starts_with("* ")
        })
        .map(|line| truncate_chars(line, MAX_DESCRIPTION_CHARS))
        .unwrap_or_default()
}

/// Parse the standard Agent Skills shape into Kodo's runtime contract.
///
/// The body becomes `guidance`; `allowed-tools` maps onto Kodo's tool names
/// when the author declared them. Everything Kodo owns — planning, verify,
/// permission — keeps its default behaviour, which the run pipeline expresses
/// with `guidance_only = true`.
pub fn parse_agent_skill(markdown: &str, fallback_name: &str) -> Result<SkillSpec, String> {
    let (front, body) =
        split_frontmatter(markdown).ok_or("missing YAML frontmatter (`--- name: … ---`)")?;
    let fields = frontmatter_fields(front);

    let raw_name = field(&fields, "name")
        .map(str::to_owned)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| fallback_name.to_owned());
    let name = sanitize_skill_name(&raw_name)
        .ok_or_else(|| format!("skill name `{raw_name}` is not usable as a chip id"))?;

    let mut task_types = Vec::new();
    for raw in field_list(&fields, "task_types")
        .into_iter()
        .chain(field_list(&fields, "applicable_task_types"))
    {
        for part in raw.split(|c: char| c == ',' || c.is_whitespace()) {
            if let Some(kind) = TaskType::parse(part) {
                if !task_types.contains(&kind) {
                    task_types.push(kind);
                }
            }
        }
    }
    let space = field(&fields, "space")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if task_types.is_empty() {
        task_types.push(if space == "work" {
            TaskType::Work
        } else {
            TaskType::Feature
        });
    }

    let mut allowed_tools: Vec<ToolName> = Vec::new();
    let mut allowed_external_tools: Vec<String> = Vec::new();
    for raw in field_list(&fields, "allowed_tools") {
        if raw.starts_with("mcp__") {
            if let Some(pattern) = agent_external_tool(&raw) {
                if !allowed_external_tools.contains(&pattern) {
                    allowed_external_tools.push(pattern);
                }
            }
            continue;
        }
        for tool in agent_tool_names(&raw) {
            if !allowed_tools.contains(&tool) {
                allowed_tools.push(tool);
            }
        }
    }

    let description = field(&fields, "description")
        .map(|text| truncate_chars(text, MAX_DESCRIPTION_CHARS))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| description_from_body(body));
    let guidance = truncate_chars(body.trim(), MAX_GUIDANCE_CHARS);
    let completion_criteria = vec![if description.is_empty() {
        format!("Follow the `{name}` skill guidance")
    } else {
        description.clone()
    }];

    Ok(SkillSpec {
        name,
        applicable_task_types: task_types.clone(),
        context_strategy: if task_types.contains(&TaskType::Work) {
            ContextStrategy::WorkFocused
        } else {
            ContextStrategy::MinimalChange
        },
        allowed_tools,
        allowed_external_tools,
        workflow: Vec::new(),
        completion_criteria,
        verification_policy: if task_types.contains(&TaskType::Work) {
            VerificationPolicy::None
        } else {
            VerificationPolicy::Full
        },
        requires_final_answer: false,
        description,
        guidance,
        guidance_only: true,
        source: String::new(),
    })
}

fn has_kodo_title(markdown: &str) -> bool {
    markdown
        .lines()
        .any(|line| line.trim().starts_with("# Skill:"))
}

/// Skill files under one root. Directory layouts may be grouped one or two
/// levels (`skills/<name>/SKILL.md`, `skills/ai/<name>/SKILL.md`); a
/// directory with its own `SKILL.md` is a leaf. Flat `<name>.md` files are
/// accepted at the root only.
fn collect_skill_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_SKILL_SCAN_DEPTH {
        return;
    }
    if depth > 0 && dir.join("SKILL.md").is_file() {
        out.push(dir.join("SKILL.md"));
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    children.sort();
    for path in children {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_skill_files(&path, depth + 1, out);
        } else if depth == 0 && path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

fn load_skill_file(path: &Path, root: &SkillRoot) -> Option<SkillRecord> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_SKILL_BYTES {
        return None;
    }
    let markdown = std::fs::read_to_string(path).ok()?;
    if markdown.len() as u64 > MAX_SKILL_BYTES {
        return None;
    }
    let (format, mut spec) = if has_kodo_title(&markdown) {
        (SkillFormat::Kodo, parse_skill(&markdown).ok()?)
    } else {
        let fallback = if path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
            path.parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
        } else {
            path.file_stem().and_then(|name| name.to_str())
        }
        .unwrap_or_default();
        (
            SkillFormat::Agent,
            parse_agent_skill(&markdown, fallback).ok()?,
        )
    };
    spec.source = root.source.clone();
    Some(SkillRecord {
        spec,
        markdown,
        source: root.source.clone(),
        scope: root.scope,
        path: path.to_path_buf(),
        read_only: root.read_only,
        format,
    })
}

/// Every skill readable under `root`, sorted by name then path.
/// Unreadable, oversized and malformed files are skipped — one broken skill
/// must not hide the rest of the directory.
pub fn discover_skills(root: &SkillRoot) -> Vec<SkillRecord> {
    let mut files = Vec::new();
    collect_skill_files(&root.path, 0, &mut files);
    let mut out: Vec<SkillRecord> = files
        .iter()
        .filter_map(|path| load_skill_file(path, root))
        .collect();
    out.sort_by(|a, b| {
        a.spec
            .name
            .cmp(&b.spec.name)
            .then_with(|| a.path.cmp(&b.path))
    });
    out
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Embedded in dispatch order: `select(task_type)` returns the **first** skill
/// that declares the type, so the seven auto-selected skills come first and
/// `kodo-ui` is last — it declares code task types only so its chip resolves,
/// never so classification picks it on its own.
const SOURCES: [&str; 9] = [
    include_str!("../../../skills/bug-fix/SKILL.md"),
    include_str!("../../../skills/feature/SKILL.md"),
    include_str!("../../../skills/test/SKILL.md"),
    include_str!("../../../skills/refactor/SKILL.md"),
    include_str!("../../../skills/code-review/SKILL.md"),
    include_str!("../../../skills/docs/SKILL.md"),
    include_str!("../../../skills/work/SKILL.md"),
    include_str!("../../../skills/workflow/SKILL.md"),
    include_str!("../../../skills/kodo-ui/SKILL.md"),
];

/// Selects the active skill for a classified task type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRegistry {
    skills: Vec<SkillSpec>,
    /// Names that came in via [`builtin`] and must not be replaced by user
    /// or project overlays (a chip install that names itself "work" could
    /// hijack the dispatch — builtins are anchored).
    builtin_names: std::collections::HashSet<String>,
}

impl SkillRegistry {
    /// The nine built-in skills, embedded at compile time.
    pub fn builtin() -> Self {
        let skills: Vec<SkillSpec> = SOURCES
            .iter()
            .map(|src| {
                let mut spec = parse_skill(src).expect("built-in SKILL.md must parse");
                spec.source = "builtin".to_owned();
                spec
            })
            .collect();
        let builtin_names = skills.iter().map(|s| s.name.clone()).collect();
        Self {
            skills,
            builtin_names,
        }
    }

    pub fn from_skills(skills: Vec<SkillSpec>) -> Self {
        Self {
            skills,
            builtin_names: std::collections::HashSet::new(),
        }
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

    /// Append user/project skills. Later calls win on name collisions among
    /// user/project skills (project shadows user); **builtin names are
    /// anchored** — a user or project skill that names itself after a
    /// builtin is silently dropped so a chip can't hijack dispatch.
    /// Caller passes in increasing priority order:
    /// `builtin().with_user(user).with_user(project)`. Returns a fresh
    /// registry; the receiver is not mutated.
    pub fn with_user(mut self, skills: Vec<SkillSpec>) -> Self {
        for s in skills {
            if self.builtin_names.contains(&s.name) {
                // Anchor: builtin name stays untouched. A chip install that
                // happens to pick a builtin name is filtered here, not at
                // load_user_skills time, so the helper can keep doing plain
                // directory walks.
                continue;
            }
            if let Some(slot) = self
                .skills
                .iter_mut()
                .find(|existing| existing.name == s.name)
            {
                *slot = s;
            } else {
                self.skills.push(s);
            }
        }
        self
    }
}

/// Built-in skill preview uses the same embedded source as execution.
pub fn builtin_markdown(name: &str) -> Option<&'static str> {
    SOURCES.iter().copied().find(|src| {
        src.lines().any(|line| {
            line.trim()
                .strip_prefix("# Skill:")
                .is_some_and(|title| title.trim() == name)
        })
    })
}

// User-skill IO. Built-in skills live in `crates/agent/skills/*` (compiled in
// via `include_str!`); user/project skills live on disk under
// `<base>/.kolo/skills/<name>/SKILL.md`. `load_user_skills` scans the
// directory and skips anything that fails to parse — same shape as the
// persona store.

fn skill_file(base: &Path, name: &str) -> PathBuf {
    base.join(format!("{name}.md"))
}

fn load_skill_dir(dir: &Path) -> Vec<SkillSpec> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // Two layouts accepted: flat file `<dir>/<name>.md` and directory
        // layout `<dir>/<name>/SKILL.md`. The flat shape is what the test
        // harness and older drafts used; the directory shape mirrors the
        // built-in skill layout so a user can `cp -r skills/foo my-skills/`
        // and have it just work.
        let md = if path.is_dir() {
            let inside = path.join("SKILL.md");
            if inside.is_file() {
                inside
            } else {
                continue;
            }
        } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
            path
        } else {
            continue;
        };
        let Ok(src) = std::fs::read_to_string(&md) else {
            continue;
        };
        if let Ok(s) = parse_skill(&src) {
            out.push(s);
        }
    }
    out
}

/// Load every parseable SKILL.md under `<dir>` (both `<dir>/<name>.md`
/// flat files and `<dir>/<name>/SKILL.md` directory layout). Used both
/// for the user's global store and the project-scoped store. Returns an
/// empty Vec when the directory doesn't exist or is empty.
pub fn load_user_skills(dir: &Path) -> Vec<SkillSpec> {
    load_skill_dir(dir)
}

pub fn save_user_skill(dir: &Path, name: &str, markdown: &str) -> Result<SkillSpec, String> {
    let path = skill_file(dir, name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, markdown).map_err(|e| e.to_string())?;
    parse_skill(markdown)
}

pub fn delete_user_skill(dir: &Path, name: &str) -> Result<(), String> {
    let path = skill_file(dir, name);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
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
        assert_eq!(reg.skills().len(), 9);
    }

    /// `kodo-ui` is a real builtin (chip + preview + reserved name) but must
    /// never win an auto-classified dispatch, and the embedded set must match
    /// the frontend's display lists. Pin both here: adding a builtin means
    /// updating `apps/desktop/src/data/skills.ts` in the same commit.
    #[test]
    fn kodo_ui_is_chip_only_and_the_builtin_set_is_pinned() {
        let reg = registry();
        let names: Vec<&str> = reg.skills().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "bug-fix",
                "feature",
                "test",
                "refactor",
                "code-review",
                "docs",
                "work",
                "workflow",
                "kodo-ui",
            ]
        );
        for kind in [TaskType::Feature, TaskType::BugFix, TaskType::Refactor] {
            assert_eq!(
                reg.select(kind).map(|s| s.name.as_str()),
                Some(match kind {
                    TaskType::Feature => "feature",
                    TaskType::BugFix => "bug-fix",
                    _ => "refactor",
                }),
                "{kind} must not auto-dispatch to kodo-ui"
            );
        }
        // …but the chip and the preview both reach it.
        assert!(reg.get("kodo-ui").is_some());
        assert!(builtin_markdown("kodo-ui").is_some());
        // A work-type chip would be filtered out of code space by
        // `resolve_skill`; kodo-ui declares code types only.
        assert!(!reg
            .get("kodo-ui")
            .unwrap()
            .applicable_task_types
            .contains(&TaskType::Work));
        // Anchored: a user file named kodo-ui can't shadow it.
        let overlay = parse_skill(
            "# Skill: kodo-ui\n## applicable_task_types\n- feature\n## context_strategy\nacceptance-first\n## allowed_tools\n- read_file\n## verification_policy\nnone\n## workflow\n- s1 | read | hijack\n## completion_criteria\n- done\n## guidance\nx\n",
        )
        .unwrap();
        let anchored = SkillRegistry::builtin().with_user(vec![overlay]);
        assert_eq!(
            anchored.get("kodo-ui").unwrap().guidance,
            reg.get("kodo-ui").unwrap().guidance
        );
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

    /// code-review is the only built-in that opts into the final-answer
    /// gate. Other skills (work / bug-fix / feature) keep the original
    /// one-shot semantics — a long reasoning pass with an empty `text`
    /// block still falls back to `model_gap` on them. This test pins the
    /// distinction so a future edit to SKILL.md can't silently flip the
    /// gate on or off without the test catching it.
    #[test]
    fn code_review_is_the_only_skill_with_requires_final_answer() {
        let reg = registry();
        let review = skill_of(TaskType::CodeReview);
        assert!(
            review.requires_final_answer,
            "code-review must require a final assistant message"
        );
        for skill in reg.skills() {
            if skill.name == "code-review" {
                continue;
            }
            assert!(
                !skill.requires_final_answer,
                "{} must keep the one-shot empty-final path",
                skill.name
            );
        }
    }

    /// Parser round-trip: a SKILL.md with `## requires_final_answer` set
    /// to `true` / `false` / absent behaves the way plan.rs expects. An
    /// invalid value is a hard parse error so a typo never silently flips
    /// the gate.
    #[test]
    fn requires_final_answer_parses_and_validates() {
        let on = "# Skill: x\n## applicable_task_types\n- code-review\n## context_strategy\nreview-only\n## allowed_tools\n- read_file\n## verification_policy\nnone\n## requires_final_answer\ntrue\n## workflow\n- s1 | read | gather\n## completion_criteria\n- done\n## guidance\nx\n";
        let off = on.replace(
            "## requires_final_answer\ntrue",
            "## requires_final_answer\nfalse",
        );
        let missing = on.replace(
            "## verification_policy\nnone\n## requires_final_answer\ntrue\n",
            "## verification_policy\nnone\n",
        );
        let bogus = on.replace(
            "## requires_final_answer\ntrue",
            "## requires_final_answer\nmaybe",
        );
        assert!(parse_skill(on).unwrap().requires_final_answer);
        assert!(!parse_skill(&off).unwrap().requires_final_answer);
        assert!(!parse_skill(&missing).unwrap().requires_final_answer);
        assert!(parse_skill(&bogus).is_err());
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
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kodo-skill-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn write_file(root: &Path, relative: &str, body: &str) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).expect("skill parent");
        std::fs::write(path, body).expect("skill file");
    }

    /// Agent Skills frontmatter maps onto Kodo's contract: the body becomes
    /// guidance, `allowed-tools` narrows the tool list, and the plan keeps
    /// Kodo's defaults (`guidance_only`).
    #[test]
    fn agent_skill_frontmatter_maps_tools_and_guidance() {
        let markdown = "---\n\
name: PDF Tools!\n\
description: \"Work with PDF files\"\n\
allowed-tools:\n\
  - Read\n\
  - Bash(git:*)\n\
  - mcp__github\n\
---\n\
\n\
# PDF\n\
Use pdfplumber for extraction.\n";
        let spec = parse_agent_skill(markdown, "fallback").expect("agent skill");
        assert_eq!(spec.name, "PDF-Tools");
        assert_eq!(spec.description, "Work with PDF files");
        assert!(spec.guidance_only);
        assert!(spec.workflow.is_empty());
        assert!(spec.allows(ToolName::ReadFile));
        assert!(spec.allows(ToolName::RunCommand));
        assert!(!spec.allows(ToolName::WriteFile));
        assert_eq!(spec.allowed_external_tools, vec!["mcp__github__*"]);
        assert!(spec.allows_label("mcp__github__search"));
        assert!(spec.guidance.contains("pdfplumber"));
        assert_eq!(spec.applicable_task_types, vec![TaskType::Feature]);
        assert_eq!(spec.verification_policy, VerificationPolicy::Full);
    }

    /// No tool list = documentation, not an allowlist. The turn must keep
    /// every tool; otherwise selecting a prose-only skill would brick the run.
    #[test]
    fn agent_skill_without_a_tool_list_is_unrestricted() {
        let spec = parse_agent_skill(
            "---\nname: notes\ndescription: Write notes\n---\nKeep it short.\n",
            "notes",
        )
        .unwrap();
        assert!(spec.allowed_tools.is_empty());
        assert!(spec.allows_label("write_file"));
        assert!(spec.allows_label("run_command"));
        assert!(spec.allows_label("mcp__files__read"));
        let registry = spec.filter_registry(ToolRegistry::standard());
        assert!(registry.find("write_file").is_some());
        assert!(registry.find("run_command").is_some());
    }

    #[test]
    fn agent_skill_space_work_uses_work_policy() {
        let spec = parse_agent_skill(
            "---\nname: deck\ndescription: Build a deck\nspace: work\n---\nBody.\n",
            "deck",
        )
        .unwrap();
        assert_eq!(spec.applicable_task_types, vec![TaskType::Work]);
        assert_eq!(spec.context_strategy, ContextStrategy::WorkFocused);
        assert_eq!(spec.verification_policy, VerificationPolicy::None);
    }

    #[test]
    fn agent_skill_requires_frontmatter_and_a_usable_name() {
        assert!(parse_agent_skill("# just markdown\nbody\n", "dir").is_err());
        assert!(parse_agent_skill("---\nname: !!!\n---\nbody\n", "dir").is_err());
        let named_by_dir =
            parse_agent_skill("---\ndescription: From the folder\n---\nbody\n", "my-skill")
                .unwrap();
        assert_eq!(named_by_dir.name, "my-skill");
    }

    /// Grouped layouts (`skills/ai/<name>/SKILL.md`), flat files, Kodo and
    /// Agent formats coexist in one root; broken files are skipped.
    #[test]
    fn discover_skills_reads_grouped_and_flat_layouts() {
        let dir = temp_dir("discover");
        write_file(
            &dir,
            "alpha/SKILL.md",
            "---\nname: alpha\ndescription: Alpha skill\nallowed-tools:\n  - Read\n  - Grep\n---\n# Alpha\nDo alpha things.\n",
        );
        write_file(
            &dir,
            "ai/beta/SKILL.md",
            "---\nname: beta\nspace: work\n---\nBeta body.\n",
        );
        write_file(
            &dir,
            "flat.md",
            "# Skill: flat\n## applicable_task_types\n- docs\n## context_strategy\ndocs-focused\n## allowed_tools\n- read_file\n## verification_policy\nnone\n## workflow\n- s1 | read | read\n## completion_criteria\n- done\n## guidance\nflat guidance\n",
        );
        write_file(&dir, "README.md", "No frontmatter here.");
        write_file(&dir, "broken/SKILL.md", "---\nname: !!!\n---\nbody\n");

        let root = SkillRoot::new("codex", SkillScope::Global, &dir, 11, true);
        let records = discover_skills(&root);
        let names: Vec<&str> = records
            .iter()
            .map(|record| record.spec.name.as_str())
            .collect();
        assert_eq!(names, vec!["alpha", "beta", "flat"]);

        let alpha = &records[0];
        assert_eq!(alpha.format, SkillFormat::Agent);
        assert_eq!(alpha.source, "codex");
        assert_eq!(alpha.spec.source, "codex");
        assert_eq!(alpha.scope, SkillScope::Global);
        assert!(alpha.read_only);
        assert!(alpha.path.ends_with("alpha/SKILL.md"));
        assert!(alpha.spec.allows(ToolName::ReadFile));

        let beta = &records[1];
        assert!(beta.path.ends_with("ai/beta/SKILL.md"));
        assert_eq!(beta.spec.applicable_task_types, vec![TaskType::Work]);

        let flat = &records[2];
        assert_eq!(flat.format, SkillFormat::Kodo);
        assert!(!flat.spec.guidance_only);
        assert!(flat.markdown.contains("flat guidance"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn external_tools_need_a_skill_allowlist_and_review_never_grants_opaque_tools() {
        let source = builtin_markdown("feature").unwrap();
        assert!(source.contains("## allowed_external_tools"));
        let explicit = parse_skill(source).unwrap();
        assert!(explicit.allows_label("mcp__files__read"));
        let closed = parse_skill(&source.replace("- mcp__*", "")).unwrap();
        assert!(!closed.allows_label("mcp__files__read"));
        let review = parse_skill(builtin_markdown("code-review").unwrap()).unwrap();
        assert!(!review.allows_label("mcp__files__read"));
        for skill in SkillRegistry::builtin().skills() {
            let body = builtin_markdown(&skill.name).unwrap();
            assert!(!body.trim().is_empty());
            assert_eq!(parse_skill(body).unwrap().name, skill.name);
        }
    }
}
