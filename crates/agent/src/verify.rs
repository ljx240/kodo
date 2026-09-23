//! Verification commands and failure compression for the repair loop.
//!
//! Verification is scoped to **user acceptance criteria** via
//! [`VerificationPlan`]: commands are ordered exact → package → typecheck →
//! broad, each command carries the `criterion_ids` it may prove, and a pass
//! is never auto-applied to every criterion.

use std::fs;
use std::path::Path;

/// Category of a verification command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyKind {
    Build,
    Typecheck,
    Test,
    Lint,
}

impl VerifyKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Typecheck => "typecheck",
            Self::Test => "test",
            Self::Lint => "lint",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyCommand {
    pub kind: VerifyKind,
    pub command: String,
    pub timeout_ms: u64,
}

impl VerifyCommand {
    pub fn test(cmd: impl Into<String>) -> Self {
        Self {
            kind: VerifyKind::Test,
            command: cmd.into(),
            timeout_ms: 120_000,
        }
    }

    pub fn build(cmd: impl Into<String>) -> Self {
        Self {
            kind: VerifyKind::Build,
            command: cmd.into(),
            timeout_ms: 120_000,
        }
    }
}

/// Why a verification command failed — product vs environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// Real product/test/assertion failure the agent should repair.
    Product,
    /// Missing compiler, network outage, command-not-found — not a code regression.
    Infrastructure,
    Timeout,
    Cancelled,
}

impl FailureClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Infrastructure => "infrastructure",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
        }
    }

    /// Infra failures must not drive a product-regression repair loop.
    pub fn is_infrastructure(self) -> bool {
        matches!(self, Self::Infrastructure)
    }
}

/// User-facing failure taxonomy. Classified once in Rust from structured
/// signals (exit code, denial flag, well-known error shapes) so the UI never
/// has to guess from display strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandFailureKind {
    /// The binary itself is missing (exit 127 / "command not found").
    CommandNotFound,
    /// The binary exists but cannot be executed (exit 126 / EACCES).
    PermissionDenied,
    /// The command ran and returned a non-zero exit code.
    NonZeroExit,
    /// The command was killed by its deadline.
    Timeout,
    /// The user refused to run this command.
    Denied,
    /// Provider/model-level failure surfaced on a command-shaped step.
    ProviderError,
    /// The working tree diverged (undo/diff conflict).
    WorkspaceConflict,
}

impl CommandFailureKind {
    /// Stable wire code. The UI maps this to localized copy.
    pub fn code(self) -> &'static str {
        match self {
            Self::CommandNotFound => "command_not_found",
            Self::PermissionDenied => "permission_denied",
            Self::NonZeroExit => "non_zero_exit",
            Self::Timeout => "timeout",
            Self::Denied => "denied",
            Self::ProviderError => "provider_error",
            Self::WorkspaceConflict => "workspace_conflict",
        }
    }

    /// Classify one failed command execution. `None` when nothing failed.
    pub fn classify(exit_code: Option<i32>, output: &str, denied: bool) -> Option<Self> {
        if denied {
            return Some(Self::Denied);
        }
        let lower = output.to_ascii_lowercase();
        if lower.contains("timed out") || lower.contains("timeout after") {
            return Some(Self::Timeout);
        }
        if lower.contains("command not found")
            || lower.contains(": not found")
            || lower.contains("is not recognized as an internal or external command")
            || exit_code == Some(127)
        {
            return Some(Self::CommandNotFound);
        }
        if exit_code == Some(126)
            || lower.contains("permission denied")
            || lower.contains("not executable")
            || lower.contains("operation not permitted")
        {
            return Some(Self::PermissionDenied);
        }
        if lower.contains("would be overwritten") || lower.contains("merge conflict") {
            return Some(Self::WorkspaceConflict);
        }
        if exit_code.map(|code| code != 0).unwrap_or(false) {
            return Some(Self::NonZeroExit);
        }
        None
    }

    /// The missing binary named by a `command not found` style failure, e.g.
    /// `/bin/sh: cargo: command not found` → `cargo`.
    pub fn missing_tool(output: &str) -> Option<String> {
        for marker in [": command not found", ": not found"] {
            if let Some(index) = output.find(marker) {
                let before = &output[..index];
                let tool = before
                    .rsplit([' ', '\n', '\t', '/', '\\', ':'])
                    .next()
                    .unwrap_or("")
                    .trim();
                if !tool.is_empty() {
                    return Some(tool.to_owned());
                }
            }
        }
        None
    }
}

/// How early a planned command proves acceptance (run order priority).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VerifyTier {
    /// Named regression / exact test from the criterion.
    ExactRegression,
    /// Tests for the changed package/crate.
    Package,
    /// Typecheck / check only.
    Typecheck,
    /// Broader workspace suite last.
    Broad,
}

impl VerifyTier {
    pub fn label(self) -> &'static str {
        match self {
            Self::ExactRegression => "exact_regression",
            Self::Package => "package",
            Self::Typecheck => "typecheck",
            Self::Broad => "broad",
        }
    }
}

/// One planned command bound to the acceptance criteria it may prove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedVerifyCommand {
    pub command: VerifyCommand,
    /// Criteria this command is allowed to mark as verification-passed.
    pub criterion_ids: Vec<String>,
    pub tier: VerifyTier,
}

/// Verification scoped to user acceptance criteria + changed scope.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VerificationPlan {
    /// Every criterion this plan is responsible for proving.
    pub criterion_ids: Vec<String>,
    pub changed_files: Vec<String>,
    /// Primary package/crate when known (e.g. `kodo-agent`).
    pub package: Option<String>,
    pub task_type: String,
    /// Ordered: exact → package → typecheck → broad.
    pub commands: Vec<PlannedVerifyCommand>,
}

impl VerificationPlan {
    /// Criteria that some planned command may prove.
    pub fn targeted_criterion_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .commands
            .iter()
            .flat_map(|c| c.criterion_ids.iter().cloned())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Failed commands from evidence, highest tier first (for repair re-run).
    pub fn failed_commands(&self, failed: &[String]) -> Vec<VerifyCommand> {
        let mut out: Vec<VerifyCommand> = Vec::new();
        for planned in &self.commands {
            if failed.iter().any(|f| f == &planned.command.command)
                && !out.iter().any(|c| c.command == planned.command.command)
            {
                out.push(planned.command.clone());
            }
        }
        out.sort_by_key(|c| {
            self.commands
                .iter()
                .find(|p| p.command.command == c.command)
                .map(|p| p.tier)
                .unwrap_or(VerifyTier::Broad)
        });
        out
    }
}

/// One observed verification run, bound to the criteria it targeted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationEvidence {
    pub command: String,
    pub cwd: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub criterion_ids: Vec<String>,
    pub output_summary: String,
    pub truncated: bool,
    pub ok: bool,
    pub failure_class: Option<FailureClass>,
}

