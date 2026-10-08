//! AgentSpec: persona records — a name, a one-line description and free-form
//! instructions that the entry point appends to the system prompt when the
//! Composer's agent selector names them. Not subagents: an agent never runs on
//! its own, it only steers the current turn's personality.
//!
//! Files live at `<dir>/<name>.md` (the shell supplies `state_dir()/agents`)
//! with section headers the parser understands. This crate never resolves
//! paths itself, so tests stay hermetic.

use std::path::Path;

use crate::skill::{section_body, single_line};

/// Embedded built-in persona sources, keyed by name, in registry order.
/// Mirrors `BUILTIN_SOURCES` in skill.rs: a static array of
/// `include_str!("../../agents/<name>.md")` so the run never depends on the
/// filesystem. `resolve_instructions` checks builtins before the user dir —
/// user files shadow nothing. File name matches `# Agent:` title (the parser
/// rejects drift).
pub(crate) const BUILTIN_AGENTS: [(&str, &str); 12] = [
    ("reviewer", include_str!("../../../agents/reviewer.md")),
    ("explainer", include_str!("../../../agents/explainer.md")),
    ("debugger", include_str!("../../../agents/debugger.md")),
    ("refactorer", include_str!("../../../agents/refactorer.md")),
    ("tester", include_str!("../../../agents/tester.md")),
    (
        "security-reviewer",
        include_str!("../../../agents/security-reviewer.md"),
    ),
    (
        "perf-reviewer",
        include_str!("../../../agents/perf-reviewer.md"),
    ),
    ("documenter", include_str!("../../../agents/documenter.md")),
    ("planner", include_str!("../../../agents/planner.md")),
    ("api-designer", include_str!("../../../agents/api-designer.md")),
    ("translator", include_str!("../../../agents/translator.md")),
    ("ui-designer", include_str!("../../../agents/ui-designer.md")),
];

/// One built-in persona's raw markdown — read-only preview source.
pub fn builtin_markdown(name: &str) -> Option<&'static str> {
    BUILTIN_AGENTS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, src)| *src)
}

/// Parsed built-in specs in registry order. The 12 markdown files are the
/// single source of truth; `parse_agent` validates them at startup.
pub fn builtin_agents() -> Vec<AgentSpec> {
    BUILTIN_AGENTS
        .iter()
        .map(|(_, src)| parse_agent(src).expect("built-in agent markdown must parse"))
        .collect()
}

/// True when `name` is reserved by a built-in — user files can't shadow it.
pub(crate) fn is_builtin_name(name: &str) -> bool {
    BUILTIN_AGENTS.iter().any(|(n, _)| *n == name)
}

/// Structured persona record. Three contract fields (`name`, `description`,
/// `instructions`) plus optional sections that broaden a persona beyond a
/// plain system-prompt suffix injection:
/// - `defaults`: per-persona model / permission / skill bindings (§2.1-2.3)
/// - `variables`: declared template variables the `## instructions` may use
///   (§2.4) — only listed names are substituted at render time
/// - `examples`: few-shot input/output pairs (§2.5)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpec {
    pub name: String,
    /// One-line `## description` shown in the selector and the list row.
    pub description: String,
    /// `## instructions` body — appended verbatim to the system prompt.
    pub instructions: String,
    pub defaults: PersonaDefaults,
    pub variables: Vec<String>,
    pub examples: Vec<PersonaExample>,
}

/// Session-state bindings a persona carries with it. Every field is
/// optional — when `None` or empty, the composer's choice wins (§2.1-2.3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersonaDefaults {
    /// Provider id (`openai`, `anthropic`, …) the persona wants. When set,
    /// the run-time check verifies it matches the active provider before
    /// applying `model` — cross-provider overrides are ignored.
    pub provider: Option<String>,
    /// Model id (`gpt-4o`, `claude-sonnet-4-5`, …) the persona wants.
    pub model: Option<String>,
    /// Permission level (`"ask"` | `"auto"` | `"full"`); parsed into the
    /// core [`Permission`] enum at the seam.
    pub permission: Option<String>,
    /// Skill ids the persona wants pre-selected. User selections in the
    /// composer are merged on top, never replaced.
    pub skills: Vec<String>,
}

