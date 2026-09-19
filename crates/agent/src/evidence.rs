//! Criterion-specific evidence. Tool success alone never proves a criterion.
//!
//! "Command failed" and "successfully reproduced the target bug" are two
//! different concepts: only [`EvidenceKind::ReproductionSucceeded`] (a
//! fingerprint-matched [`CommandExpectation::Reproduction`]) counts as
//! reproduction. Bare non-zero exits never do.

use crate::protocol::{ToolErrorCode, ToolName, ToolResult};
use serde::{Deserialize, Serialize};

/// Tokens that are too generic to fingerprint a target bug on their own
/// (unless the user task explicitly opts in via
/// [`FailureExpectation::allow_generic_fingerprint`]).
pub fn is_generic_failure_token(token: &str) -> bool {
    matches!(
        token.trim().to_ascii_lowercase().as_str(),
        "fail"
            | "failed"
            | "failure"
            | "failing"
            | "error"
            | "errors"
            | "err"
            | "non-zero"
            | "nonzero"
            | "non zero"
            | "exit"
    )
}

/// Task-specific fingerprint for a **successful** bug reproduction.
///
/// `ReproductionSucceeded` requires the command to have executed, failed in
/// the expected way, and matched at least one of: `expected_exit`,
/// `output_contains`, `stderr_contains`, `test_name` (combinable; at least
/// one constraint must be present). Generic-only needles such as `"fail"` /
/// `"error"` are rejected unless explicitly allowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FailureExpectation {
    /// Exact non-zero exit code required when set.
    #[serde(default)]
    pub expected_exit: Option<i32>,
    /// Substrings that must appear in combined stdout+stderr.
    #[serde(default)]
    pub output_contains: Vec<String>,
    /// Substrings that must appear in stderr (or the error message).
    #[serde(default)]
    pub stderr_contains: Vec<String>,
    /// Named failing test that must appear in the output.
    #[serde(default)]
    pub test_name: Option<String>,
    /// Explicit user opt-in: allow `"fail"`/`"error"` as the sole fingerprint.
    #[serde(default)]
    pub allow_generic_fingerprint: bool,
    /// When no specific fingerprint fields are set, mint a stable fingerprint
    /// from the first observed failure (exit + extracted test/assertion).
    #[serde(default)]
    pub capture_on_first_failure: bool,
}

impl FailureExpectation {
    /// Capture-on-first-failure: fingerprint is minted from the first
    /// application-level failure (never from command-not-found / auth noise).
    pub fn capture() -> Self {
        Self {
            capture_on_first_failure: true,
            ..Self::default()
        }
    }

    pub fn with_test_name(name: impl Into<String>) -> Self {
        Self {
            test_name: Some(name.into()),
            ..Self::default()
        }
    }

    pub fn with_exit(code: i32) -> Self {
        Self {
            expected_exit: Some(code),
            ..Self::default()
        }
    }

    pub fn with_output(mut self, needle: impl Into<String>) -> Self {
        self.output_contains.push(needle.into());
        self
    }

    pub fn with_stderr(mut self, needle: impl Into<String>) -> Self {
        self.stderr_contains.push(needle.into());
        self
    }

    /// User explicitly asked for a generic token fingerprint.
    pub fn allowing_generic(mut self) -> Self {
        self.allow_generic_fingerprint = true;
        self
    }

    /// At least one fingerprint field is present (constraint exists).
    pub fn has_any_constraint(&self) -> bool {
        self.expected_exit.is_some()
            || self.test_name.is_some()
            || !self.output_contains.is_empty()
            || !self.stderr_contains.is_empty()
    }

    /// At least one constraint is specific (not merely `"fail"`/`"error"`).
    pub fn has_specific_fingerprint(&self) -> bool {
        if self.expected_exit.is_some() || self.test_name.is_some() {
            return true;
        }
        self.output_contains
            .iter()
            .chain(&self.stderr_contains)
            .any(|s| !is_generic_failure_token(s))
    }

    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if let Some(code) = self.expected_exit {
            parts.push(format!("exit={code}"));
        }
        if let Some(name) = &self.test_name {
            parts.push(format!("test={name}"));
        }
        for n in &self.output_contains {
            parts.push(format!("out=/{n}/"));
        }
        for n in &self.stderr_contains {
            parts.push(format!("err=/{n}/"));
        }
        if parts.is_empty() {
            if self.capture_on_first_failure {
                "capture-on-first-failure".into()
            } else {
                "empty-fingerprint".into()
            }
        } else {
            parts.join(" & ")
        }
    }

    /// ReproductionSucceeded = executed AND failed as expected AND fingerprint
    /// matched. Returns false for success exits, missing constraints, or
    /// generic-only fingerprints without explicit opt-in.
    pub fn evaluate(&self, exit_code: Option<i32>, stdout: &str, stderr: &str) -> bool {
        let failed_ok = match self.expected_exit {
            Some(code) => exit_code == Some(code),
            None => matches!(exit_code, Some(c) if c != 0),
        };
        if !failed_ok {
            return false;
        }
        if !self.has_any_constraint() {
            return false;
        }
        if !self.allow_generic_fingerprint && !self.has_specific_fingerprint() {
            // Empty or generic-only ("fail"/"error") fingerprint.
            return false;
        }

        let combined = format!("{stdout}\n{stderr}");
        if let Some(name) = &self.test_name {
            if !combined
                .to_ascii_lowercase()
                .contains(&name.to_ascii_lowercase())
            {
                return false;
            }
        }
        for needle in &self.output_contains {
            if !combined.contains(needle.as_str())
                && !combined
                    .to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase())
            {
                return false;
            }
        }
        for needle in &self.stderr_contains {
            if !stderr.contains(needle.as_str())
                && !stderr
                    .to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase())
            {
                return false;
            }
        }
        true
    }

    /// Mint a stable fingerprint from the first real failure.
    ///
    /// Rejects command-not-found / environmental noise and anything that
    /// cannot produce a specific application-level fingerprint.
    pub fn mint_from_observation(
        exit_code: Option<i32>,
        stdout: &str,
        stderr: &str,
        command: &str,
    ) -> Option<Self> {
        // Shell cannot-executable codes are never an application bug.
        if matches!(exit_code, Some(126) | Some(127)) {
            return None;
        }
        let raw = format!("{command}\n{stdout}\n{stderr}");
        let lower = raw.to_ascii_lowercase();

        const ENV: &[&str] = &[
            "command not found",
            "no such file or directory",
            "permission denied",
            "connection refused",
            "econnrefused",
            "network is unreachable",
            "could not resolve host",
            "temporary failure in name resolution",
        ];
        let looks_env = ENV.iter().any(|m| lower.contains(m));
        let test_name = extract_test_name(&raw);
        let assertion = extract_assertion_marker(&raw);
        let app_level = test_name.is_some()
            || assertion.is_some()
            || lower.contains("assertion failed")
            || lower.contains("panicked at")
            || lower.contains("assertionerror");
        if looks_env && !app_level {
            return None;
        }

        // Auth/network-only failures without a target test/assertion cannot
        // prove the target regression.
        let auth_only = (lower.contains("unauthorized")
            || lower.contains("401 ")
            || lower.contains("403 ")
            || lower.contains("forbidden"))
            && !app_level;
        if auth_only {
            return None;
        }

        let mut fe = Self {
            expected_exit: exit_code,
            ..Self::default()
        };
        if let Some(name) = test_name {
            fe.test_name = Some(name);
        }
        if let Some(marker) = assertion {
            fe.output_contains.push(marker);
        }
        // Minted fingerprints must be specific (bare exit code alone is too
        // weak without a test name or assertion marker).
        if fe.test_name.is_none() && fe.output_contains.is_empty() {
            return None;
        }
        if !fe.evaluate(exit_code, stdout, stderr) {
            return None;
        }
        Some(fe)
    }
}