impl VerificationEvidence {
    pub fn from_outcome(
        outcome: &VerifyOutcome,
        cwd: &str,
        criterion_ids: Vec<String>,
        max_summary: usize,
    ) -> Self {
        let mut output_summary = outcome.output_tail.clone();
        let mut truncated = output_summary.len() > max_summary;
        if truncated {
            let mut end = max_summary;
            while end > 0 && !output_summary.is_char_boundary(end) {
                end -= 1;
            }
            output_summary.truncate(end);
            output_summary.push('…');
        }
        let failure_class = if outcome.ok {
            None
        } else if outcome.cancelled {
            Some(FailureClass::Cancelled)
        } else if outcome.timed_out {
            Some(FailureClass::Timeout)
        } else {
            Some(classify_verify_failure(
                &outcome.command.command,
                &outcome.output_tail,
            ))
        };
        // Keep truncated flag true if source was already a tail.
        if outcome.output_tail.len() >= 2000 {
            truncated = true;
        }
        Self {
            command: outcome.command.command.clone(),
            cwd: cwd.to_owned(),
            exit_code: outcome.exit_code,
            duration_ms: outcome.duration_ms,
            criterion_ids,
            output_summary,
            truncated,
            ok: outcome.ok,
            failure_class,
        }
    }
}

/// Classify a failed verify command: product regression vs infrastructure.
pub fn classify_verify_failure(command: &str, output: &str) -> FailureClass {
    let lower = format!(
        "{} \n{}",
        command.to_ascii_lowercase(),
        output.to_ascii_lowercase()
    );
    const INFRA: [&str; 24] = [
        "command not found",
        "no such file or directory",
        "cannot find crate",
        "error: no test target",
        "could not resolve host",
        "connection refused",
        "econnrefused",
        "network is unreachable",
        "temporary failure in name resolution",
        "dns error",
        "rustc not found",
        "cargo not found",
        "node: command not found",
        "npm err! missing script",
        "enoent",
        "no such package",
        "unable to locate package",
        "permission denied when spawning",
        "not executable",
        "http 503",
        "http 502",
        "http 429",
        "rate limit",
        "proxy error",
    ];
    if INFRA.iter().any(|m| lower.contains(m)) {
        return FailureClass::Infrastructure;
    }
    FailureClass::Product
}

/// What to do after a verification run when something failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairDecision {
    /// Product failure worth entering Repair.
    pub can_repair: bool,
    pub failure_class: FailureClass,
    pub failed_evidence: Vec<VerificationEvidence>,
    /// Commands to re-run first after repair (highest priority still failing).
    pub targeted_rerun: Vec<VerifyCommand>,
    /// Compressed hint for the model.
    pub hint: String,
    /// Budget already exhausted — do not enter Repair.
    pub budget_exhausted: bool,
}

impl RepairDecision {
    /// Build a repair decision from plan outcomes.
    ///
    /// * Infrastructure failures → `can_repair=false` (Blocked path).
    /// * Product failures + budget left → repair with targeted re-run list.
    /// * Product failures + budget out → `budget_exhausted=true`.
    pub fn from_evidence(
        evidence: &[VerificationEvidence],
        plan: &VerificationPlan,
        budget_ok: bool,
    ) -> Self {
        let failed: Vec<&VerificationEvidence> = evidence.iter().filter(|e| !e.ok).collect();
        if failed.is_empty() {
            return Self {
                can_repair: false,
                failure_class: FailureClass::Product,
                failed_evidence: Vec::new(),
                targeted_rerun: Vec::new(),
                hint: String::new(),
                budget_exhausted: false,
            };
        }
        let infra = failed
            .iter()
            .all(|e| e.failure_class == Some(FailureClass::Infrastructure))
            || failed
                .iter()
                .any(|e| e.failure_class == Some(FailureClass::Infrastructure))
                && failed.iter().all(|e| {
                    matches!(
                        e.failure_class,
                        Some(FailureClass::Infrastructure) | Some(FailureClass::Timeout)
                    )
                });
        let class = if failed
            .iter()
            .any(|e| e.failure_class == Some(FailureClass::Product))
        {
            FailureClass::Product
        } else if failed
            .iter()
            .any(|e| e.failure_class == Some(FailureClass::Infrastructure))
        {
            FailureClass::Infrastructure
        } else if failed
            .iter()
            .any(|e| e.failure_class == Some(FailureClass::Timeout))
        {
            FailureClass::Timeout
        } else {
            FailureClass::Cancelled
        };
        let _ = infra;

        let failed_cmds: Vec<String> = failed.iter().map(|e| e.command.clone()).collect();
        let targeted = plan.failed_commands(&failed_cmds);
        let hint = failed
            .iter()
            .map(|e| {
                format!(
                    "FAIL [{}] exit={:?} criteria={:?}\n{}",
                    e.command, e.exit_code, e.criterion_ids, e.output_summary
                )
            })
            .collect::<Vec<_>>()
            .join("\n---\n");

        match class {
            FailureClass::Infrastructure => Self {
                can_repair: false,
                failure_class: class,
                failed_evidence: failed.into_iter().cloned().collect(),
                targeted_rerun: targeted,
                hint: format!(
                    "Infrastructure verification failure (not a product regression):\n{hint}"
                ),
                budget_exhausted: false,
            },
            _ if !budget_ok => Self {
                can_repair: false,
                failure_class: class,
                failed_evidence: failed.into_iter().cloned().collect(),
                targeted_rerun: targeted,
                hint,
                budget_exhausted: true,
            },
            _ => Self {
                can_repair: true,
                failure_class: class,
                failed_evidence: failed.into_iter().cloned().collect(),
                targeted_rerun: targeted,
                hint,
                budget_exhausted: false,
            },
        }
    }
}

/// Compressed failure for the model / repair prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureReport {
    pub command: String,
    pub exit_code: Option<i32>,
    pub primary_error: String,
    pub relevant_files: Vec<String>,
}

impl FailureReport {
    pub fn to_prompt_block(&self) -> String {
        format!(
            "command: {}\nexit_code: {}\nprimary_error: {}\nrelevant_files:\n{}\n",
            self.command,
            self.exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "none".into()),
            self.primary_error,
            if self.relevant_files.is_empty() {
                "  (none)".to_owned()
            } else {
                self.relevant_files
                    .iter()
                    .map(|f| format!("  {f}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyOutcome {
    pub command: VerifyCommand,
    pub exit_code: Option<i32>,
    pub ok: bool,
    pub timed_out: bool,
    pub cancelled: bool,
    /// Working directory the command ran in.
    pub cwd: String,
    /// Wall-clock duration of the run.
    pub duration_ms: u64,
    /// Truncated raw output (for notes only).
    pub output_tail: String,
    pub failure: Option<FailureReport>,
}

/// Final verification status for the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalStatus {
    Verified,
    PartiallyVerified,
    VerificationFailed,
    NotVerified,
    /// User stop / alive() went false — never reported as Completed.
    Cancelled,
    /// Budget or denial exhausted without a verified result.
    Blocked,
    /// Provider-level failure ended the turn before verification.
    ProviderError,
}

impl FinalStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Verified => "Verified",
            Self::PartiallyVerified => "Partially verified",
            Self::VerificationFailed => "Verification failed",
            Self::NotVerified => "Not verified",
            Self::Cancelled => "Cancelled",
            Self::Blocked => "Blocked",
            Self::ProviderError => "Provider error",
        }
    }
}