/// One few-shot example used to steer the model in the persona's style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaExample {
    pub input: String,
    pub output: String,
}

/// Lift an `AgentSpec` (the on-disk / built-in shape) into the run-time
/// `PersonaBlock`. Drop `description` — it is selector copy, not part of
/// the injected system prompt.
impl From<AgentSpec> for crate::PersonaBlock {
    fn from(spec: AgentSpec) -> Self {
        crate::PersonaBlock {
            name: spec.name,
            instructions: spec.instructions,
            defaults: spec.defaults,
            variables: spec.variables,
            examples: spec.examples,
        }
    }
}

// ---------------------------------------------------------------------------
// Parser — section-based Markdown, mirroring skill.rs (no extra deps).
// ---------------------------------------------------------------------------

/// Parse one agent markdown file into an [`AgentSpec`]. The three contract
/// sections (`# Agent:`, `## description`, `## instructions`) are required —
/// the optional sections (`## defaults`, `## variables`, `## examples`)
/// default to empty when absent, so all 12 built-ins keep loading without
/// touching their markdown.
pub fn parse_agent(markdown: &str) -> Result<AgentSpec, String> {
    let lines: Vec<&str> = markdown.lines().collect();
    let name = lines
        .iter()
        .find_map(|l| l.trim().strip_prefix("# Agent:"))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("缺少 `# Agent: <name>` 标题")?
        .to_owned();

    let description = single_line(&section_body(&lines, "description"));
    if description.is_empty() {
        return Err("缺少 `## description`".to_owned());
    }

    let instructions = section_body(&lines, "instructions")
        .iter()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned();
    if instructions.is_empty() {
        return Err("缺少 `## instructions`".to_owned());
    }

    let defaults = parse_defaults(&section_body(&lines, "defaults"));
    let variables = parse_variables(&section_body(&lines, "variables"));
    let examples = parse_examples(&section_body(&lines, "examples"));

    Ok(AgentSpec {
        name,
        description,
        instructions,
        defaults,
        variables,
        examples,
    })
}