/// Extract a named failing test from harness output (cargo / node styles).
fn extract_test_name(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let t = line.trim();
        // cargo: `test module::name ... FAILED` / `test name ... ok`
        if let Some(rest) = t.strip_prefix("test ") {
            if let Some(idx) = rest.find(" ...") {
                let name = rest[..idx].trim();
                if !name.is_empty() && name != "result" && !name.eq_ignore_ascii_case("ignored") {
                    // Only FAILED-style lines are failures; callers only mint on fail.
                    return Some(name.to_owned());
                }
            }
        }
        // `running 1 test` is not a name; skip.
        // node/custom: `FAIL test_login_panic` / `✗ test_login_panic`
        for prefix in ["FAIL ", "fail ", "✗ ", "x "] {
            if let Some(rest) = t.strip_prefix(prefix) {
                let name = rest.trim().trim_end_matches(':');
                if !name.is_empty()
                    && !name.contains(' ')
                    && name
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == ':')
                {
                    return Some(name.to_owned());
                }
            }
        }
    }
    None
}

/// Extract a stable assertion/panic marker from failure output.
fn extract_assertion_marker(raw: &str) -> Option<String> {
    const MARKERS: &[&str] = &[
        "assertion failed",
        "assertionerror",
        "panicked at",
        "left=",
        "expected outcome",
    ];
    let lower = raw.to_ascii_lowercase();
    for marker in MARKERS {
        if lower.contains(marker) {
            // Prefer the full source line when it is distinctive enough.
            for line in raw.lines() {
                if line.to_ascii_lowercase().contains(marker) {
                    let trimmed = line.trim();
                    if trimmed.chars().count() >= marker.chars().count() + 4 {
                        let capped: String = trimmed.chars().take(120).collect();
                        return Some(capped);
                    }
                }
            }
            return Some((*marker).to_owned());
        }
    }
    None
}

/// How a command result is interpreted for evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandExpectation {
    /// Command must exit 0.
    ExpectedSuccess,
    /// Command must exit non-zero. Yields [`EvidenceKind::CommandFailed`]
    /// only — never reproduction proof.
    ExpectedFailure,
    /// Successful bug reproduction: failed **and** matched a task-specific
    /// [`FailureExpectation`] fingerprint.
    Reproduction(FailureExpectation),
    /// Command must exit 0 AND output must contain the needle.
    ExpectedOutputMatch { needle: String },
}

impl CommandExpectation {
    pub fn label(&self) -> String {
        match self {
            Self::ExpectedSuccess => "exit 0".into(),
            Self::ExpectedFailure => "non-zero exit (not reproduction)".into(),
            Self::Reproduction(fe) => format!("reproduce({})", fe.label()),
            Self::ExpectedOutputMatch { needle } => format!("pass + /{needle}/"),
        }
    }

    pub fn is_reproduction(&self) -> bool {
        matches!(self, Self::Reproduction(_))
    }

    /// Evaluate a command outcome against this expectation (non-capture).
    pub fn evaluate(&self, exit_code: Option<i32>, output: &str) -> bool {
        match self {
            Self::ExpectedSuccess => exit_code == Some(0),
            Self::ExpectedFailure => matches!(exit_code, Some(c) if c != 0),
            Self::Reproduction(fe) => fe.evaluate(exit_code, output, ""),
            Self::ExpectedOutputMatch { needle } => {
                exit_code == Some(0) && output.contains(needle.as_str())
            }
        }
    }
}

/// Typed structural evidence. Never model prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceKind {
    FileRead {
        path: String,
    },
    SearchHit {
        query: String,
        hits: usize,
    },
    CommandSucceeded {
        command: String,
        exit_code: i32,
    },
    /// Bare command failure (non-zero). **Not** reproduction proof.
    CommandFailed {
        command: String,
        exit_code: Option<i32>,
    },
    /// First-class: the target bug was successfully reproduced — command
    /// executed, failed as expected, and matched the task fingerprint.
    ReproductionSucceeded {
        command: String,
        exit_code: Option<i32>,
        /// Concrete fingerprint that matched (stable across fix/regression).
        expectation: FailureExpectation,
    },
    FileChanged {
        path: String,
    },
    PatchApplied {
        path: String,
    },
    TestPassed {
        command: String,
    },
    TestFailed {
        command: String,
    },
    BuildPassed {
        command: String,
    },
    LintPassed {
        command: String,
    },
    UserApproval {
        detail: String,
    },
    DiffReviewed {
        paths: Vec<String>,
    },
    ContextObservation {
        path: String,
    },
}

/// Sentinel `criterion_id` meaning the evidence is explicitly reusable
/// across every acceptance criterion.
pub const REUSABLE_CRITERION: &str = "*";

/// Semantic acceptance targets that must never fall back to empty-string
/// wildcard matchers or bare `AnyToolSuccess`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SemanticTarget {
    /// Root cause located — needs a real search hit, not any read/command.
    RootCause,
    /// Failure reproduced — needs a fingerprint-matched
    /// [`EvidenceKind::ReproductionSucceeded`], never a bare non-zero exit.
    Reproduction,
    /// Behavior implemented — needs a real file change / patch.
    BehaviorImplemented,
    /// Regression prevented — needs passing test/build/lint evidence.
    RegressionPrevented,
}

impl SemanticTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::RootCause => "root-cause",
            Self::Reproduction => "reproduction",
            Self::BehaviorImplemented => "behavior-implemented",
            Self::RegressionPrevented => "regression-prevented",
        }
    }
}

/// One proven observation: provenance (`source`), optional semantic target
/// binding (`criterion_id`), structural observation (`kind`), and — when the
/// requirement carries an expectation — that expectation lives on the
/// requirement side. Never produced from model prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub id: String,
    /// Provenance: which tool / pipeline produced this observation.
    pub source: String,
    pub kind: EvidenceKind,
    /// Optional binding to one acceptance criterion id (see
    /// [`REUSABLE_CRITERION`] for the explicit cross-criterion sentinel).
    /// `None` = unbound observation.
    #[serde(default)]
    pub criterion_id: Option<String>,
}

impl EvidenceItem {
    pub fn new(id: impl Into<String>, source: impl Into<String>, kind: EvidenceKind) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            kind,
            criterion_id: None,
        }
    }

    /// Bind this observation to a specific criterion id.
    pub fn bound_to(mut self, criterion_id: impl Into<String>) -> Self {
        self.criterion_id = Some(criterion_id.into());
        self
    }

    /// Mark this observation explicitly reusable across criteria.
    pub fn reusable(mut self) -> Self {
        self.criterion_id = Some(REUSABLE_CRITERION.to_owned());
        self
    }

    /// May this item be considered by `criterion_id`?
    /// Unbound items are generally allowed (matcher decides); items bound to
    /// another criterion are rejected unless bound to [`REUSABLE_CRITERION`].
    pub fn binding_allows(&self, criterion_id: &str) -> bool {
        match self.criterion_id.as_deref() {
            None => true,
            Some(bound) => bound == REUSABLE_CRITERION || bound == criterion_id,
        }
    }

    pub fn describe(&self) -> String {
        let base = match &self.kind {
            EvidenceKind::FileRead { path } => format!("read:{path}"),
            EvidenceKind::SearchHit { query, hits } => format!("search:{query}#{hits}"),
            EvidenceKind::CommandSucceeded { command, exit_code } => {
                format!("cmd-ok:{command}#{exit_code}")
            }
            EvidenceKind::CommandFailed { command, exit_code } => format!(
                "cmd-fail:{command}#{}",
                exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into())
            ),
            EvidenceKind::ReproductionSucceeded {
                command,
                exit_code,
                expectation,
            } => format!(
                "reproduced:{command}#{} ({})",
                exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into()),
                expectation.label()
            ),
            EvidenceKind::FileChanged { path } => format!("changed:{path}"),
            EvidenceKind::PatchApplied { path } => format!("patch:{path}"),
            EvidenceKind::TestPassed { command } => format!("test-pass:{command}"),
            EvidenceKind::TestFailed { command } => format!("test-fail:{command}"),
            EvidenceKind::BuildPassed { command } => format!("build-pass:{command}"),
            EvidenceKind::LintPassed { command } => format!("lint-pass:{command}"),
            EvidenceKind::UserApproval { detail } => format!("approval:{detail}"),
            EvidenceKind::DiffReviewed { paths } => format!("diff:{}", paths.join(",")),
            EvidenceKind::ContextObservation { path } => format!("ctx:{path}"),
        };
        match &self.criterion_id {
            Some(id) => format!("{base} -> criterion:{id}"),
            None => base,
        }
    }
}