/// Infers and runs build/typecheck/test/lint with timeouts.
pub struct VerificationRunner {
    pub default_timeout_ms: u64,
}

impl Default for VerificationRunner {
    fn default() -> Self {
        Self {
            default_timeout_ms: 90_000,
        }
    }
}

impl VerificationRunner {
    pub fn new(default_timeout_ms: u64) -> Self {
        Self { default_timeout_ms }
    }

    /// Infer candidate commands from project layout and package scripts.
    pub fn infer(&self, project: &Path) -> Vec<VerifyCommand> {
        self.infer_targeted(project, &[])
    }

    /// Infer verification commands, narrowing to changed files' packages when
    /// possible (targeted first, then package-level, then workspace).
    pub fn infer_targeted(&self, project: &Path, changed_files: &[String]) -> Vec<VerifyCommand> {
        let mut cmds: Vec<VerifyCommand> = Vec::new();
        let push = |cmds: &mut Vec<VerifyCommand>, kind: VerifyKind, command: String| {
            if !cmds.iter().any(|c| c.command == command) {
                cmds.push(VerifyCommand {
                    kind,
                    command,
                    timeout_ms: self.default_timeout_ms,
                });
            }
        };

        // Map changed paths → cargo package / npm package for targeted runs.
        let mut crates_changed: Vec<String> = Vec::new();
        let mut web_changed = false;
        for path in changed_files {
            if let Some(rest) = path.strip_prefix("crates/") {
                if let Some(pkg) = rest.split('/').next() {
                    let name =
                        std::fs::read_to_string(project.join(format!("crates/{pkg}/Cargo.toml")))
                            .ok()
                            .and_then(|text| {
                                text.lines()
                                    .find_map(|l| l.trim().strip_prefix("name = "))
                                    .map(|n| n.trim_matches('"').to_owned())
                            })
                            .unwrap_or_else(|| pkg.to_owned());
                    if !crates_changed.contains(&name) {
                        crates_changed.push(name);
                    }
                }
            }
            if path.starts_with("apps/desktop/") {
                web_changed = true;
            }
        }

        if project.join("Cargo.toml").is_file() {
            if !crates_changed.is_empty() {
                for pkg in crates_changed.iter().take(3) {
                    push(
                        &mut cmds,
                        VerifyKind::Test,
                        format!("cargo test -p {pkg} --quiet"),
                    );
                }
                push(&mut cmds, VerifyKind::Build, "cargo check --quiet".into());
            } else {
                push(&mut cmds, VerifyKind::Build, "cargo check --quiet".into());
                push(
                    &mut cmds,
                    VerifyKind::Test,
                    "cargo test --workspace --quiet".into(),
                );
            }
        }

        let web_manifest = if web_changed {
            ["apps/desktop/package.json", "package.json"]
                .iter()
                .map(|p| project.join(p))
                .find(|p| p.is_file())
        } else if project.join("package.json").is_file() {
            Some(project.join("package.json"))
        } else {
            None
        };
        if let Some(manifest) = web_manifest {
            let text = fs::read_to_string(&manifest).unwrap_or_default();
            let npm_prefix = if manifest.ends_with("apps/desktop/package.json") {
                "npm --prefix apps/desktop run "
            } else {
                "npm run "
            };
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(scripts) = value.get("scripts").and_then(|s| s.as_object()) {
                    // Prefer explicit script names.
                    let mapping = [
                        ("typecheck", VerifyKind::Typecheck),
                        ("type-check", VerifyKind::Typecheck),
                        ("tsc", VerifyKind::Typecheck),
                        ("build", VerifyKind::Build),
                        ("test", VerifyKind::Test),
                        ("lint", VerifyKind::Lint),
                    ];
                    for (name, kind) in mapping {
                        if scripts.contains_key(name) {
                            push(&mut cmds, kind, format!("{npm_prefix}{name}"));
                        }
                    }
                }
            }
            // Fallbacks when scripts missing
            if !cmds.iter().any(|c| c.kind == VerifyKind::Test) {
                push(&mut cmds, VerifyKind::Test, "npm test --silent".into());
            }
        }

        if project.join("pyproject.toml").is_file() || project.join("pytest.ini").is_file() {
            push(&mut cmds, VerifyKind::Test, "python -m pytest -q".into());
        }