/// Parse `## defaults` body — `key: value` per line. Unknown keys are
/// ignored (the schema evolves; built-in files don't have to change).
/// Whitespace inside values is trimmed.
fn parse_defaults(body: &[&str]) -> PersonaDefaults {
    let mut defaults = PersonaDefaults::default();
    for line in body {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();
        if value.is_empty() {
            continue;
        }
        match key.as_str() {
            "provider" => defaults.provider = Some(value),
            "model" => defaults.model = Some(value),
            "permission" => {
                if matches!(value.as_str(), "ask" | "auto" | "full") {
                    defaults.permission = Some(value);
                }
            }
            "skills" => {
                defaults.skills = value
                    .split(',')
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            _ => {}
        }
    }
    defaults
}

/// Parse `## variables` body — `- name` per line. Each name must satisfy
/// `valid_variable_name`; the list is returned deduplicated in input order.
fn parse_variables(body: &[&str]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in body {
        let trimmed = line.trim();
        let Some(name) = trimmed.strip_prefix("- ") else {
            continue;
        };
        let name = name.trim();
        if !valid_variable_name(name) || !seen.insert(name.to_owned()) {
            continue;
        }
        out.push(name.to_owned());
    }
    out
}

/// Parse `## examples` body — pairs of `- input: …` / `  output: …`. Each
/// `- input:` starts a new pair; an `output:` indented two spaces closes it.
/// Pairs missing either side are dropped silently.
fn parse_examples(body: &[&str]) -> Vec<PersonaExample> {
    let mut out: Vec<PersonaExample> = Vec::new();
    let mut current: Option<PersonaExample> = None;
    for line in body {
        if let Some(rest) = line.trim_start().strip_prefix("- input:") {
            if let Some(prev) = current.take() {
                out.push(prev);
            }
            current = Some(PersonaExample {
                input: rest.trim().to_owned(),
                output: String::new(),
            });
        } else if let Some(rest) = line.strip_prefix("  output:") {
            if let Some(example) = current.as_mut() {
                let v = rest.trim();
                if !v.is_empty() {
                    example.output = v.to_owned();
                }
            }
        }
    }
    if let Some(prev) = current.take() {
        if !prev.output.is_empty() {
            out.push(prev);
        }
    }
    out
}

/// Variable name charset: snake_case identifiers, 1–32 long. Same shape as
/// shell env vars — no spaces, no braces, no slashes.
fn valid_variable_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

// ---------------------------------------------------------------------------
// User agent store — flat `<dir>/<name>.md`. The directory is always supplied
// by the caller (`agents_dir()` in the shell); this crate stays path-free.
// ---------------------------------------------------------------------------

/// File stems: `[A-Za-z0-9_-]{1,64}` — same rule as skill chip ids.
fn valid_agent_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Load every usable `*.md` in `dir`. Missing dir → empty. Files are skipped
/// when they fail [`parse_agent`], the stem or the `# Agent:` title violates
/// the name rules, or a name already loaded (first wins). Sorted by file name
/// so loading is deterministic.
pub fn load_user_agents(dir: &Path) -> Vec<AgentSpec> {
    load_agents_in(dir, true)
}

/// Project-scope loader for `<root>/.kodo/agents/*.md`. Same rules as
/// `load_user_agents` except project files MAY shadow built-ins — a project
/// persona overrides the registry copy for that project only.
pub fn load_project_agents(project_root: &Path) -> Vec<AgentSpec> {
    let dir = project_root.join(".kodo").join("agents");
    load_agents_in(&dir, false)
}

/// Shared loader. `skip_builtin_shadow=true` is the user-scope rule: a
/// file matching a builtin name is silently dropped to protect built-ins.
/// `false` is the project-scope rule: project files override builtins.
pub fn load_agents_in(dir: &Path, skip_builtin_shadow: bool) -> Vec<AgentSpec> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();
    paths.sort();
    let mut out: Vec<AgentSpec> = Vec::new();
    for path in paths {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !valid_agent_name(stem) {
            continue;
        }
        let Ok(markdown) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(spec) = parse_agent(&markdown) else {
            continue;
        };
        // Layout invariant: file name = selector id = spec.name (save enforces
        // it; tolerate no drift on load so selection lookup can't miss).
        if spec.name != stem {
            continue;
        }
        // User files can't shadow builtins — the run resolves builtin first.
        // Project files CAN shadow builtins (project-local override).
        if skip_builtin_shadow && is_builtin_name(&spec.name) {
            continue;
        }
        if out.iter().any(|a| a.name == spec.name) {
            continue;
        }
        out.push(spec);
    }
    out
}