/// What a criterion/subtask actually requires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceRequirement {
    /// Any successful tool observation is enough (context gathering).
    AnyToolSuccess,
    /// A file under a path prefix/substring must be changed by Kodo.
    FileChanged { path_contains: String },
    /// Any Kodo file change.
    AnyFileChange,
    /// A verification command passed.
    VerificationPassed,
    /// A command with expectation-aware outcome matching this command substring.
    CommandOutcome {
        command_contains: String,
        expectation: CommandExpectation,
    },
    /// Search returned at least one hit for this query substring.
    /// Only real [`EvidenceKind::SearchHit`] observations match — never an
    /// arbitrary FileRead or successful command (empty query is not a
    /// wildcard over reads/commands).
    SearchHit { query_contains: String },
    /// Explicit source inspection: a successful project file read or search hit.
    /// Used for read/locate subtasks that are not root-cause criteria.
    FileInspected { path_contains: String },
    /// Semantic proof for criteria that must not degrade to `AnyToolSuccess`
    /// or empty-string wildcard matchers (root cause, reproduction,
    /// behavior implemented, regression prevented).
    SemanticProof { target: SemanticTarget },
    /// Fail-closed marker for free-form criteria that could not be structured
    /// reliably. Only evidence explicitly bound to the criterion id (or
    /// [`REUSABLE_CRITERION`]) can satisfy this — never bare tool success.
    RequiresExplicitEvidence,
    /// A specific tool must have succeeded (e.g. apply_patch).
    ToolSucceeded { tool: String },
    /// Explicit user approval of a dangerous/write step.
    UserApproval,
    /// Diff review recorded for at least one of these paths.
    DiffReviewed { path_contains: String },
    /// Pre-scan / pinned context observed for a path substring.
    ContextRead { path_contains: String },
}

impl EvidenceRequirement {
    pub fn label(&self) -> String {
        match self {
            Self::AnyToolSuccess => "any-tool-success".into(),
            Self::FileChanged { path_contains } => format!("file-changed:{path_contains}"),
            Self::AnyFileChange => "any-file-change".into(),
            Self::VerificationPassed => "verification-passed".into(),
            Self::CommandOutcome {
                command_contains,
                expectation,
            } => {
                format!("command:{command_contains} ({})", expectation.label())
            }
            Self::SearchHit { query_contains } => format!("search:{query_contains}"),
            Self::FileInspected { path_contains } => format!("file-inspected:{path_contains}"),
            Self::SemanticProof { target } => format!("semantic:{}", target.label()),
            Self::RequiresExplicitEvidence => "requires-explicit-evidence".into(),
            Self::ToolSucceeded { tool } => format!("tool-ok:{tool}"),
            Self::UserApproval => "user-approval".into(),
            Self::DiffReviewed { path_contains } => format!("diff-reviewed:{path_contains}"),
            Self::ContextRead { path_contains } => format!("context-read:{path_contains}"),
        }
    }

    /// True when the criterion could not be structured and must stay
    /// unresolved until explicitly bound evidence arrives.
    pub fn is_unresolved(&self) -> bool {
        matches!(self, Self::RequiresExplicitEvidence)
    }

    /// True when only criterion-bound evidence may satisfy this requirement.
    pub fn requires_explicit_binding(&self) -> bool {
        matches!(self, Self::RequiresExplicitEvidence)
    }

    pub fn matches(&self, item: &EvidenceItem) -> bool {
        // Pre-scan / context observations never complete strict work.
        let is_context_obs = matches!(item.kind, EvidenceKind::ContextObservation { .. })
            || item.source == "context";
        match (self, &item.kind) {
            (Self::AnyToolSuccess, kind) => {
                if is_context_obs {
                    return true; // contextual Read / gather
                }
                matches!(
                    kind,
                    EvidenceKind::CommandSucceeded { .. }
                        | EvidenceKind::SearchHit { .. }
                        | EvidenceKind::FileRead { .. }
                        | EvidenceKind::TestPassed { .. }
                        | EvidenceKind::BuildPassed { .. }
                        | EvidenceKind::LintPassed { .. }
                        | EvidenceKind::CommandFailed { .. }
                        | EvidenceKind::ReproductionSucceeded { .. }
                )
            }
            (Self::FileChanged { path_contains }, EvidenceKind::FileChanged { path })
            | (Self::FileChanged { path_contains }, EvidenceKind::PatchApplied { path }) => {
                path.contains(path_contains.as_str())
            }
            (Self::AnyFileChange, EvidenceKind::FileChanged { .. })
            | (Self::AnyFileChange, EvidenceKind::PatchApplied { .. }) => true,
            (Self::VerificationPassed, EvidenceKind::TestPassed { .. })
            | (Self::VerificationPassed, EvidenceKind::BuildPassed { .. })
            | (Self::VerificationPassed, EvidenceKind::LintPassed { .. }) => true,
            (
                Self::CommandOutcome {
                    command_contains,
                    expectation,
                },
                EvidenceKind::CommandSucceeded { command, exit_code },
            ) => {
                if expectation.is_reproduction()
                    || matches!(expectation, CommandExpectation::ExpectedFailure)
                {
                    false
                } else if command_contains.is_empty() {
                    expectation.evaluate(Some(*exit_code), command)
                } else {
                    command.contains(command_contains.as_str())
                        && expectation.evaluate(Some(*exit_code), command)
                }
            }
            // Bare failure satisfies ExpectedFailure only — not Reproduction.
            (
                Self::CommandOutcome {
                    command_contains,
                    expectation: CommandExpectation::ExpectedFailure,
                },
                EvidenceKind::CommandFailed { command, .. },
            ) => command_contains.is_empty() || command.contains(command_contains.as_str()),
            // Fingerprint-matched reproduction evidence.
            (
                Self::CommandOutcome {
                    command_contains,
                    expectation: CommandExpectation::Reproduction(_),
                },
                EvidenceKind::ReproductionSucceeded { command, .. },
            ) => command_contains.is_empty() || command.contains(command_contains.as_str()),
            // Wrong expectation class never matches.
            (Self::CommandOutcome { .. }, EvidenceKind::CommandFailed { .. }) => false,
            (Self::CommandOutcome { .. }, EvidenceKind::ReproductionSucceeded { .. }) => false,
            // SearchHit: only real search observations. Empty `query_contains`
            // is NOT a wildcard over FileRead / CommandSucceeded — those were
            // the false-completion holes for root-cause evidence.
            (Self::SearchHit { query_contains }, EvidenceKind::SearchHit { query, hits }) => {
                *hits > 0 && (query_contains.is_empty() || query.contains(query_contains.as_str()))
            }
            // Explicit inspection (read/locate subtasks): file read or search.
            (Self::FileInspected { path_contains }, EvidenceKind::FileRead { path }) => {
                path_contains.is_empty() || path.contains(path_contains.as_str())
            }
            (Self::FileInspected { path_contains }, EvidenceKind::SearchHit { hits, .. }) => {
                *hits > 0 && path_contains.is_empty()
            }
            // Semantic criteria — strict structural kinds only, no wildcards.
            (Self::SemanticProof { target }, kind) => match (target, kind) {
                (SemanticTarget::RootCause, EvidenceKind::SearchHit { query, hits }) => {
                    *hits > 0 && !query.trim().is_empty()
                }
                (SemanticTarget::Reproduction, EvidenceKind::ReproductionSucceeded { .. }) => true,
                (
                    SemanticTarget::BehaviorImplemented,
                    EvidenceKind::FileChanged { .. } | EvidenceKind::PatchApplied { .. },
                ) => true,
                (
                    SemanticTarget::RegressionPrevented,
                    EvidenceKind::TestPassed { .. }
                    | EvidenceKind::BuildPassed { .. }
                    | EvidenceKind::LintPassed { .. },
                ) => true,
                _ => false,
            },
            // Unresolved free-form: only criterion-bound structural evidence
            // passes; bare unbound tool success is never enough.
            (Self::RequiresExplicitEvidence, kind) => {
                !is_context_obs
                    && !matches!(kind, EvidenceKind::ContextObservation { .. })
                    && item.criterion_id.is_some()
            }
            (Self::ToolSucceeded { tool }, kind) => match kind {
                EvidenceKind::CommandSucceeded { command, .. } => {
                    command == tool || command.contains(tool.as_str())
                }
                EvidenceKind::PatchApplied { .. } => {
                    tool == "apply_patch" || tool == "replace_range" || tool == "patch"
                }
                EvidenceKind::FileChanged { .. } => {
                    tool == "write_file" || tool == "create_file" || tool.contains("write")
                }
                EvidenceKind::TestPassed { command } => command == tool || tool == "run_command",
                EvidenceKind::SearchHit { query, .. } => tool == "search" || query == tool,
                EvidenceKind::FileRead { path } => tool == "read_file" || path == tool,
                _ => false,
            },
            (Self::UserApproval, EvidenceKind::UserApproval { .. }) => true,
            (Self::DiffReviewed { path_contains }, EvidenceKind::DiffReviewed { paths }) => {
                paths.iter().any(|p| p.contains(path_contains.as_str()))
            }
            // Code-review flows may complete diff-review via real file reads
            // of changed targets when no DiffReviewed item was recorded.
            (Self::DiffReviewed { path_contains }, EvidenceKind::FileRead { path }) => {
                path_contains.is_empty() || path.contains(path_contains.as_str())
            }
            (Self::DiffReviewed { .. }, EvidenceKind::SearchHit { hits, .. }) => *hits > 0,
            (Self::ContextRead { path_contains }, EvidenceKind::ContextObservation { path }) => {
                path.contains(path_contains.as_str())
            }
            (Self::ContextRead { path_contains }, EvidenceKind::FileRead { path }) => {
                path_contains.is_empty() || path.contains(path_contains.as_str())
            }
            (Self::ContextRead { path_contains }, EvidenceKind::SearchHit { .. }) => {
                path_contains.is_empty()
            }
            (Self::ContextRead { .. }, EvidenceKind::CommandSucceeded { command, .. }) => {
                command.contains("git status") || command.contains("pre_search")
            }
            _ => false,
        }
    }
}