        // Cap: run most relevant first (test > typecheck > build > lint).
        cmds.sort_by_key(|c| match c.kind {
            VerifyKind::Test => 0,
            VerifyKind::Typecheck => 1,
            VerifyKind::Build => 2,
            VerifyKind::Lint => 3,
        });
        cmds.truncate(4);
        cmds
    }

    /// Build a criterion-scoped verification plan.
    ///
    /// Order: exact regression test → related package tests → typecheck →
    /// broader suite. Each command lists only the `criterion_ids` it may prove
    /// — a generic pass never blankets every criterion.
    ///
    /// `criteria` is `(id, description)` for acceptance criteria that need
    /// verification evidence.
    pub fn build_plan(
        project: &Path,
        criteria: &[(String, String)],
        changed_files: &[String],
        task_type: &str,
    ) -> VerificationPlan {
        let mut all_ids: Vec<String> = criteria.iter().map(|(id, _)| id.clone()).collect();
        all_ids.sort();
        all_ids.dedup();

        // Criteria with a specific test/target extracted vs generic verify.
        let mut exact_targets: Vec<(String, String)> = Vec::new(); // (criterion_id, test_filter)
        let mut generic_ids: Vec<String> = Vec::new();
        for (id, desc) in criteria {
            if let Some(filter) = extract_test_filter(desc) {
                exact_targets.push((id.clone(), filter));
            } else {
                generic_ids.push(id.clone());
            }
        }

        let mut package = None;
        let mut crates_changed: Vec<String> = Vec::new();
        let mut web_changed = false;
        for path in changed_files {
            if let Some(rest) = path.strip_prefix("crates/") {
                if let Some(pkg) = rest.split('/').next() {
                    let name =
                        std::fs::read_to_string(project.join(format!("crates/{pkg}/Cargo.toml")))
                            .ok()
                            .and_then(|text| {
                                text.lines()
                                    .find_map(|l| l.trim().strip_prefix("name = "))
                                    .map(|n| n.trim_matches('"').to_owned())
                            })
                            .unwrap_or_else(|| pkg.to_owned());
                    if !crates_changed.contains(&name) {
                        crates_changed.push(name.clone());
                    }
                    if package.is_none() {
                        package = Some(name);
                    }
                }
            }
            if path.starts_with("apps/desktop/") {
                web_changed = true;
            }
        }

        let mut commands: Vec<PlannedVerifyCommand> = Vec::new();
        struct PushCtx<'a> {
            commands: &'a mut Vec<PlannedVerifyCommand>,
            timeout_ms: u64,
        }
        impl PushCtx<'_> {
            fn push(
                &mut self,
                kind: VerifyKind,
                command: String,
                tier: VerifyTier,
                ids: Vec<String>,
            ) {
                if command.is_empty() || ids.is_empty() {
                    return;
                }
                if let Some(existing) = self
                    .commands
                    .iter_mut()
                    .find(|c| c.command.command == command)
                {
                    for id in ids {
                        if !existing.criterion_ids.contains(&id) {
                            existing.criterion_ids.push(id);
                        }
                    }
                    return;
                }
                self.commands.push(PlannedVerifyCommand {
                    command: VerifyCommand {
                        kind,
                        command,
                        timeout_ms: self.timeout_ms,
                    },
                    criterion_ids: ids,
                    tier,
                });
            }
        }

        // Pushes that need &mut live in this scope so later code can use `commands`.
        {
            let mut ctx = PushCtx {
                commands: &mut commands,
                timeout_ms: 120_000,
            };

            // 1) Exact regression tests (one per criterion that named a test).
            for (cid, filter) in &exact_targets {
                ctx.push(
                    VerifyKind::Test,
                    format!("cargo test {filter} --quiet"),
                    VerifyTier::ExactRegression,
                    vec![cid.clone()],
                );
            }

            // 2) Related package tests — generic verification criteria only.
            let generic_for_package: Vec<String> = if exact_targets.is_empty() {
                all_ids.clone()
            } else {
                generic_ids.clone()
            };
            if !generic_for_package.is_empty() {
                if !crates_changed.is_empty() {
                    for pkg in crates_changed.iter().take(3) {
                        ctx.push(
                            VerifyKind::Test,
                            format!("cargo test -p {pkg} --quiet"),
                            VerifyTier::Package,
                            generic_for_package.clone(),
                        );
                    }
                } else if project.join("Cargo.toml").is_file() {
                    ctx.push(
                        VerifyKind::Test,
                        "cargo test --quiet".to_owned(),
                        VerifyTier::Package,
                        generic_for_package.clone(),
                    );
                }
            }

            // 3) Typecheck / check
            if !generic_for_package.is_empty() || !exact_targets.is_empty() {
                let typecheck_ids: Vec<String> = if exact_targets.is_empty() {
                    all_ids.clone()
                } else {
                    generic_ids.clone()
                };
                if !typecheck_ids.is_empty() {
                    if project.join("Cargo.toml").is_file() {
                        ctx.push(
                            VerifyKind::Build,
                            "cargo check --quiet".to_owned(),
                            VerifyTier::Typecheck,
                            typecheck_ids.clone(),
                        );
                    }
                    if web_changed {
                        ctx.push(
                            VerifyKind::Typecheck,
                            "npm --prefix apps/desktop run typecheck".to_owned(),
                            VerifyTier::Typecheck,
                            typecheck_ids.clone(),
                        );
                    }
                }
            }
        }

        // 4) Broader suite last — only when no package-level test was planned.
        let has_package_cmd = commands.iter().any(|c| c.tier == VerifyTier::Package);
        if !generic_ids.is_empty() && project.join("Cargo.toml").is_file() && !has_package_cmd {
            commands.push(PlannedVerifyCommand {
                command: VerifyCommand {
                    kind: VerifyKind::Test,
                    command: "cargo test --workspace --quiet".to_owned(),
                    timeout_ms: 120_000,
                },
                criterion_ids: generic_ids.clone(),
                tier: VerifyTier::Broad,
            });
        }

        commands.sort_by_key(|c| c.tier);

        // task_type currently informs callers / logging; keep on the plan.
        VerificationPlan {
            criterion_ids: all_ids,
            changed_files: changed_files.to_vec(),
            package,
            task_type: task_type.to_owned(),
            commands,
        }
    }

    /// Run a verification plan in tier order. Stops after the first product
    /// failure at ExactRegression (targeted repair) unless `stop_on_fail=false`.
    /// A missing tool blocks every later command that needs the same binary
    /// (recorded without spawning), and never drives a product-repair loop.
    pub fn run_plan(
        &self,
        project: &Path,
        plan: &VerificationPlan,
        alive: &dyn Fn() -> bool,
        stop_on_fail: bool,
    ) -> Vec<VerificationEvidence> {
        let mut out = Vec::new();
        let cwd = project.display().to_string();
        let mut missing_tools: Vec<String> = Vec::new();
        for planned in &plan.commands {
            if !alive() {
                break;
            }
            let binary = first_token(&planned.command.command);
            if missing_tools.iter().any(|tool| *tool == binary) {
                // Same missing tool: running it again cannot succeed. Record
                // the block without spawning so the trace shows every doomed
                // step and the summary can say "N steps blocked".
                let outcome = blocked_outcome(
                    &planned.command,
                    &cwd,
                    &format!("{binary}: command not found (tool unavailable)"),
                );
                out.push(VerificationEvidence::from_outcome(
                    &outcome,
                    &cwd,
                    planned.criterion_ids.clone(),
                    500,
                ));
                continue;
            }
            let outcome = self.run_one(project, &planned.command, alive);
            let ev = VerificationEvidence::from_outcome(
                &outcome,
                &cwd,
                planned.criterion_ids.clone(),
                500,
            );
            let failed = !ev.ok;
            if failed {
                if let Some(tool) = CommandFailureKind::missing_tool(&outcome.output_tail) {
                    missing_tools.push(tool);
                    out.push(ev);
                    // Other tools' commands may still run; same-tool ones are
                    // skipped above. No point stopping the whole plan.
                    continue;
                }
            }
            out.push(ev);
            if failed && stop_on_fail {
                // Stop so repair can re-run this targeted command first.
                break;
            }
        }
        out
    }

    /// Run one command with timeout + process-tree kill via **the same**
    /// [`crate::process::ProcessRunner`] as agent shell tools. `alive` can
    /// cancel earlier; cancelled/timeout are not ordinary exit failures.
    pub fn run_one(
        &self,
        project: &Path,
        cmd: &VerifyCommand,
        alive: &dyn Fn() -> bool,
    ) -> VerifyOutcome {
        let timeout_secs = (cmd.timeout_ms.max(1_000)).div_ceil(1_000);
        // Shared ProcessRunner path — no uncancellable side channel.
        let outcome = crate::tools::command_run(project, &cmd.command, alive, timeout_secs);
        let combined = crate::tools::redact_secrets(&outcome.output);
        let code = outcome.exit_code;
        let ok = outcome.kind == crate::tools::CommandOutcomeKind::Success;
        let output_tail = tail_chars(&combined, 2500);
        let failure = if ok {
            None
        } else {
            Some(FailureAnalyzer::analyze(&cmd.command, code, &combined))
        };
        VerifyOutcome {
            command: cmd.clone(),
            exit_code: code,
            ok,
            timed_out: outcome.timed_out,
            cancelled: outcome.cancelled,
            cwd: project.display().to_string(),
            duration_ms: outcome.duration_ms,
            output_tail,
            failure,
        }
    }

    /// Run all inferred commands; stop early on first hard failure optional.
    pub fn run_all(
        &self,
        project: &Path,
        alive: &dyn Fn() -> bool,
        stop_on_fail: bool,
    ) -> Vec<VerifyOutcome> {
        let cmds = self.infer(project);
        self.run_commands(project, &cmds, alive, stop_on_fail)
    }

    /// Run an explicit command list (already filtered by skill policy).
    /// Same missing-tool skip rules as [`Self::run_plan`].
    pub fn run_commands(
        &self,
        project: &Path,
        cmds: &[VerifyCommand],
        alive: &dyn Fn() -> bool,
        stop_on_fail: bool,
    ) -> Vec<VerifyOutcome> {
        let mut out = Vec::new();
        let cwd = project.display().to_string();
        let mut missing_tools: Vec<String> = Vec::new();
        for cmd in cmds {
            if !alive() {
                break;
            }
            let binary = first_token(&cmd.command);
            if missing_tools.iter().any(|tool| *tool == binary) {
                out.push(blocked_outcome(
                    cmd,
                    &cwd,
                    &format!("{binary}: command not found (tool unavailable)"),
                ));
                continue;
            }
            let outcome = self.run_one(project, cmd, alive);
            let failed = !outcome.ok;
            if failed {
                if let Some(tool) = CommandFailureKind::missing_tool(&outcome.output_tail) {
                    missing_tools.push(tool);
                    out.push(outcome);
                    continue;
                }
            }
            out.push(outcome);
            if failed && stop_on_fail {
                break;
            }
        }
        out
    }

    /// Combine outcomes into a final status.
    pub fn final_status(outcomes: &[VerifyOutcome]) -> FinalStatus {
        if outcomes.is_empty() {
            return FinalStatus::NotVerified;
        }
        let passed = outcomes.iter().filter(|o| o.ok).count();
        let total = outcomes.len();
        if passed == total {
            FinalStatus::Verified
        } else if passed > 0 {
            FinalStatus::PartiallyVerified
        } else {
            FinalStatus::VerificationFailed
        }
    }
}