/// Validate and write one agent to `dir/<name>.md`. The `# Agent:` title must
/// equal `name`. Errors are zh-CN for direct display in the UI. Built-in
/// names are reserved — the UI cannot shadow them.
pub fn save_user_agent(dir: &Path, name: &str, markdown: &str) -> Result<AgentSpec, String> {
    if !valid_agent_name(name) {
        return Err(format!(
            "智能体名无效 `{name}`：仅限字母、数字、- 和 _，最多 64 字符"
        ));
    }
    if is_builtin_name(name) {
        return Err(format!("`{name}` 是内置智能体名，请换一个名称"));
    }
    let spec = parse_agent(markdown)?;
    if spec.name != name {
        return Err(format!(
            "智能体标题 `# Agent: {}` 与名称 `{name}` 不一致",
            spec.name
        ));
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{name}.md")), markdown).map_err(|e| e.to_string())?;
    Ok(spec)
}

/// Remove `dir/<name>.md`. Invalid names are rejected; a missing file is
/// `智能体不存在` (the UI already removed the row — not fatal). Built-in
/// names are reserved and reject the call.
pub fn delete_user_agent(dir: &Path, name: &str) -> Result<(), String> {
    if !valid_agent_name(name) {
        return Err(format!("智能体名无效 `{name}`"));
    }
    if is_builtin_name(name) {
        return Err(format!("内置智能体 `{name}` 不可删除"));
    }
    let path = dir.join(format!("{name}.md"));
    if !path.exists() {
        return Err("智能体不存在".to_owned());
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

/// Resolve one persona's instructions text for prompt injection. This is the
/// unit the shell calls before a run starts — an unknown name must fail the
/// send, not silently drop the persona. Builtins win over a stale user file
/// of the same name (user load also skips them).
pub fn resolve_instructions(dir: &Path, name: &str) -> Result<String, String> {
    resolve_spec(dir, None, name).map(|spec| spec.instructions)
}

/// Resolve one full persona spec (instructions + defaults + variables +
/// examples) for the persona chain seam in `lib.rs::run`. Priority order:
/// project (`<root>/.kodo/agents`) → user (`<dir>/<name>.md`) → registry.
/// Project scope may shadow a built-in name (local override); user scope
/// is filtered by `load_user_agents` so user files can never shadow a
/// built-in name.
pub fn resolve_spec(
    user_dir: &Path,
    project_root: Option<&Path>,
    name: &str,
) -> Result<AgentSpec, String> {
    if !valid_agent_name(name) {
        return Err(format!("智能体名无效 `{name}`"));
    }
    if let Some(root) = project_root {
        if let Some(spec) = load_project_agents(root)
            .into_iter()
            .find(|a| a.name == name)
        {
            return Ok(spec);
        }
    }
    if let Some(spec) = load_user_agents(user_dir)
        .into_iter()
        .find(|a| a.name == name)
    {
        return Ok(spec);
    }
    if let Some(spec) = builtin_agents().into_iter().find(|a| a.name == name) {
        return Ok(spec);
    }
    Err(format!("智能体不存在：{name}"))
}

/// Resolve a chain of persona specs in order. Each name is checked through
/// the same priority as [`resolve_spec`]; an unknown name in the chain
/// fails the entire send (one bad link must not stall the run).
pub fn resolve_chain(
    user_dir: &Path,
    project_root: Option<&Path>,
    names: &[String],
) -> Result<Vec<AgentSpec>, String> {
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        out.push(resolve_spec(user_dir, project_root, name)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT_MD: &str = "# Agent: reviewer\n\
        ## description\nStrict code reviewer.\n\
        ## instructions\nChallenge every assumption.\nPoint at real risks.\n";

    /// A non-builtin persona fixture — `parse_agent` rejects files whose
    /// title shadows a builtin name.
    const CUSTOM_AGENT_MD: &str = "# Agent: custom\n\
        ## description\nA user-defined persona.\n\
        ## instructions\nSpeak politely and stay terse.\n";

    /// Fresh temp dir per test (name-scoped so parallel tests never collide).
    fn temp_store(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kodo_user_agents_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn parse_agent_round_trips_three_fields() {
        let spec = parse_agent(AGENT_MD).expect("parse");
        assert_eq!(spec.name, "reviewer");
        assert_eq!(spec.description, "Strict code reviewer.");
        assert_eq!(spec.instructions, "Challenge every assumption.\nPoint at real risks.");
    }

    #[test]
    fn parse_agent_requires_title_and_both_sections() {
        assert!(parse_agent("no title").is_err());
        assert!(parse_agent("# Agent: x\n## instructions\nBe nice.\n")
            .unwrap_err()
            .contains("description"));
        assert!(parse_agent("# Agent: x\n## description\nNice bot.\n")
            .unwrap_err()
            .contains("instructions"));
        // Empty bodies count as missing.
        assert!(parse_agent("# Agent: x\n## description\n\n## instructions\n\n").is_err());
    }

    #[test]
    fn save_load_delete_round_trip() {
        let dir = temp_store("roundtrip");
        let spec = save_user_agent(&dir, "custom", CUSTOM_AGENT_MD).expect("save");
        assert_eq!(spec.name, "custom");
        assert!(dir.join("custom.md").exists());

        let loaded = load_user_agents(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "custom");

        delete_user_agent(&dir, "custom").expect("delete");
        assert!(!dir.join("custom.md").exists());
        assert!(load_user_agents(&dir).is_empty());
        assert!(delete_user_agent(&dir, "custom")
            .unwrap_err()
            .contains("不存在"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_rejects_invalid_names_and_title_drift() {
        let dir = temp_store("invalid");
        assert!(save_user_agent(&dir, "bad name!", CUSTOM_AGENT_MD)
            .unwrap_err()
            .contains("无效"));
        // `CUSTOM_AGENT_MD` title is `custom`; saving as "custom" with a
        // mismatched title must reject for drift, not for a builtin clash.
        let wrong = CUSTOM_AGENT_MD.replace("custom", "something-else");
        assert!(save_user_agent(&dir, "custom", &wrong)
            .unwrap_err()
            .contains("不一致"));
        assert!(save_user_agent(&dir, "custom", "not an agent").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_skips_unusable_files() {
        let dir = temp_store("skip");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("custom.md"), CUSTOM_AGENT_MD).expect("good");
        std::fs::write(dir.join("broken.md"), "no title").expect("broken");
        // Stem ≠ title: selection id would miss on lookup — skipped.
        std::fs::write(dir.join("other.md"), CUSTOM_AGENT_MD).expect("mismatch");
        std::fs::write(dir.join("bad name.md"), CUSTOM_AGENT_MD).expect("bad stem");
        std::fs::write(dir.join("readme.txt"), CUSTOM_AGENT_MD).expect("txt");

        let loaded = load_user_agents(&dir);
        assert_eq!(loaded.len(), 1, "only custom.md loads: {loaded:?}");
        assert!(load_user_agents(&dir.join("missing")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_instructions_finds_and_reports_misses() {
        let dir = temp_store("resolve");
        let markdown = "# Agent: user-only\n\
            ## description\nUser persona.\n\
            ## instructions\nSpeak politely and stay terse.\n";
        save_user_agent(&dir, "user-only", markdown).expect("save");
        assert_eq!(
            resolve_instructions(&dir, "user-only").expect("hit"),
            "Speak politely and stay terse."
        );
        assert!(resolve_instructions(&dir, "missing")
            .unwrap_err()
            .contains("不存在"));
        assert!(resolve_instructions(&dir, "bad name").is_err());
        // A builtin name still resolves (its instructions live in the
        // registry, not the user dir).
        assert!(resolve_instructions(&dir.join("missing"), "reviewer").is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Built-in personas (12 static markdown files under ../../agents/) ---

    #[test]
    fn builtin_agents_parses_all_twelve() {
        let specs = builtin_agents();
        assert_eq!(specs.len(), 12, "expected 12 builtins, got {specs:?}");
        for spec in &specs {
            assert!(!spec.name.is_empty());
            assert!(!spec.description.is_empty());
            assert!(!spec.instructions.is_empty());
        }
    }

    #[test]
    fn builtin_markdown_round_trips_for_each_builtin() {
        for (name, _) in BUILTIN_AGENTS.iter() {
            let md = builtin_markdown(name).expect("builtin markdown");
            assert!(
                md.contains(&format!("# Agent: {name}")),
                "markdown for `{name}` missing title: {md:?}"
            );
            assert!(is_builtin_name(name));
        }
        assert_eq!(BUILTIN_AGENTS.len(), 12);
    }

    #[test]
    fn resolve_instructions_prefers_builtin_over_user_shadow() {
        let dir = temp_store("builtin_shadow");
        // A user file with a builtin name must not win — load_user_agents
        // skips it; resolve_instructions reads the builtin first.
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("reviewer.md"), AGENT_MD).expect("user reviewer");
        let builtin_instructions = builtin_agents()
            .into_iter()
            .find(|a| a.name == "reviewer")
            .map(|a| a.instructions)
            .expect("builtin reviewer");
        let resolved = resolve_instructions(&dir, "reviewer").expect("hit");
        assert_eq!(resolved, builtin_instructions);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_user_agent_rejects_builtin_name() {
        let dir = temp_store("save_builtin");
        let err = save_user_agent(&dir, "reviewer", AGENT_MD)
            .expect_err("builtin name must reject");
        assert!(
            err.contains("内置"),
            "error must mention 内置, got: {err}"
        );
        assert!(!dir.join("reviewer.md").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_user_agent_rejects_builtin_name() {
        let dir = temp_store("delete_builtin");
        // Even if the user manages to drop a file named after a builtin,
        // delete rejects — the file shouldn't be there in the first place.
        let err = delete_user_agent(&dir, "explainer")
            .expect_err("builtin name must reject");
        assert!(
            err.contains("内置"),
            "error must mention 内置, got: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_user_agents_skips_file_that_shadows_builtin() {
        let dir = temp_store("shadow");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("reviewer.md"), AGENT_MD).expect("reviewer");
        std::fs::write(dir.join("custom.md"), CUSTOM_AGENT_MD).expect("custom");
        let loaded = load_user_agents(&dir);
        assert_eq!(loaded.len(), 1, "reviewer must be skipped: {loaded:?}");
        assert_eq!(loaded[0].name, "custom");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Optional sections: defaults / variables / examples (§2.1-2.5) ---

    /// Full-shape fixture — every optional section populated. Raw string to
/// preserve the two-space indent in front of `output:` (Rust's `\` line
/// continuation would strip leading whitespace).
    const ADVANCED_AGENT_MD: &str = r#"# Agent: advanced
## description
A persona that exercises defaults/variables/examples.
## instructions
Review `{{project_path}}` on branch `{{branch}}` carefully.
## defaults
provider: anthropic
model: claude-sonnet-4-5
permission: auto
skills: code-review, security-audit
## variables
- project_path
- branch
## examples
- input: 帮我看下这个 PR
  output: 风险 1：... 建议：...
"#;

    #[test]
    fn parse_agent_parses_defaults_section() {
        let spec = parse_agent(ADVANCED_AGENT_MD).expect("parse");
        assert_eq!(spec.defaults.provider.as_deref(), Some("anthropic"));
        assert_eq!(spec.defaults.model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(spec.defaults.permission.as_deref(), Some("auto"));
        assert_eq!(
            spec.defaults.skills,
            vec!["code-review".to_owned(), "security-audit".to_owned()]
        );
    }

    #[test]
    fn parse_agent_parses_variables_section() {
        let spec = parse_agent(ADVANCED_AGENT_MD).expect("parse");
        assert_eq!(
            spec.variables,
            vec!["project_path".to_owned(), "branch".to_owned()]
        );
    }

    #[test]
    fn parse_agent_parses_examples_section() {
        let spec = parse_agent(ADVANCED_AGENT_MD).expect("parse");
        assert_eq!(spec.examples.len(), 1);
        assert!(spec.examples[0].input.contains("帮我看下这个 PR"));
        assert!(spec.examples[0].output.contains("风险 1"));
    }

    #[test]
    fn parse_agent_optional_sections_default_to_empty() {
        // CUSTOM_AGENT_MD has no `## defaults` / `## variables` / `## examples`.
        let spec = parse_agent(CUSTOM_AGENT_MD).expect("parse");
        assert_eq!(spec.defaults.provider, None);
        assert_eq!(spec.defaults.model, None);
        assert_eq!(spec.defaults.permission, None);
        assert!(spec.defaults.skills.is_empty());
        assert!(spec.variables.is_empty());
        assert!(spec.examples.is_empty());
    }

    #[test]
    fn parse_agent_rejects_unknown_permission_value() {
        let bad = ADVANCED_AGENT_MD.replace("permission: auto", "permission: chaos");
        let spec = parse_agent(&bad).expect("parse still succeeds");
        assert_eq!(spec.defaults.permission, None);
    }

    #[test]
    fn parse_agent_rejects_invalid_variable_name() {
        let bad = ADVANCED_AGENT_MD.replace("- branch", "- 1bad-name");
        let spec = parse_agent(&bad).expect("parse still succeeds");
        assert_eq!(
            spec.variables,
            vec!["project_path".to_owned()],
            "1bad-name must be skipped: {spec:?}"
        );
    }

    // --- Resolution priority: project > user > builtin (§2.7) ---

    /// Build a temp project root with `.kodo/agents/<name>.md`.
    fn temp_project(tag: &str, agents: &[(&str, &str)]) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("kodo_project_agents_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join(".kodo").join("agents");
        std::fs::create_dir_all(&dir).expect("project dir");
        for (name, md) in agents {
            std::fs::write(dir.join(format!("{name}.md")), md).expect("write");
        }
        root
    }

    #[test]
    fn resolve_spec_prefers_project_over_user_over_builtin() {
        let user_dir = temp_store("resolve_spec");
        let project_root = temp_project("resolve_spec", &[]);
        std::fs::create_dir_all(&user_dir).expect("user dir");
        // builtin name; user has a copy with one instruction, project has another
        std::fs::write(
            user_dir.join("reviewer.md"),
            "# Agent: reviewer\n## description\nUser reviewer.\n## instructions\nUSER-INSTANCE.\n",
        )
        .expect("user reviewer");
        std::fs::write(
            project_root.join(".kodo/agents/reviewer.md"),
            "# Agent: reviewer\n## description\nProject reviewer.\n## instructions\nPROJECT-INSTANCE.\n",
        )
        .expect("project reviewer");
        // Builtin must lose because project shadows it.
        let spec = resolve_spec(&user_dir, Some(&project_root), "reviewer")
            .expect("resolve");
        assert_eq!(spec.instructions, "PROJECT-INSTANCE.");
        // Without a project root the user copy is also blocked by the
        // builtin name protection (load_user_agents skips it).
        let spec = resolve_spec(&user_dir, None, "reviewer").expect("resolve");
        assert_eq!(spec.instructions, builtin_agents()
            .into_iter()
            .find(|a| a.name == "reviewer")
            .unwrap()
            .instructions);
        let _ = std::fs::remove_dir_all(&user_dir);
        let _ = std::fs::remove_dir_all(&project_root);
    }

    #[test]
    fn resolve_chain_returns_specs_in_order() {
        let user_dir = temp_store("resolve_chain");
        std::fs::create_dir_all(&user_dir).expect("user dir");
        std::fs::write(
            user_dir.join("user-only.md"),
            "# Agent: user-only\n## description\nU.\n## instructions\nUSER.\n",
        )
        .expect("user-only");
        let names = vec!["reviewer".to_owned(), "user-only".to_owned()];
        let chain = resolve_chain(&user_dir, None, &names).expect("resolve");
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].name, "reviewer");
        assert_eq!(chain[1].name, "user-only");
        assert_eq!(chain[1].instructions, "USER.");
        // An unknown name in the chain fails the whole resolve.
        let bad = vec!["reviewer".to_owned(), "ghost".to_owned()];
        assert!(resolve_chain(&user_dir, None, &bad).is_err());
        let _ = std::fs::remove_dir_all(&user_dir);
    }
}