/// Free-form criterion attached to a plan, with a structural matcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub id: String,
    pub description: String,
    pub requirement: EvidenceRequirement,
    pub evidence: Vec<EvidenceItem>,
    pub status: CriterionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CriterionStatus {
    Pending,
    Met,
    Unmet,
}

impl AcceptanceCriterion {
    pub fn new(
        id: impl Into<String>,
        description: impl Into<String>,
        requirement: EvidenceRequirement,
    ) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            requirement,
            evidence: Vec::new(),
            status: CriterionStatus::Pending,
        }
    }

    /// Binding + matcher gate. Evidence bound to another criterion never
    /// satisfies this one unless explicitly reusable; unresolved requirements
    /// only accept already-bound evidence for this id.
    pub fn accepts(&self, item: &EvidenceItem) -> bool {
        if !item.binding_allows(&self.id) {
            return false;
        }
        if self.requirement.requires_explicit_binding() {
            let bound_ok = match item.criterion_id.as_deref() {
                Some(bound) => bound == REUSABLE_CRITERION || bound == self.id,
                None => false,
            };
            if !bound_ok {
                return false;
            }
        }
        self.requirement.matches(item)
    }

    pub fn absorb(&mut self, item: &EvidenceItem) -> bool {
        if self.accepts(item) {
            let mut bound = item.clone();
            if bound.criterion_id.is_none() {
                bound.criterion_id = Some(self.id.clone());
            }
            if !self.evidence.iter().any(|e| e.id == bound.id) {
                self.evidence.push(bound);
            }
            self.status = CriterionStatus::Met;
            return true;
        }
        false
    }

    pub fn is_met(&self) -> bool {
        self.status == CriterionStatus::Met && !self.evidence.is_empty()
    }

    /// Free-form criterion that never received explicit bound evidence.
    pub fn is_unresolved(&self) -> bool {
        !self.is_met() && self.requirement.is_unresolved()
    }
}

/// Requirement attached to a subtask so arbitrary Read cannot complete it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtaskRequirement {
    pub evidence: EvidenceRequirement,
    /// Context/pre-scan observations may only satisfy this if true.
    /// Default false: pre-scan is context, not proof of "criteria extracted".
    pub allow_context_as_evidence: bool,
}

impl SubtaskRequirement {
    pub fn strict(evidence: EvidenceRequirement) -> Self {
        Self {
            evidence,
            allow_context_as_evidence: false,
        }
    }

    pub fn contextual(evidence: EvidenceRequirement) -> Self {
        Self {
            evidence,
            allow_context_as_evidence: true,
        }
    }

    pub fn any_observation() -> Self {
        Self::contextual(EvidenceRequirement::AnyToolSuccess)
    }

    pub fn matches(&self, item: &EvidenceItem) -> bool {
        if matches!(item.kind, EvidenceKind::ContextObservation { .. })
            && !self.allow_context_as_evidence
        {
            return false;
        }
        self.evidence.matches(item)
    }
}

/// Parse `exit=N` from a failed run_command error message (or default 1).
fn parse_exit_code(result: &ToolResult) -> Option<i32> {
    result
        .error
        .as_ref()
        .and_then(|e| {
            e.message.lines().find_map(|l| {
                l.strip_prefix("exit=")
                    .and_then(|v| v.trim().parse::<i32>().ok())
                    .or_else(|| {
                        e.message.rsplit("exit=").next().and_then(|tail| {
                            tail.chars()
                                .take_while(|c| c.is_ascii_digit() || *c == '-')
                                .collect::<String>()
                                .parse::<i32>()
                                .ok()
                        })
                    })
            })
        })
        .or(Some(1))
}

/// Infer a typed evidence item from a tool result + command expectation.
pub fn evidence_from_tool_result(
    result: &ToolResult,
    expectation: Option<&CommandExpectation>,
) -> Option<EvidenceItem> {
    let tool = ToolName::parse(&result.name)?;
    let id = format!("ev_{}", result.id);
    let source = result.name.clone();
    // Pre-scan / context manager observations are context, not proof.
    if result.id.as_str().starts_with("ctx_")
        || result.id.as_str().starts_with("pre_")
        || result.id.as_str().starts_with("pin_")
    {
        return Some(EvidenceItem::new(
            id,
            "context",
            EvidenceKind::ContextObservation {
                path: result.input.clone(),
            },
        ));
    }
    let kind = match tool {
        ToolName::Search => {
            // "N 个文件匹配" / "N matches"
            let hits = result
                .output
                .lines()
                .next()
                .and_then(|line| {
                    line.split_whitespace()
                        .find(|tok| tok.chars().all(|c| c.is_ascii_digit()))
                        .and_then(|n| n.parse::<usize>().ok())
                })
                .unwrap_or(if result.ok { 1 } else { 0 });
            if !result.ok || hits == 0 {
                return None;
            }
            EvidenceKind::SearchHit {
                query: result.input.clone(),
                hits,
            }
        }
        ToolName::ReadFile => {
            if !result.ok {
                return None;
            }
            EvidenceKind::FileRead {
                path: result.input.clone(),
            }
        }
        ToolName::WriteFile | ToolName::CreateFile => {
            if !result.ok {
                return None;
            }
            EvidenceKind::FileChanged {
                path: result.input.clone(),
            }
        }
        ToolName::ApplyPatch | ToolName::ReplaceRange => {
            if !result.ok {
                return None;
            }
            EvidenceKind::PatchApplied {
                path: result.input.clone(),
            }
        }
        ToolName::DeleteFile => {
            if !result.ok {
                return None;
            }
            EvidenceKind::FileChanged {
                path: result.input.clone(),
            }
        }
        ToolName::RunCommand => {
            let command = result.input.clone();
            if result.ok {
                let lower = command.to_ascii_lowercase();
                if lower.contains("test") || lower.contains("pytest") {
                    EvidenceKind::TestPassed {
                        command: command.clone(),
                    }
                } else if lower.contains("cargo check")
                    || lower.contains("tsc")
                    || lower.contains("build")
                {
                    EvidenceKind::BuildPassed {
                        command: command.clone(),
                    }
                } else if lower.contains("lint") || lower.contains("clippy") {
                    EvidenceKind::LintPassed {
                        command: command.clone(),
                    }
                } else {
                    EvidenceKind::CommandSucceeded {
                        command: command.clone(),
                        exit_code: 0,
                    }
                }
            } else {
                let exit_code = parse_exit_code(result);
                let err_text = result
                    .error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("");
                let stdout = result.output.as_str();
                match expectation.unwrap_or(&CommandExpectation::ExpectedSuccess) {
                    // Successful reproduction: fingerprint must match.
                    CommandExpectation::Reproduction(fe) => {
                        let concrete =
                            if fe.capture_on_first_failure && !fe.has_specific_fingerprint() {
                                // None → not an application-level failure; stay unresolved.
                                FailureExpectation::mint_from_observation(
                                    exit_code, stdout, err_text, &command,
                                )?
                            } else {
                                fe.clone()
                            };
                        if !concrete.evaluate(exit_code, stdout, err_text) {
                            // Wrong signature — stay unresolved, never "reproduced".
                            return None;
                        }
                        EvidenceKind::ReproductionSucceeded {
                            command: command.clone(),
                            exit_code,
                            expectation: concrete,
                        }
                    }
                    // Bare non-zero: failure only, never reproduction.
                    CommandExpectation::ExpectedFailure => {
                        if !matches!(exit_code, Some(c) if c != 0) {
                            return None;
                        }
                        EvidenceKind::CommandFailed {
                            command: command.clone(),
                            exit_code,
                        }
                    }
                    // Unexpected failure under a success/output expectation.
                    _ => return None,
                }
            }
        }
        ToolName::ListFiles | ToolName::FindSymbol | ToolName::FindReferences => {
            if !result.ok {
                return None;
            }
            EvidenceKind::SearchHit {
                query: result.input.clone(),
                hits: result
                    .output
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .count()
                    .max(1),
            }
        }
        ToolName::ReadRange => {
            if !result.ok {
                return None;
            }
            EvidenceKind::FileRead {
                path: result.input.clone(),
            }
        }
    };
    Some(EvidenceItem::new(id, source, kind))
}