/// Leading binary of a shell command (`cargo test --lib` → `cargo`).
fn first_token(command: &str) -> String {
    command
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// A command that was never spawned because its tool is already known missing.
fn blocked_outcome(cmd: &VerifyCommand, cwd: &str, reason: &str) -> VerifyOutcome {
    VerifyOutcome {
        command: cmd.clone(),
        exit_code: None,
        ok: false,
        timed_out: false,
        cancelled: false,
        cwd: cwd.to_owned(),
        duration_ms: 0,
        output_tail: reason.to_owned(),
        failure: Some(FailureReport {
            command: cmd.command.clone(),
            exit_code: None,
            primary_error: reason.to_owned(),
            relevant_files: Vec::new(),
        }),
    }
}

/// Compress command output into a small structured failure.
pub struct FailureAnalyzer;

impl FailureAnalyzer {
    pub fn analyze(command: &str, exit_code: Option<i32>, output: &str) -> FailureReport {
        let primary_error = primary_error(output);
        let relevant_files = relevant_files(output);
        FailureReport {
            command: command.to_owned(),
            exit_code,
            primary_error,
            relevant_files,
        }
    }
}

fn primary_error(output: &str) -> String {
    // Prefer structured compiler / test patterns.
    let patterns: [&str; 12] = [
        "error[E",
        "error:",
        "error TS",
        "FAILED",
        "AssertionError",
        "panicked at",
        "ERR!",
        "error: test failed",
        "test result: FAILED",
        "Traceback (most recent call last)",
        "Compile error",
        "failed to compile",
    ];
    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        for p in patterns {
            if t.contains(p) {
                return clip(t, 240);
            }
        }
    }
    // Fallback: first non-empty error-looking line or last line.
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    if let Some(last) = lines.last() {
        return clip(last.trim(), 240);
    }
    "(no error line found)".to_owned()
}

fn relevant_files(output: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in output.lines() {
        // path-like tokens with an extension
        for token in line.split_whitespace() {
            let clean =
                token.trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == ':' || c == ',');
            if looks_like_source_path(clean) && !files.iter().any(|f: &String| f == clean) {
                files.push(clean.to_owned());
            }
        }
        if files.len() >= 8 {
            break;
        }
    }
    files
}

fn looks_like_source_path(s: &str) -> bool {
    // Strip rustc/cargo style `:line:col` or `:line` suffixes.
    let mut s = s.trim();
    // e.g. src/lib.rs:10:5
    if let Some(idx) = s.find(".rs:") {
        s = &s[..idx + 3];
    } else if let Some(idx) = s.find(".ts:") {
        s = &s[..idx + 3];
    } else if let Some(idx) = s.find(".js:") {
        s = &s[..idx + 3];
    } else if let Some(idx) = s.find(".py:") {
        s = &s[..idx + 3];
    } else if s.contains(':') {
        // generic path:line
        let (head, tail) = s.rsplit_once(':').unwrap_or((s, ""));
        if tail.chars().all(|c| c.is_ascii_digit()) && head.contains('.') {
            s = head;
        }
    }
    if s.len() < 5 || s.len() > 200 || s.contains(' ') {
        return false;
    }
    if !(s.contains('/') || s.contains('\\')) {
        return false;
    }
    const EXT: [&str; 16] = [
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".java", ".json", ".toml", ".md",
        ".css", ".html", ".yml", ".yaml", ".toml",
    ];
    EXT.iter().any(|e| s.ends_with(e))
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut o: String = s.chars().take(max - 1).collect();
        o.push('…');
        o
    }
}

fn tail_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let skip = s.chars().count() - max;
    let mut out: String = s.chars().skip(skip).collect();
    out.insert(0, '…');
    out
}

/// Extract a concrete test filter from a criterion description, if any.
///
/// Recognizes:
/// * backtick-quoted identifiers (`test_foo`, `foo::bar`)
/// * bare `test_*` / `*_test` tokens
/// * "the X test" where X looks like an identifier
fn extract_test_filter(description: &str) -> Option<String> {
    // Backtick identifiers
    let mut rest = description;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let end = after.find('`')?;
        let inner = after[..end].trim();
        if looks_like_test_filter(inner) {
            return Some(inner.to_owned());
        }
        rest = &after[end + 1..];
    }
    // test_* tokens
    for token in description.split_whitespace() {
        let clean = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != ':');
        if looks_like_test_filter(clean) {
            return Some(clean.to_owned());
        }
    }
    // "the login_panic test"
    let lower = description.to_ascii_lowercase();
    if let Some(idx) = lower.find("the ") {
        let after = &description[idx + 4..];
        if let Some(end) = after.to_ascii_lowercase().find(" test") {
            let inner = after[..end].trim();
            if looks_like_test_filter(inner) {
                return Some(inner.to_owned());
            }
        }
    }
    None
}

fn looks_like_test_filter(s: &str) -> bool {
    if s.len() < 4 || s.len() > 80 {
        return false;
    }
    if s.contains(' ') {
        return false;
    }
    // Must look like a Rust/node test path, not prose.
    let lower = s.to_ascii_lowercase();
    lower.starts_with("test_")
        || lower.ends_with("_test")
        || lower.contains("::test")
        || (s.contains("::") && !s.contains('.'))
        || (s
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == ':' || c == '.')
            && s.contains('_')
            && !lower.contains("the")
            && !lower.contains("verification")
            && !lower.contains("command"))
}