/// Build a contextual pre-scan evidence item (never proves extraction/root-cause).
pub fn context_observation(path: impl Into<String>, id: impl Into<String>) -> EvidenceItem {
    EvidenceItem::new(
        id,
        "context",
        EvidenceKind::ContextObservation { path: path.into() },
    )
}

/// Accumulator for a turn's evidence bag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EvidenceBag {
    pub items: Vec<EvidenceItem>,
    pub verify_ok: Option<bool>,
    /// Commands that passed project verification (never mixed with repro).
    pub verify_commands: Vec<String>,
    /// Commands that successfully reproduced the target bug.
    pub repro_commands: Vec<String>,
    /// Active reproduction fingerprints (cleared when the command passes).
    pub reproduction: Vec<ReproductionRecord>,
    /// Set when a recorded reproduction fingerprint reappears after the fix.
    pub regression_failed: bool,
    pub model_claimed_done: bool,
    pub denied: Vec<String>,
    pub files_written: Vec<String>,
}

/// One successful reproduction, kept so a later reappearance fails regression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReproductionRecord {
    pub command: String,
    pub expectation: FailureExpectation,
}

impl EvidenceBag {
    pub fn push(&mut self, item: EvidenceItem) {
        if !self.items.iter().any(|e| e.id == item.id) {
            self.items.push(item);
        }
    }

    pub fn absorb_tool_result(
        &mut self,
        result: &ToolResult,
        expectation: Option<&CommandExpectation>,
    ) {
        let is_run = ToolName::parse(&result.name) == Some(ToolName::RunCommand);
        if result.ok {
            if ToolName::parse(&result.name)
                .map(|n| n.is_mutation())
                .unwrap_or(false)
            {
                self.files_written.push(result.input.clone());
            }
            if is_run {
                // Reproduced command now passes — fixed; drop its fingerprint.
                let had = self.reproduction.iter().any(|r| r.command == result.input);
                self.reproduction.retain(|r| r.command != result.input);
                if had && self.reproduction.is_empty() {
                    self.regression_failed = false;
                }
            }
        } else if result
            .error
            .as_ref()
            .map(|e| e.code == ToolErrorCode::PermissionDenied)
            .unwrap_or(false)
        {
            self.denied
                .push(format!("{}: {}", result.name, result.input));
        }

        // After a fix has started, the same fingerprint reappearing is a
        // regression and must fail verification (not "reproduced again").
        if !result.ok && is_run && !self.files_written.is_empty() {
            if let Some(rec) = self.reproduction.iter().find(|r| r.command == result.input) {
                let exit = parse_exit_code(result);
                let err = result
                    .error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("");
                if rec.expectation.evaluate(exit, &result.output, err) {
                    self.regression_failed = true;
                }
            }
        }

        if let Some(item) = evidence_from_tool_result(result, expectation) {
            if let EvidenceKind::ReproductionSucceeded {
                command,
                expectation: fe,
                ..
            } = &item.kind
            {
                self.reproduction.retain(|r| r.command != *command);
                self.reproduction.push(ReproductionRecord {
                    command: command.clone(),
                    expectation: fe.clone(),
                });
                if !self.repro_commands.contains(command) {
                    self.repro_commands.push(command.clone());
                }
            }
            self.push(item);
        }
    }

    pub fn mark_verify(&mut self, ok: bool, commands: Vec<String>) {
        // A reappeared reproduction fingerprint always fails regression verify.
        let ok = ok && !self.regression_failed;
        self.verify_ok = Some(ok);
        if ok {
            self.verify_commands = commands.clone();
            for command in commands {
                let lower = command.to_ascii_lowercase();
                let kind = if lower.contains("test") {
                    EvidenceKind::TestPassed {
                        command: command.clone(),
                    }
                } else if lower.contains("lint") || lower.contains("clippy") {
                    EvidenceKind::LintPassed {
                        command: command.clone(),
                    }
                } else {
                    EvidenceKind::BuildPassed {
                        command: command.clone(),
                    }
                };
                self.push(EvidenceItem::new(
                    format!("verify_{command}"),
                    "verify",
                    kind,
                ));
            }
        }
    }

    /// Does any observation satisfy `requirement` (no criterion binding)?
    pub fn satisfies(&self, requirement: &EvidenceRequirement) -> bool {
        self.satisfies_for(None, requirement)
    }

    /// Binding-aware satisfaction: when `criterion_id` is set, evidence bound
    /// to a different criterion never counts; unresolved requirements need
    /// evidence bound to this id (or [`REUSABLE_CRITERION`]).
    pub fn satisfies_for(
        &self,
        criterion_id: Option<&str>,
        requirement: &EvidenceRequirement,
    ) -> bool {
        if matches!(requirement, EvidenceRequirement::VerificationPassed)
            && self.verify_ok == Some(true)
        {
            return true;
        }
        if matches!(
            requirement,
            EvidenceRequirement::SemanticProof {
                target: SemanticTarget::RegressionPrevented
            }
        ) && self.verify_ok == Some(true)
        {
            return true;
        }
        self.items.iter().any(|item| match criterion_id {
            None => requirement.matches(item),
            Some(cid) => {
                if !item.binding_allows(cid) {
                    return false;
                }
                if requirement.requires_explicit_binding() {
                    let bound_ok = match item.criterion_id.as_deref() {
                        Some(bound) => bound == REUSABLE_CRITERION || bound == cid,
                        None => false,
                    };
                    bound_ok && requirement.matches(item)
                } else {
                    requirement.matches(item)
                }
            }
        })
    }
}