/// Failure report helper re-export used by callers that only need detect.
#[allow(dead_code)]
pub fn detect_verify_command(project: &Path) -> Option<String> {
    VerificationRunner::default()
        .infer(project)
        .first()
        .map(|c| c.command.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture_with_failing_test() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kodo-verify-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Tiny cargo project would be heavy; use package.json scripts instead.
        fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"typecheck":"node -e \"process.exit(1)\"","test":"node -e \"console.error('AssertionError: expected 1 got 2 at src/app.test.js'); process.exit(1)\"","lint":"node -e \"process.exit(0)\"","build":"node -e \"process.exit(0)\""}}"#,
        )
        .unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/app.test.js"), "// test\n").unwrap();
        dir
    }

    #[test]
    fn infer_reads_package_scripts() {
        let root = fixture_with_failing_test();
        let runner = VerificationRunner::new(5_000);
        let cmds = runner.infer(&root);
        assert!(cmds
            .iter()
            .any(|c| c.kind == VerifyKind::Test && c.command.contains("test")));
        assert!(cmds.iter().any(|c| c.kind == VerifyKind::Typecheck));
        assert!(cmds.iter().any(|c| c.kind == VerifyKind::Build));
        assert!(cmds.iter().any(|c| c.kind == VerifyKind::Lint));
        for c in &cmds {
            assert!(c.timeout_ms > 0, "each command needs a timeout");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn infer_cargo_project() {
        let dir = std::env::temp_dir().join(format!("kodo-cv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let cmds = VerificationRunner::default().infer(&dir);
        assert!(cmds.iter().any(|c| c.command.contains("cargo test")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn analyzer_compresses_failure() {
        let out = r#"
error[E0308]: mismatched types
  --> src/lib.rs:10:5
   |
10 |     let x = 1;
   |             ^ expected `&str`
"#;
        let report = FailureAnalyzer::analyze("cargo check --quiet", Some(101), out);
        assert_eq!(report.exit_code, Some(101));
        assert_eq!(report.command, "cargo check --quiet");
        assert!(
            report.primary_error.contains("error[E0308]"),
            "{}",
            report.primary_error
        );
        assert!(
            report
                .relevant_files
                .iter()
                .any(|f| f.contains("src/lib.rs")),
            "files={:?}",
            report.relevant_files
        );
        let block = report.to_prompt_block();
        assert!(block.contains("exit_code: 101"));
        assert!(block.contains("primary_error:"));
    }

    #[test]
    fn run_failing_command_produces_failure_report() {
        let root = fixture_with_failing_test();
        let runner = VerificationRunner::new(10_000);
        let cmds = runner.infer(&root);
        let test_cmd = cmds
            .iter()
            .find(|c| c.kind == VerifyKind::Test)
            .cloned()
            .expect("test cmd");
        let outcome = runner.run_one(&root, &test_cmd, &|| true);
        assert!(!outcome.ok);
        assert_ne!(outcome.exit_code, Some(0));
        let f = outcome.failure.expect("failure");
        assert!(!f.primary_error.is_empty());
        assert_eq!(f.command, test_cmd.command);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn timeout_marks_failed() {
        let dir = std::env::temp_dir().join(format!("kodo-to-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let runner = VerificationRunner::new(50);
        let cmd = VerifyCommand {
            kind: VerifyKind::Test,
            command: "sleep 2".into(),
            timeout_ms: 100,
        };
        let outcome = runner.run_one(&dir, &cmd, &|| true);
        assert!(outcome.timed_out || !outcome.ok);
        assert!(!outcome.ok);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn final_status_includes_cancelled_and_blocked() {
        assert_eq!(FinalStatus::Cancelled.label(), "Cancelled");
        assert_eq!(FinalStatus::Blocked.label(), "Blocked");
        assert_eq!(FinalStatus::ProviderError.label(), "Provider error");
        assert_ne!(
            FinalStatus::Cancelled.label(),
            FinalStatus::Verified.label()
        );
    }

    #[test]
    fn targeted_infer_narrows_to_changed_crate() {
        let dir = std::env::temp_dir().join(format!("kodo-tgt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("crates/agent")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/agent\"]\n",
        )
        .unwrap();
        fs::write(
            dir.join("crates/agent/Cargo.toml"),
            "[package]\nname = \"kodo-agent\"\n",
        )
        .unwrap();
        let runner = VerificationRunner::new(5_000);
        let cmds = runner.infer_targeted(&dir, &["crates/agent/src/lib.rs".into()]);
        assert!(
            cmds.iter()
                .any(|c| c.command.contains("cargo test -p kodo-agent")),
            "cmds={:?}",
            cmds.iter().map(|c| &c.command).collect::<Vec<_>>()
        );
        assert!(
            !cmds.iter().any(|c| c.command.contains("--workspace")),
            "targeted run must not fall back to workspace tests first: {:?}",
            cmds
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_timeout_kills_process_tree() {
        let dir = std::env::temp_dir().join(format!("kodo-vto-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let marker = dir.join("v.marker");
        // Grandchild would write at t=3s; timeout at ~1s must kill the group first.
        let script = format!("(sleep 3; echo x > '{}') & sleep 10", marker.display());
        let runner = VerificationRunner::new(5_000);
        let cmd = VerifyCommand {
            kind: VerifyKind::Test,
            command: script,
            timeout_ms: 400,
        };
        let outcome = runner.run_one(&dir, &cmd, &|| true);
        assert!(outcome.timed_out || outcome.cancelled, "{outcome:?}");
        assert!(!outcome.ok);
        std::thread::sleep(std::time::Duration::from_millis(3500));
        assert!(
            !marker.exists(),
            "verify timeout must kill the process tree"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn final_status_classification() {
        let pass = VerifyOutcome {
            command: VerifyCommand::test("true"),
            exit_code: Some(0),
            ok: true,
            timed_out: false,
            cancelled: false,
            cwd: ".".into(),
            duration_ms: 1,
            output_tail: String::new(),
            failure: None,
        };
        let fail = VerifyOutcome {
            command: VerifyCommand::test("false"),
            exit_code: Some(1),
            ok: false,
            timed_out: false,
            cancelled: false,
            cwd: ".".into(),
            duration_ms: 1,
            output_tail: String::new(),
            failure: None,
        };
        assert_eq!(
            VerificationRunner::final_status(&[]),
            FinalStatus::NotVerified
        );
        assert_eq!(
            VerificationRunner::final_status(std::slice::from_ref(&pass)),
            FinalStatus::Verified
        );
        assert_eq!(
            VerificationRunner::final_status(&[pass.clone(), fail.clone()]),
            FinalStatus::PartiallyVerified
        );
        let fail2 = VerifyOutcome {
            command: VerifyCommand::test("false"),
            exit_code: Some(1),
            ok: false,
            timed_out: false,
            cancelled: false,
            cwd: ".".into(),
            duration_ms: 1,
            output_tail: String::new(),
            failure: None,
        };
        assert_eq!(
            VerificationRunner::final_status(&[fail2, fail]),
            FinalStatus::VerificationFailed
        );
        let _ = pass;
    }

    #[test]
    fn successful_command_is_ok() {
        let dir = std::env::temp_dir().join(format!("kodo-ok-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let runner = VerificationRunner::new(5_000);
        let cmd = VerifyCommand::test("true");
        let outcome = runner.run_one(&dir, &cmd, &|| true);
        assert!(outcome.ok);
        assert!(outcome.failure.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_verification_cancel_is_not_completed() {
        let dir = std::env::temp_dir().join(format!("kodo-vcancel-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let runner = VerificationRunner::new(30_000);
        let cmd = VerifyCommand {
            kind: VerifyKind::Test,
            command: "sleep 20".into(),
            timeout_ms: 15_000,
        };
        let outcome = runner.run_one(&dir, &cmd, &|| false);
        assert!(outcome.cancelled, "expected cancel, got {outcome:?}");
        assert!(!outcome.ok, "cancelled verify must not be ok");
        assert_ne!(
            VerificationRunner::final_status(std::slice::from_ref(&outcome)),
            FinalStatus::Verified
        );
        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // VerificationPlan / criterion-scoped evidence / repair regressions
    // -----------------------------------------------------------------------

    #[test]
    fn verify_exact_regression_tier_precedes_package_and_broad() {
        let dir = temp_verify_dir("plan_order");
        fs::write(dir.join("Cargo.toml"), "[package]\nname=\"t\"\n").unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/lib.rs"), "").unwrap();
        let criteria = vec![
            (
                "c_specific".to_string(),
                "Fix the `test_login_panic` regression".to_string(),
            ),
            (
                "c_generic".to_string(),
                "Verification commands pass".to_string(),
            ),
        ];
        let plan = VerificationRunner::build_plan(
            &dir,
            &criteria,
            &["crates/agent/src/lib.rs".into()],
            "bug-fix",
        );
        assert!(!plan.commands.is_empty(), "plan={:?}", plan.commands);
        // Exact tier first when a named test exists.
        if let Some(first) = plan.commands.first() {
            assert_eq!(
                first.tier,
                VerifyTier::ExactRegression,
                "exact must be first: {:?}",
                plan.commands
            );
            assert!(first.command.command.contains("test_login_panic"));
            assert_eq!(first.criterion_ids, vec!["c_specific".to_string()]);
        }
        // Tier order is non-decreasing.
        let tiers: Vec<VerifyTier> = plan.commands.iter().map(|c| c.tier).collect();
        let mut sorted = tiers.clone();
        sorted.sort();
        assert_eq!(tiers, sorted, "commands must be sorted by tier");
        // Package/typecheck targets must NOT include the exact-only criterion.
        for cmd in plan
            .commands
            .iter()
            .filter(|c| c.tier != VerifyTier::ExactRegression)
        {
            assert!(
                !cmd.criterion_ids.contains(&"c_specific".to_string()),
                "unrelated pass must not target exact criterion: {:?}",
                cmd
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_unrelated_test_pass_cannot_prove_target_criterion() {
        let dir = temp_verify_dir("unrelated");
        fs::write(dir.join("Cargo.toml"), "[package]\nname=\"t\"\n").unwrap();
        let criteria = vec![(
            "c_target".to_string(),
            "Must pass `test_login_panic`".to_string(),
        )];
        let plan = VerificationRunner::build_plan(&dir, &criteria, &[], "bug-fix");
        // Simulate: broad/package command that does NOT target c_target.
        let mut evidence = VerificationEvidence {
            command: "cargo test --workspace --quiet".into(),
            cwd: dir.display().to_string(),
            exit_code: Some(0),
            duration_ms: 10,
            criterion_ids: vec![], // unrelated run — no binding
            output_summary: "all tests passed".into(),
            truncated: false,
            ok: true,
            failure_class: None,
        };
        // Empty criterion_ids must not prove c_target.
        assert!(!evidence.criterion_ids.iter().any(|id| id == "c_target"));
        // Even if we force wrong binding, exact criterion stays unmet without
        // an evidence item bound to c_target from the exact command.
        evidence.criterion_ids = vec!["c_other".into()];
        assert!(!evidence.criterion_ids.contains(&"c_target".to_string()));

        // Binding-aware bag check: only bound-to-c_target TestPassed counts.
        use crate::evidence::{EvidenceBag, EvidenceItem, EvidenceKind, EvidenceRequirement};
        let mut bag = EvidenceBag::default();
        bag.push(
            EvidenceItem::new(
                "v1",
                "verify",
                EvidenceKind::TestPassed {
                    command: "cargo test --workspace --quiet".into(),
                },
            )
            .bound_to("c_other"),
        );
        let req = EvidenceRequirement::VerificationPassed;
        assert!(
            !bag.satisfies_for(Some("c_target"), &req),
            "pass bound to c_other must not prove c_target"
        );
        bag.push(
            EvidenceItem::new(
                "v2",
                "verify",
                EvidenceKind::TestPassed {
                    command: "cargo test test_login_panic --quiet".into(),
                },
            )
            .bound_to("c_target"),
        );
        assert!(
            bag.satisfies_for(Some("c_target"), &req),
            "exact bound pass must prove c_target"
        );
        assert!(!plan.commands.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_exact_regression_pass_can_prove_criterion() {
        use crate::evidence::{EvidenceBag, EvidenceItem, EvidenceKind, EvidenceRequirement};
        let mut bag = EvidenceBag::default();
        bag.push(
            EvidenceItem::new(
                "v",
                "verify",
                EvidenceKind::TestPassed {
                    command: "cargo test test_login_panic --quiet".into(),
                },
            )
            .bound_to("c_exact"),
        );
        assert!(bag.satisfies_for(Some("c_exact"), &EvidenceRequirement::VerificationPassed));
    }

    #[test]
    fn verify_infra_failure_is_not_product_class() {
        assert_eq!(
            classify_verify_failure("cargo test foo", "error: command not found"),
            FailureClass::Infrastructure
        );
        assert_eq!(
            classify_verify_failure("cargo test foo", "cannot find crate `nope`"),
            FailureClass::Infrastructure
        );
        assert_eq!(
            classify_verify_failure("npm test", "Could not resolve host: registry.npmjs.org"),
            FailureClass::Infrastructure
        );
        assert_eq!(
            classify_verify_failure(
                "cargo test",
                "test login::test_login_panic ... FAILED\nassertion failed"
            ),
            FailureClass::Product
        );
    }

    #[test]
    fn verify_repair_decision_infra_blocks_repair() {
        let plan = VerificationPlan {
            criterion_ids: vec!["c1".into()],
            changed_files: vec![],
            package: None,
            task_type: "bug-fix".into(),
            commands: vec![PlannedVerifyCommand {
                command: VerifyCommand::test("cargo test"),
                criterion_ids: vec!["c1".into()],
                tier: VerifyTier::Package,
            }],
        };
        let ev = VerificationEvidence {
            command: "cargo test".into(),
            cwd: ".".into(),
            exit_code: Some(101),
            duration_ms: 5,
            criterion_ids: vec!["c1".into()],
            output_summary: "error: command not found".into(),
            truncated: false,
            ok: false,
            failure_class: Some(FailureClass::Infrastructure),
        };
        let decision = RepairDecision::from_evidence(&[ev], &plan, true);
        assert!(!decision.can_repair, "infra must not open product repair");
        assert_eq!(decision.failure_class, FailureClass::Infrastructure);
        assert!(decision.hint.contains("Infrastructure"));
    }

    #[test]
    fn verify_repair_decision_product_and_budget() {
        let plan = VerificationPlan::default();
        let ev = VerificationEvidence {
            command: "cargo test".into(),
            cwd: ".".into(),
            exit_code: Some(1),
            duration_ms: 5,
            criterion_ids: vec!["c1".into()],
            output_summary: "assertion failed".into(),
            truncated: false,
            ok: false,
            failure_class: Some(FailureClass::Product),
        };
        let decision = RepairDecision::from_evidence(std::slice::from_ref(&ev), &plan, true);
        assert!(decision.can_repair);
        assert_eq!(decision.failure_class, FailureClass::Product);

        let decision = RepairDecision::from_evidence(std::slice::from_ref(&ev), &plan, false);
        assert!(!decision.can_repair);
        assert!(decision.budget_exhausted);
    }

    #[test]
    fn verify_evidence_records_criterion_ids_and_truncation() {
        let outcome = VerifyOutcome {
            command: VerifyCommand::test("cargo test x"),
            exit_code: Some(0),
            ok: true,
            timed_out: false,
            cancelled: false,
            cwd: "/tmp".into(),
            duration_ms: 42,
            output_tail: "ok".into(),
            failure: None,
        };
        let ev = VerificationEvidence::from_outcome(&outcome, "/tmp", vec!["c_verify".into()], 500);
        assert_eq!(ev.criterion_ids, vec!["c_verify".to_string()]);
        assert_eq!(ev.duration_ms, 42);
        assert_eq!(ev.exit_code, Some(0));
        assert!(ev.ok);
        assert!(!ev.truncated);

        let long = VerifyOutcome {
            output_tail: "x".repeat(3000),
            ..outcome
        };
        let ev = VerificationEvidence::from_outcome(&long, "/tmp", vec![], 500);
        assert!(ev.truncated);
        assert!(ev.output_summary.chars().count() <= 501);
    }

    fn temp_verify_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kodo-vplan-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
    #[test]
    fn failure_kind_classifies_structured_signals() {
        use CommandFailureKind::*;
        assert_eq!(
            CommandFailureKind::classify(Some(127), "/bin/sh: cargo: command not found", false),
            Some(CommandNotFound)
        );
        assert_eq!(
            CommandFailureKind::classify(
                Some(127),
                "/bin/sh: cargo: command not found",
                true // denial wins: the user never let it run
            ),
            Some(Denied)
        );
        assert_eq!(
            CommandFailureKind::classify(Some(126), "permission denied", false),
            Some(PermissionDenied)
        );
        assert_eq!(
            CommandFailureKind::classify(Some(2), "error: test failed", false),
            Some(NonZeroExit)
        );
        assert_eq!(
            CommandFailureKind::classify(None, "timed out after 90s", false),
            Some(Timeout)
        );
        assert_eq!(
            CommandFailureKind::classify(Some(1), "would be overwritten by merge", false),
            Some(WorkspaceConflict)
        );
        assert_eq!(CommandFailureKind::classify(Some(0), "ok", false), None);
    }

    #[test]
    fn missing_tool_reads_the_binary_name() {
        assert_eq!(
            CommandFailureKind::missing_tool("/bin/sh: cargo: command not found"),
            Some("cargo".to_owned())
        );
        assert_eq!(
            CommandFailureKind::missing_tool("bash: flub: command not found"),
            Some("flub".to_owned())
        );
        assert_eq!(CommandFailureKind::missing_tool("error: test failed"), None);
    }

    #[test]
    fn a_missing_tool_blocks_same_binary_and_skips_the_doomed_chain() {
        let dir = fixture_with_failing_test();
        let runner = VerificationRunner::new(10_000);
        let cmds = vec![
            VerifyCommand::build("kodo_no_such_tool_xyz build"),
            VerifyCommand::test("kodo_no_such_tool_xyz test"),
            VerifyCommand::test("node -e \"process.exit(0)\""),
        ];
        let out = runner.run_commands(&dir, &cmds, &|| true, true);
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(out.len(), 3, "every planned command gets a row");
        assert!(!out[0].ok, "first attempt runs and fails");
        assert!(!out[1].ok, "second is blocked");
        assert_eq!(out[1].duration_ms, 0, "blocked rows never spawned");
        assert!(
            out[1].output_tail.contains("tool unavailable"),
            "blocked rows say why: {}",
            out[1].output_tail
        );
        assert!(out[2].ok, "a different tool still runs after a missing one");
    }
}