/// Default requirement heuristic from free-text criterion / subtask title.
///
/// Unknown free-form text never degrades to [`EvidenceRequirement::AnyToolSuccess`]
/// — it becomes [`EvidenceRequirement::RequiresExplicitEvidence`] (fail closed).
pub fn infer_requirement(description: &str) -> EvidenceRequirement {
    let lower = description.to_ascii_lowercase();
    if lower.contains("reproduc") || lower.contains("复现") {
        return EvidenceRequirement::SemanticProof {
            target: SemanticTarget::Reproduction,
        };
    }
    if lower.contains("verif")
        || lower.contains("test")
        || lower.contains("验证")
        || lower.contains("测试")
    {
        return EvidenceRequirement::VerificationPassed;
    }
    if lower.contains("regression") {
        return EvidenceRequirement::SemanticProof {
            target: SemanticTarget::RegressionPrevented,
        };
    }
    if lower.contains("writ")
        || lower.contains("edit")
        || lower.contains("修复")
        || lower.contains("修改")
    {
        return EvidenceRequirement::SemanticProof {
            target: SemanticTarget::BehaviorImplemented,
        };
    }
    if lower.contains("criteria") || lower.contains("acceptance") || lower.contains("验收") {
        return EvidenceRequirement::ToolSucceeded {
            tool: "extract_criteria".into(),
        };
    }
    if lower.contains("locate") || lower.contains("root cause") || lower.contains("定位") {
        return EvidenceRequirement::SemanticProof {
            target: SemanticTarget::RootCause,
        };
    }
    if lower.contains("context") || lower.contains("gather") || lower.contains("上下文") {
        return EvidenceRequirement::AnyToolSuccess;
    }
    EvidenceRequirement::RequiresExplicitEvidence
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ToolCallId, ToolError};

    fn cmd_ok(command: &str) -> ToolResult {
        ToolResult::success(ToolCallId::new("c1"), "run_command", command, "ok")
    }

    fn cmd_fail(command: &str, message: &str) -> ToolResult {
        ToolResult::failure(
            ToolCallId::new("c2"),
            "run_command",
            command,
            ToolError::execution(message),
        )
    }

    #[test]
    fn git_status_cannot_prove_bug_reproduction() {
        let requirement = EvidenceRequirement::CommandOutcome {
            command_contains: "cargo test".into(),
            expectation: CommandExpectation::Reproduction(
                FailureExpectation::default().with_output("assertion failed"),
            ),
        };
        let bag_ok = {
            let mut bag = EvidenceBag::default();
            bag.absorb_tool_result(
                &cmd_ok("git status --short"),
                Some(&CommandExpectation::ExpectedSuccess),
            );
            bag
        };
        assert!(!bag_ok.satisfies(&requirement));

        // Generic success is also not reproduction.
        let expectation = CommandExpectation::Reproduction(
            FailureExpectation::default().with_output("assertion failed"),
        );
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(&cmd_ok("cargo test"), Some(&expectation));
        // ok command + expected failure → no ReproductionSucceeded evidence
        assert!(!bag.satisfies(&requirement), "bag={:?}", bag.items);
    }

    #[test]
    fn failing_reproduction_can_be_successful_evidence() {
        let expectation = CommandExpectation::Reproduction(
            FailureExpectation::default().with_output("assertion failed"),
        );
        let requirement = EvidenceRequirement::CommandOutcome {
            command_contains: "cargo test".into(),
            expectation: expectation.clone(),
        };
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_fail(
                "cargo test auth",
                "test auth ... assertion failed left=1 right=0",
            ),
            Some(&expectation),
        );
        assert!(bag.satisfies(&requirement), "bag={:?}", bag.items);
        assert!(bag
            .items
            .iter()
            .any(|i| matches!(i.kind, EvidenceKind::ReproductionSucceeded { .. })));
        assert!(bag.repro_commands.iter().any(|c| c.contains("cargo test")));
        assert!(bag.verify_commands.is_empty(), "repro ≠ verify");
    }

    #[test]
    fn arbitrary_read_cannot_satisfy_acceptance_criteria_identified() {
        let mut criterion = AcceptanceCriterion::new(
            "c1",
            "Acceptance criteria identified",
            EvidenceRequirement::ToolSucceeded {
                tool: "extract_criteria".into(),
            },
        );
        let read = EvidenceItem::new(
            "ev_read",
            "read_file",
            EvidenceKind::FileRead {
                path: "src/auth.rs".into(),
            },
        );
        assert!(!criterion.absorb(&read));
        assert!(!criterion.is_met());

        let extract = EvidenceItem::new(
            "ev_extract",
            "extract_criteria",
            EvidenceKind::CommandSucceeded {
                command: "extract_criteria".into(),
                exit_code: 0,
            },
        );
        // ToolSucceeded matches command == tool name
        assert!(criterion.absorb(&extract) || !criterion.requirement.matches(&extract));
        // Explicitly: FileRead never matches ToolSucceeded{tool:"extract_criteria"}
        assert!(!criterion.requirement.matches(&read));
    }

    #[test]
    fn context_observation_is_not_strict_subtask_proof() {
        let strict = SubtaskRequirement::strict(EvidenceRequirement::ToolSucceeded {
            tool: "search".into(),
        });
        let ctx = context_observation("src/auth.rs", "ctx_1");
        assert!(!strict.matches(&ctx));

        let hit = EvidenceItem::new(
            "ev_hit",
            "search",
            EvidenceKind::SearchHit {
                query: "auth".into(),
                hits: 2,
            },
        );
        let strict_search = SubtaskRequirement::strict(EvidenceRequirement::SearchHit {
            query_contains: "auth".into(),
        });
        assert!(strict_search.matches(&hit));
        // Context still blocked
        assert!(!strict_search.matches(&ctx));
    }

    #[test]
    fn model_done_without_criterion_evidence_cannot_finish() {
        let criterion = AcceptanceCriterion::new(
            "c2",
            "Verification commands pass",
            EvidenceRequirement::VerificationPassed,
        );
        let mut bag = EvidenceBag {
            model_claimed_done: true,
            ..Default::default()
        };
        bag.absorb_tool_result(
            &cmd_ok("git status --short"),
            Some(&CommandExpectation::ExpectedSuccess),
        );
        assert!(!bag.satisfies(&criterion.requirement));
        assert!(!criterion.is_met());
        // Model prose never creates EvidenceItem in production; simulate empty bag:
        let empty = EvidenceBag::default();
        assert!(!empty.satisfies(&criterion.requirement));
        // A claim flag alone is never structural proof.
        assert!(bag.model_claimed_done);
        assert!(!empty.model_claimed_done || !empty.satisfies(&criterion.requirement));
    }

    #[test]
    fn dirty_worktree_user_changes_are_not_kodo_evidence() {
        // FileChanged evidence is only created from successful mutation tools,
        // not from git status listing pre-existing dirt.
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_ok("git status --short"),
            Some(&CommandExpectation::ExpectedSuccess),
        );
        assert!(bag.files_written.is_empty());
        assert!(!bag.satisfies(&EvidenceRequirement::AnyFileChange));
        assert!(!bag.satisfies(&EvidenceRequirement::FileChanged {
            path_contains: "src/".into()
        }));
    }

    #[test]
    fn failure_without_matching_signature_is_not_reproduction() {
        let expectation = CommandExpectation::Reproduction(
            FailureExpectation::default().with_output("NullPointerException"),
        );
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_fail("npm test", "FAIL empty suite"),
            Some(&expectation),
        );
        let requirement = EvidenceRequirement::CommandOutcome {
            command_contains: "npm test".into(),
            expectation,
        };
        assert!(!bag.satisfies(&requirement));
        assert!(bag
            .items
            .iter()
            .all(|i| !matches!(i.kind, EvidenceKind::ReproductionSucceeded { .. })));
    }

    // -----------------------------------------------------------------------
    // semantic_completion regressions — false-completion holes must stay shut
    // -----------------------------------------------------------------------

    fn root_cause_requirement() -> EvidenceRequirement {
        EvidenceRequirement::SemanticProof {
            target: SemanticTarget::RootCause,
        }
    }

    #[test]
    fn semantic_completion_arbitrary_read_cannot_prove_root_cause() {
        let mut criterion =
            AcceptanceCriterion::new("rc", "root cause located", root_cause_requirement());
        let read = EvidenceItem::new(
            "ev_read",
            "read_file",
            EvidenceKind::FileRead {
                path: "src/auth.rs".into(),
            },
        );
        // Even the legacy SearchHit{""} wildcard must not accept FileRead.
        let legacy = EvidenceRequirement::SearchHit {
            query_contains: String::new(),
        };
        assert!(!legacy.matches(&read));
        assert!(!criterion.absorb(&read));
        assert!(!criterion.is_met());

        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &ToolResult::success(
                ToolCallId::new("r"),
                "read_file",
                "src/auth.rs",
                "fn auth() {}",
            ),
            None,
        );
        assert!(!bag.satisfies(&root_cause_requirement()));
        assert!(!bag.satisfies_for(Some("rc"), &root_cause_requirement()));
    }

    #[test]
    fn semantic_completion_git_status_cannot_prove_root_cause() {
        let mut criterion =
            AcceptanceCriterion::new("rc", "root cause located", root_cause_requirement());
        let git = EvidenceItem::new(
            "ev_git",
            "run_command",
            EvidenceKind::CommandSucceeded {
                command: "git status --short".into(),
                exit_code: 0,
            },
        );
        assert!(!criterion.absorb(&git));
        assert!(!criterion.is_met());

        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_ok("git status --short"),
            Some(&CommandExpectation::ExpectedSuccess),
        );
        assert!(!bag.satisfies(&root_cause_requirement()));
        // Empty SearchHit wildcard must not accept the command either.
        let legacy = EvidenceRequirement::SearchHit {
            query_contains: String::new(),
        };
        assert!(!legacy.matches(&git));
        for item in &bag.items {
            assert!(!legacy.matches(item), "legacy matched {:?}", item.kind);
        }
    }

    #[test]
    fn semantic_completion_generic_success_command_cannot_prove_root_cause() {
        let mut criterion =
            AcceptanceCriterion::new("rc", "root cause located", root_cause_requirement());
        let ok = EvidenceItem::new(
            "ev_cmd",
            "run_command",
            EvidenceKind::CommandSucceeded {
                command: "cargo check".into(),
                exit_code: 0,
            },
        );
        assert!(!criterion.absorb(&ok));
        assert!(!criterion.is_met());

        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_ok("cargo check"),
            Some(&CommandExpectation::ExpectedSuccess),
        );
        assert!(!bag.satisfies(&root_cause_requirement()));
        let legacy = EvidenceRequirement::SearchHit {
            query_contains: String::new(),
        };
        assert!(!legacy.matches(&ok));

        // A real search hit is the structural proof.
        let hit = EvidenceItem::new(
            "ev_search",
            "search",
            EvidenceKind::SearchHit {
                query: "root_cause_site".into(),
                hits: 3,
            },
        );
        let mut criterion =
            AcceptanceCriterion::new("rc", "root cause located", root_cause_requirement());
        assert!(criterion.absorb(&hit));
        assert!(criterion.is_met());
        // Bound provenance recorded on the stored evidence.
        assert_eq!(criterion.evidence[0].criterion_id.as_deref(), Some("rc"));
    }

    #[test]
    fn semantic_completion_unknown_freeform_criterion_not_any_tool_success() {
        let unknown = "Ship it with the usual polish";
        let req = infer_requirement(unknown);
        assert!(
            matches!(req, EvidenceRequirement::RequiresExplicitEvidence),
            "unknown free-form must not degrade, got {:?}",
            req
        );
        assert!(!matches!(req, EvidenceRequirement::AnyToolSuccess));

        let mut criterion = AcceptanceCriterion::new("m_c1", unknown, req.clone());
        // Unbound generic tool success cannot finish an unresolved criterion.
        let ok = EvidenceItem::new(
            "ev_cmd",
            "run_command",
            EvidenceKind::CommandSucceeded {
                command: "git status --short".into(),
                exit_code: 0,
            },
        );
        assert!(!criterion.absorb(&ok));
        assert!(!criterion.is_met());
        assert!(criterion.is_unresolved());

        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_ok("git status --short"),
            Some(&CommandExpectation::ExpectedSuccess),
        );
        assert!(!bag.satisfies(&req));
        assert!(!bag.satisfies_for(Some("m_c1"), &req));

        // Explicit bound evidence can complete it.
        let bound = EvidenceItem::new(
            "ev_bound",
            "extract_criteria",
            EvidenceKind::CommandSucceeded {
                command: "extract_criteria".into(),
                exit_code: 0,
            },
        )
        .bound_to("m_c1");
        assert!(criterion.absorb(&bound));
        assert!(criterion.is_met());
        assert!(!criterion.is_unresolved());
    }

    #[test]
    fn semantic_completion_bound_evidence_cannot_satisfy_other_criterion() {
        let mut c1 = AcceptanceCriterion::new(
            "c1",
            "verification passes",
            EvidenceRequirement::VerificationPassed,
        );
        let mut c2 = AcceptanceCriterion::new(
            "c2",
            "regression prevented",
            EvidenceRequirement::SemanticProof {
                target: SemanticTarget::RegressionPrevented,
            },
        );
        let bound_to_c1 = EvidenceItem::new(
            "ev_t",
            "verify",
            EvidenceKind::TestPassed {
                command: "cargo test".into(),
            },
        )
        .bound_to("c1");

        assert!(c1.absorb(&bound_to_c1));
        assert!(c1.is_met());
        assert!(
            !c2.accepts(&bound_to_c1),
            "evidence bound to c1 must not satisfy c2"
        );
        assert!(!c2.absorb(&bound_to_c1));
        assert!(!c2.is_met());

        // Explicitly reusable evidence may satisfy both.
        let reusable = EvidenceItem::new(
            "ev_r",
            "verify",
            EvidenceKind::TestPassed {
                command: "cargo test".into(),
            },
        )
        .reusable();
        assert!(c1.accepts(&reusable));
        assert!(c2.accepts(&reusable));

        // satisfies_for also enforces binding.
        let mut bag = EvidenceBag::default();
        bag.push(bound_to_c1.clone());
        assert!(bag.satisfies_for(Some("c1"), &c1.requirement));
        assert!(
            !bag.satisfies_for(Some("c2"), &c2.requirement),
            "bound evidence must not leak across criteria"
        );
    }

    #[test]
    fn semantic_completion_reproduction_and_regression_semantic_kinds() {
        // Reproduction: only fingerprint-matched ReproductionSucceeded.
        let repro = EvidenceRequirement::SemanticProof {
            target: SemanticTarget::Reproduction,
        };
        let expectation = CommandExpectation::Reproduction(
            FailureExpectation::default().with_output("assertion failed"),
        );
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(&cmd_ok("git status --short"), Some(&expectation));
        assert!(!bag.satisfies(&repro));
        bag.absorb_tool_result(
            &cmd_fail("cargo test auth", "assertion failed left=1"),
            Some(&expectation),
        );
        assert!(bag.satisfies(&repro));

        // Behavior implemented: file change only.
        let behavior = EvidenceRequirement::SemanticProof {
            target: SemanticTarget::BehaviorImplemented,
        };
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &ToolResult::success(ToolCallId::new("r"), "read_file", "src/a.rs", "…"),
            None,
        );
        assert!(!bag.satisfies(&behavior));
        bag.absorb_tool_result(
            &ToolResult::success(ToolCallId::new("w"), "write_file", "src/a.rs", "wrote"),
            None,
        );
        assert!(bag.satisfies(&behavior));
    }

    // -----------------------------------------------------------------------
    // reproduction — "failed" vs "successfully reproduced target bug"
    // -----------------------------------------------------------------------

    #[test]
    fn reproduction_unrelated_failing_test_cannot_prove() {
        let expectation = CommandExpectation::Reproduction(FailureExpectation::with_test_name(
            "test_root_cause_panic",
        ));
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_fail(
                "cargo test",
                "test unrelated_helper ... FAILED\nfailures: unrelated_helper",
            ),
            Some(&expectation),
        );
        assert!(
            !bag.items
                .iter()
                .any(|i| matches!(i.kind, EvidenceKind::ReproductionSucceeded { .. })),
            "unrelated failing test must not reproduce the target"
        );
        assert!(!bag.satisfies(&EvidenceRequirement::SemanticProof {
            target: SemanticTarget::Reproduction
        }));
        assert!(bag.repro_commands.is_empty());
    }

    #[test]
    fn reproduction_command_not_found_cannot_prove_application_bug() {
        // Capture mode must refuse exit 127 / "command not found".
        let expectation = CommandExpectation::Reproduction(FailureExpectation::capture());
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &ToolResult::failure(
                ToolCallId::new("nf"),
                "run_command",
                "my-repro-binary --bug",
                ToolError::execution("my-repro-binary: command not found\nexit=127"),
            ),
            Some(&expectation),
        );
        assert!(bag
            .items
            .iter()
            .all(|i| !matches!(i.kind, EvidenceKind::ReproductionSucceeded { .. })));

        // Explicit wrong-exit fingerprint also rejects 127.
        let explicit = CommandExpectation::Reproduction(
            FailureExpectation::with_exit(101).with_output("panic"),
        );
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &ToolResult::failure(
                ToolCallId::new("nf2"),
                "run_command",
                "cargo test repro",
                ToolError::execution("bash: cargo: command not found\nexit=127"),
            ),
            Some(&explicit),
        );
        assert!(bag
            .items
            .iter()
            .all(|i| !matches!(i.kind, EvidenceKind::ReproductionSucceeded { .. })));
    }

    #[test]
    fn reproduction_auth_network_failure_cannot_prove_target_regression() {
        // Target is a named test panic — auth/network noise must not match.
        let expectation = CommandExpectation::Reproduction(
            FailureExpectation::with_test_name("test_login_panic").with_output("panicked at"),
        );
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_fail(
                "npm test -- auth",
                "HTTP 401 Unauthorized\nECONNREFUSED 127.0.0.1",
            ),
            Some(&expectation),
        );
        assert!(bag
            .items
            .iter()
            .all(|i| !matches!(i.kind, EvidenceKind::ReproductionSucceeded { .. })));

        // Capture mode also refuses pure auth failures without app-level markers.
        let capture = CommandExpectation::Reproduction(FailureExpectation::capture());
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_fail("curl -s /login", "HTTP/1.1 401 Unauthorized"),
            Some(&capture),
        );
        assert!(bag
            .items
            .iter()
            .all(|i| !matches!(i.kind, EvidenceKind::ReproductionSucceeded { .. })));
    }

    #[test]
    fn reproduction_matching_named_failing_test_can_prove() {
        let expectation = CommandExpectation::Reproduction(FailureExpectation::with_test_name(
            "test_login_panic",
        ));
        let requirement = EvidenceRequirement::SemanticProof {
            target: SemanticTarget::Reproduction,
        };
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_fail(
                "cargo test login",
                "test auth::test_login_panic ... FAILED\npanicked at src/auth.rs:10",
            ),
            Some(&expectation),
        );
        assert!(
            bag.items
                .iter()
                .any(|i| matches!(i.kind, EvidenceKind::ReproductionSucceeded { .. })),
            "bag={:?}",
            bag.items
        );
        assert!(bag.satisfies(&requirement));
        assert!(!bag.repro_commands.is_empty());
        assert!(bag.verify_commands.is_empty());

        let mut criterion = AcceptanceCriterion::new(
            "skill_c1",
            "The failure was reproduced before the fix",
            requirement.clone(),
        );
        let item = bag
            .items
            .iter()
            .find(|i| matches!(i.kind, EvidenceKind::ReproductionSucceeded { .. }))
            .expect("repro item")
            .clone();
        assert!(criterion.absorb(&item));
        assert!(criterion.is_met());
    }

    #[test]
    fn reproduction_after_fix_same_regression_command_must_pass() {
        let fe = FailureExpectation::default().with_output("assertion failed");
        let expectation = CommandExpectation::Reproduction(fe.clone());
        let mut bag = EvidenceBag::default();

        // 1) First failure reproduces the bug.
        bag.absorb_tool_result(
            &cmd_fail("cargo test auth", "assertion failed left=1"),
            Some(&expectation),
        );
        assert_eq!(bag.repro_commands, vec!["cargo test auth".to_owned()]);
        assert!(!bag.regression_failed);

        // 2) Fix starts (file written), same fingerprint reappears → regression.
        bag.absorb_tool_result(
            &ToolResult::success(ToolCallId::new("w"), "write_file", "src/auth.rs", "wrote"),
            None,
        );
        bag.absorb_tool_result(
            &cmd_fail("cargo test auth", "assertion failed left=1"),
            Some(&expectation),
        );
        assert!(
            bag.regression_failed,
            "same fingerprint after fix must fail regression"
        );
        bag.mark_verify(true, vec!["cargo test auth".into()]);
        assert_eq!(
            bag.verify_ok,
            Some(false),
            "regression_failed forces verify failure"
        );

        // 3) After a real fix the same command passes → regression cleared.
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_fail("cargo test auth", "assertion failed left=1"),
            Some(&expectation),
        );
        bag.absorb_tool_result(
            &ToolResult::success(ToolCallId::new("w2"), "write_file", "src/auth.rs", "wrote"),
            None,
        );
        bag.absorb_tool_result(
            &cmd_fail("cargo test auth", "assertion failed left=1"),
            Some(&expectation),
        );
        assert!(bag.regression_failed);
        bag.absorb_tool_result(
            &cmd_ok("cargo test auth"),
            Some(&CommandExpectation::ExpectedSuccess),
        );
        assert!(!bag.regression_failed, "passing repro command clears it");
        assert!(bag.reproduction.is_empty());
        bag.mark_verify(true, vec!["cargo test auth".into()]);
        assert_eq!(bag.verify_ok, Some(true));
        assert!(bag.verify_commands.contains(&"cargo test auth".to_owned()));
        // repro_commands remain a separate ledger.
        assert_eq!(bag.repro_commands, vec!["cargo test auth".to_owned()]);
    }

    #[test]
    fn reproduction_expected_failure_with_wrong_signature_remains_unresolved() {
        let expectation = CommandExpectation::Reproduction(
            FailureExpectation::default().with_output("NullPointerException"),
        );
        let requirement = EvidenceRequirement::CommandOutcome {
            command_contains: String::new(),
            expectation: expectation.clone(),
        };
        let semantic = EvidenceRequirement::SemanticProof {
            target: SemanticTarget::Reproduction,
        };
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_fail("npm test", "FAIL auth_login expected NullPointer"),
            Some(&expectation),
        );
        assert!(!bag.satisfies(&requirement));
        assert!(!bag.satisfies(&semantic));
        assert!(bag.repro_commands.is_empty());

        let mut criterion = AcceptanceCriterion::new("rc", "failure reproduced", semantic);
        assert!(!criterion.is_met());
        // CommandFailed (bare) must never complete a Reproduction criterion.
        let bare = CommandExpectation::ExpectedFailure;
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(
            &cmd_fail("something", "failed for other reasons"),
            Some(&bare),
        );
        assert!(bag
            .items
            .iter()
            .any(|i| matches!(i.kind, EvidenceKind::CommandFailed { .. })));
        assert!(!bag.satisfies(&EvidenceRequirement::SemanticProof {
            target: SemanticTarget::Reproduction
        }));
        assert!(!criterion.absorb(
            bag.items
                .iter()
                .find(|i| matches!(i.kind, EvidenceKind::CommandFailed { .. }))
                .unwrap()
        ));
    }

    #[test]
    fn reproduction_generic_fail_token_cannot_be_sole_fingerprint() {
        let generic_only = FailureExpectation::default().with_output("fail");
        assert!(!generic_only.has_specific_fingerprint());
        assert!(!generic_only.evaluate(Some(1), "test x ... fail", ""));

        // Explicit user opt-in allows it.
        let allowed = FailureExpectation::default()
            .with_output("fail")
            .allowing_generic();
        assert!(allowed.evaluate(Some(1), "test x ... fail", ""));

        // capture mint refuses generic-only / environmental outcomes.
        assert!(
            FailureExpectation::mint_from_observation(Some(1), "something failed", "", "ls")
                .is_none()
        );
        let minted = FailureExpectation::mint_from_observation(
            Some(101),
            "test auth ... FAILED\nassertion failed left=1",
            "exit=101",
            "cargo test auth",
        );
        assert!(minted.is_some(), "application failure should mint");
        let minted = minted.unwrap();
        assert!(minted.test_name.is_some() || !minted.output_contains.is_empty());
        assert!(minted.evaluate(
            Some(101),
            "test auth ... FAILED\nassertion failed left=1",
            "exit=101"
        ));
    }
}
