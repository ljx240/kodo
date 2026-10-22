//! Verification commands and failure compression for the repair loop.
//!
//! Verification is scoped to **user acceptance criteria** via
//! [`VerificationPlan`]: commands are ordered exact → package → typecheck →
//! broad, each command carries the `criterion_ids` it may prove, and a pass
//! is never auto-applied to every criterion.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureClass {
    /// Real product/test/assertion failure the agent should repair.
    Product,
    /// Missing compiler, network outage, command-not-found — not a code regression.
    Infrastructure,
    Timeout,
    Cancelled,
    Denied,
}

impl FailureClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Infrastructure => "infrastructure",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Denied => "denied",
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Why it failed, read from the process state and the output itself.
    /// `None` when the run passed, and `None` for a record written before this
    /// field existed.
    #[serde(default)]
    pub triage: Option<FailureTriage>,
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
        let triage = if outcome.ok {
            None
        } else {
            Some(triage_verify_failure(
                &outcome.command.command,
                outcome.exit_code,
                outcome.timed_out,
                outcome.cancelled,
                &outcome.output_tail,
            ))
        };
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
            triage,
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

/// What a failed command means for the next move.
///
/// [`FailureClass`] answers "which marker did the runtime land on" and is
/// persisted, so its shape cannot change. This answers "why did it fail, and
/// is there anything to repair?" over the same evidence:
/// * `Product` — the output names source or tests; code repair is the job.
/// * `Infrastructure` — the environment blocked the run; code repair must not
///   be attempted, because that is how an outage turns into a bad diff.
/// * `Transient` — a temporary external condition; a re-run is the move.
/// * `Unknown` — a non-zero exit with nothing identifiable in the output. It
///   stays on the repair path (unattributed is not the same as environmental),
///   but it does not pretend to know a category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureTriage {
    Product,
    Infrastructure,
    Transient,
    Unknown,
}

impl FailureTriage {
    /// The stable wire label, shared with the frozen corpus and the decision
    /// assistant's option list.
    pub fn label(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Infrastructure => "infrastructure",
            Self::Transient => "transient",
            Self::Unknown => "unknown",
        }
    }

    /// Only a confirmed environment fault may suppress a code repair.
    pub fn blocks_code_repair(self) -> bool {
        matches!(self, Self::Infrastructure)
    }

    /// A temporary external fault is worth re-running before anything else.
    pub fn is_retryable(self) -> bool {
        matches!(self, Self::Transient)
    }

    /// Everything this class can say — used to keep the label vocabulary and
    /// the corpus in one place.
    pub const ALL: [FailureTriage; 4] = [
        Self::Product,
        Self::Infrastructure,
        Self::Transient,
        Self::Unknown,
    ];
}

/// Triage a failed verification command from real evidence.
///
/// Signals are used strongest-first, because each kind answers a different
/// question and a weaker one must never override a harder one:
///
/// 1. the process exit code — a shell convention about the *process*, not the code;
/// 2. network-interruption shapes — a temporary external condition;
/// 3. environment/toolchain shapes — a persistent external condition;
/// 4. source-or-test shapes — an actual product failure;
/// 5. nothing identifiable — `Unknown`.
///
/// Every entry in 2–4 is an error-code name, an exit-code convention, an HTTP
/// status phrase, a toolchain object or a *source location* — not a loose word.
/// A failure whose output carries no identifiable subject is deliberately left
/// unattributed rather than guessed at.
pub fn triage_verify_failure(
    command: &str,
    exit_code: Option<i32>,
    timed_out: bool,
    cancelled: bool,
    output: &str,
) -> FailureTriage {
    let lower_output = output.to_ascii_lowercase();
    let lower_all = format!("{} \n{lower_output}", command.to_ascii_lowercase());

    // (1) Process state. A deadline or a cancel is an interrupted attempt: it
    // says nothing about the source, and re-running is a legitimate next move.
    // A user-initiated cancel still reaches the caller as
    // `FailureClass::Cancelled`, so nothing here re-runs it on its own.
    if timed_out || cancelled {
        return FailureTriage::Transient;
    }
    // 126 = found but not executable, 127 = command not found: shell
    // conventions about the *process*, so they outrank any text. 137/139 are
    // deliberately absent — SIGKILL may be the OOM killer or a timeout
    // wrapper, and a segfault may be the code under test, so those need the
    // output to say which.
    if matches!(exit_code, Some(126) | Some(127)) {
        return FailureTriage::Infrastructure;
    }

    // (2) Temporary external conditions.
    const TRANSIENT: [&str; 15] = [
        "econnreset",
        "econnaborted",
        "etimedout",
        "ehostunreach",
        "enethunreach",
        "eai_again",
        "connection reset",
        "connection timed out",
        "name resolution",
        "rate limit",
        "retry after",
        "upstream connect",
        "socket hang up",
        "unexpected end of stream",
        "spurious network error",
    ];
    // The status codes that mean "the transport failed, ask again later". A
    // `500` from the code under test is deliberately absent: that is a product
    // answer, not a transport one, so only the `status 5xx` / gateway-reason
    // shapes count and a bare three-digit number never does.
    const TRANSIENT_STATUS: [&str; 7] = [
        "bad gateway",
        "service unavailable",
        "gateway time-out",
        "gateway timeout",
        "too many requests",
        "status 429",
        "status 50",
    ];
    // `timed out` / `was canceled` only count as transport when they name a
    // request or an operation, never as a bare word.
    const TRANSIENT_PHRASES: [&str; 4] = [
        "request timed out",
        "timed out after",
        "operation was canceled",
        "operation was cancelled",
    ];
    if TRANSIENT.iter().any(|m| lower_all.contains(m))
        || TRANSIENT_STATUS.iter().any(|m| lower_output.contains(m))
        || TRANSIENT_PHRASES.iter().any(|m| lower_output.contains(m))
    {
        return FailureTriage::Transient;
    }

    // (3) Persistent environment faults: the tool, the dependency set, the
    // filesystem or the socket the environment owns.
    const INFRASTRUCTURE: [&str; 26] = [
        "command not found",
        "no such file or directory",
        "cannot find crate",
        "can't find crate",
        "linking with",
        "linker `",
        "custom build command",
        "failed to select a version",
        "could not find `cargo.toml`",
        "cannot find module",
        "cannot find type definition",
        "missing script",
        "no space left",
        "disk quota",
        "killed (oom)",
        "out of memory",
        "eacces",
        "enospc",
        "enoent",
        "eperm",
        "econnrefused",
        "eaddrinuse",
        "eaddrnotavail",
        "docker daemon",
        "network is unreachable",
        "proxy error",
    ];
    if INFRASTRUCTURE.iter().any(|m| lower_all.contains(m)) {
        return FailureTriage::Infrastructure;
    }

    // (4) The failure names source or tests. Each marker is a diagnostic the
    // product itself produced, not a word that happens to appear in prose.
    const PRODUCT: [&str; 16] = [
        "error[",
        "error ts",
        "panicked at",
        "assertion",
        "test failed",
        "test result:",
        "could not compile",
        "not assignable",
        "is not satisfied",
        "no method named",
        "failed to resolve",
        "syntaxerror",
        "referenceerror",
        "typeerror:",
        // Assertion wording. `unexpected token` also contains this, and that
        // is intended: a parse complaint is the product talking.
        "expected",
        // `Tests: 1 failed, 2 passed` and `1 passed; 2 failed`
        "tests:",
    ];
    if PRODUCT.iter().any(|m| lower_output.contains(m)) {
        return FailureTriage::Product;
    }
    // A compiler or test diagnostic that names a file and a position is about
    // this repository's code, whatever else it says.
    if has_source_location(&lower_output) || names_failing_test(&lower_output) {
        return FailureTriage::Product;
    }

    // (5) No identifiable subject. `error: test failed` style lines are caught
    // above; what is left here is a bare exit with nothing to attribute.
    if has_identifiable_subject(&lower_output) {
        FailureTriage::Product
    } else {
        FailureTriage::Unknown
    }
}

/// Source extensions the toolchains in this project report diagnostics against.
const SOURCE_EXTENSIONS: [&str; 11] = [
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt",
];

fn is_file_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '\\' | '[' | ']' | '.')
}

/// A file-name token that carries a known source extension.
fn is_source_file(token: &str) -> bool {
    match token.rfind('.') {
        Some(dot) if dot > 0 => SOURCE_EXTENSIONS.contains(&&token[dot + 1..]),
        _ => false,
    }
}

/// The maximal run of file-name characters ending just before `upto`.
fn file_token(output: &str, upto: usize) -> Option<&str> {
    let start = output[..upto]
        .char_indices()
        .rfind(|(_, c)| !is_file_char(*c))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    Some(&output[start..upto])
}

/// `path/to/file.ext` followed by a line position, in either the rustc and
/// node `foo.rs:12:5` shape or the tsc `foo.tsx(12,7)` shape.
fn has_source_location(output: &str) -> bool {
    for (pos, ch) in output.char_indices() {
        if ch != ':' && ch != '(' {
            continue;
        }
        // A position must start with a digit: `foo.rs: warning` is prose.
        if !output[pos + ch.len_utf8()..].starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        if file_token(output, pos).is_some_and(is_source_file) {
            return true;
        }
    }
    false
}

/// A test runner's own per-test failure line: `FAIL src/verify.test.ts > …`,
/// `FAILED tests/a.py::test_b`, `not ok 1 - …`.
///
/// The status token alone is not evidence — a bare `FAILED` is exactly what an
/// unattributed run leaves on the wire — so the line must also name a file.
fn names_failing_test(output: &str) -> bool {
    output.lines().any(|line| {
        let trimmed = line.trim_start().to_ascii_lowercase();
        if !["fail ", "failed ", "not ok "]
            .iter()
            .any(|prefix| trimmed.starts_with(prefix))
        {
            return false;
        }
        trimmed.split(|c| !is_file_char(c)).any(is_source_file)
    })
}

/// Does the output carry anything an attribution could hang on?
///
/// Three diagnostic shapes count: an identifier-style error code (`E0308`,
/// `TS2322`), a counted summary (`2 failed`, `1 passed`), or an assertion
/// comparison (`expected … actual …`). `exit status 1`, `exit code 1`,
/// `command failed`, `FAILED`, `???` and an empty tail carry none of them, so
/// naming a class from them would be a guess rather than a reading.
fn has_identifiable_subject(output: &str) -> bool {
    if output.trim().is_empty() {
        return false;
    }
    // Letters then two or more digits inside one token: an error code, not a
    // version number or a count.
    let coded = output
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| {
            let letters = token
                .find(|c: char| c.is_ascii_digit())
                .unwrap_or(token.len());
            let digits = token.len() - letters;
            letters >= 1 && letters <= 6 && digits >= 2
        });
    // `1 passed; 2 failed` — a count immediately before a result word.
    let words: Vec<&str> = output
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
        .collect();
    let counted = words.windows(2).any(|pair| {
        !pair[0].is_empty()
            && pair[0].chars().all(|c| c.is_ascii_digit())
            && matches!(
                pair[1],
                "failed" | "passed" | "failures" | "errors" | "error" | "expected"
            )
    });
    let comparison = output.contains("expected") && output.contains("actual");
    coded || counted || comparison
}

/// Where a failure classification came from.
///
/// The existing [`FailureClass`] enum stays exactly as it is — including
/// its persisted form. This is a separate, local reading of the same
/// evidence: it answers "how sure are we?" rather than "what class is it?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureOrigin {
    /// Every failed item carries an explicit, non-negotiable signal: the
    /// user denied it, the user cancelled it, the command does not exist,
    /// the environment refused, the run timed out.
    HardEvidence,
    /// The batch mixes explicit signals with default guesses, so the
    /// aggregate class is a rule's judgement over mixed evidence.
    Heuristic,
    /// No explicit marker matched: the default `Product` guess.
    Default,
}

/// Read the origin of a failed verification batch.
///
/// `None` when nothing failed — there is no failure to reason about.
pub fn failure_evidence_origin(evidence: &[VerificationEvidence]) -> Option<FailureOrigin> {
    let failed: Vec<&VerificationEvidence> = evidence.iter().filter(|e| !e.ok).collect();
    if failed.is_empty() {
        return None;
    }
    let explicit = failed
        .iter()
        .filter(|e| {
            matches!(
                e.failure_class,
                Some(
                    FailureClass::Denied
                        | FailureClass::Cancelled
                        | FailureClass::Timeout
                        | FailureClass::Infrastructure
                )
            )
        })
        .count();
    if explicit == failed.len() {
        return Some(FailureOrigin::HardEvidence);
    }
    if explicit > 0 {
        return Some(FailureOrigin::Heuristic);
    }
    Some(FailureOrigin::Default)
}

/// Whether a failed verification batch may be handed to the optional
/// decision component at all.
///
/// Only a plain non-zero exit with the default guess qualifies. A denial, a
/// cancellation, a timeout, an explicit environment marker, a missing exit
/// code or an exhausted repair budget all keep the failure on the existing
/// path — the decision component never gets to overturn them.
pub fn failure_triage_eligible(evidence: &[VerificationEvidence], budget_exhausted: bool) -> bool {
    if budget_exhausted {
        return false;
    }
    let failed: Vec<&VerificationEvidence> = evidence.iter().filter(|e| !e.ok).collect();
    if failed.is_empty() {
        return false;
    }
    failed.iter().all(|e| {
        e.failure_class == Some(FailureClass::Product)
            && !matches!(
                e.triage,
                Some(FailureTriage::Infrastructure | FailureTriage::Transient)
            )
            && e.exit_code.is_some_and(|code| code != 0)
    })
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
        // The persisted class says which marker matched; the triage says
        // whether the failure has anything to do with the source. An
        // environment fault must block repair even when the legacy marker
        // defaulted to `Product` (e.g. a verify command that cannot find its
        // own `Cargo.toml`).
        let all_environment = failed.iter().all(|e| {
            e.failure_class == Some(FailureClass::Infrastructure)
                || e.triage.is_some_and(FailureTriage::blocks_code_repair)
        });
        let class = if failed
            .iter()
            .any(|e| e.failure_class == Some(FailureClass::Denied))
        {
            FailureClass::Denied
        } else if all_environment {
            FailureClass::Infrastructure
        } else if failed
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
            FailureClass::Infrastructure | FailureClass::Denied | FailureClass::Cancelled => Self {
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

/// How the verification runner executes its commands.
///
/// Unconfined execution is an **explicit** choice, never the meaning of a
/// missing value: the runner refuses to execute when the caller never
/// established a sandbox, exactly like the agent's own command path.
#[derive(Debug, Clone)]
pub enum VerificationConfinement {
    /// Confined by the OS sandbox under the session's policy.
    Confined(kodo_sandbox::SandboxContext),
    /// Explicitly unconfined.
    ///
    /// Only for the trusted control plane — commands Kodo itself issues.
    /// A model-planned verification command never lands here.
    Unconfined,
}

impl VerificationConfinement {
    /// The context, when the commands are confined.
    pub fn context(&self) -> Option<&kodo_sandbox::SandboxContext> {
        match self {
            Self::Confined(context) => Some(context),
            Self::Unconfined => None,
        }
    }
}

/// Infers and runs build/typecheck/test/lint with timeouts.
pub struct VerificationRunner {
    pub default_timeout_ms: u64,
    /// OS confinement for the commands this runner executes.
    ///
    /// Verification commands are planned by the model, so they run under the
    /// same sandbox as any other model-driven command. `None` means the
    /// caller did not establish a sandbox — the runner then **refuses** to
    /// execute rather than falling back to an unconfined run.
    pub confinement: Option<VerificationConfinement>,
}

impl Default for VerificationRunner {
    fn default() -> Self {
        Self {
            default_timeout_ms: 90_000,
            confinement: None,
        }
    }
}

impl VerificationRunner {
    /// Run verification commands under OS confinement.
    pub fn with_sandbox(mut self, sandbox: kodo_sandbox::SandboxContext) -> Self {
        self.confinement = Some(VerificationConfinement::Confined(sandbox));
        self
    }
}

/// How far upward a changed path may search for the manifest that owns it.
const MAX_PKG_WALK_DEPTH: usize = 4;
/// At most this many packages take part in one verification plan.
const MAX_PKG_DIRS: usize = 3;

/// Project-relative directories (`.` for the root) that own `marker` for the
/// given changed paths, nearest ancestor first. Deliberately layout-agnostic:
/// nothing about `crates/` or `apps/desktop/` is assumed.
fn nearest_pkg_dirs(project: &Path, changed_files: &[String], marker: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let owns = |rel: &str| {
        crate::tools::resolve_in_project(project, rel)
            .ok()
            .filter(|p| p.is_file())
            .is_some()
    };
    for path in changed_files {
        let normalized = path.trim_start_matches("./").replace('\\', "/");
        let mut dir = PathBuf::from(&normalized);
        for _ in 0..MAX_PKG_WALK_DEPTH {
            if !dir.pop() {
                break;
            }
            let rel_dir = dir.to_string_lossy().replace('\\', "/");
            let candidate = if rel_dir.is_empty() {
                marker.to_owned()
            } else {
                format!("{rel_dir}/{marker}")
            };
            if owns(&candidate) {
                let key = if rel_dir.is_empty() {
                    ".".into()
                } else {
                    rel_dir
                };
                if !out.contains(&key) {
                    out.push(key);
                }
                break;
            }
        }
    }
    if out.is_empty() && owns(marker) {
        out.push(".".to_owned());
    }
    out.truncate(MAX_PKG_DIRS);
    out
}

/// Package name declared by `<rel_dir>/Cargo.toml`, falling back to the
/// directory name. Only the `[package]` section is considered, so dependency
/// entries cannot be mistaken for the package name.
fn cargo_pkg_name(project: &Path, rel_dir: &str) -> String {
    let manifest_rel = if rel_dir == "." {
        "Cargo.toml".to_owned()
    } else {
        format!("{rel_dir}/Cargo.toml")
    };
    let fallback = || {
        if rel_dir == "." || rel_dir == "/" {
            project
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "root".to_owned())
        } else {
            rel_dir.rsplit('/').next().unwrap_or(rel_dir).to_owned()
        }
    };
    let Ok(path) = crate::tools::resolve_in_project(project, &manifest_rel) else {
        return fallback();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return fallback();
    };
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("name")
            .and_then(|r| r.trim_start().strip_prefix('='))
        {
            let name = rest.trim().trim_matches('"').trim_matches('\'');
            if !name.is_empty() {
                return name.to_owned();
            }
        }
    }
    fallback()
}

/// Run one named test with whatever runner the project actually uses.
/// Returns `None` when there is no recognizable runner, so the plan degrades to
/// evidence-derived commands instead of inventing a Rust command for a JS repo.
fn test_command_for_filter(
    project: &Path,
    changed_files: &[String],
    filter: &str,
) -> Option<String> {
    // The filter is interpolated into a shell command: keep it to test-name
    // characters so a criterion description can never smuggle arguments.
    let safe = !filter.is_empty()
        && filter
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '/'));
    if !safe {
        return None;
    }
    let has_file = |rel: &str| {
        crate::tools::resolve_in_project(project, rel)
            .ok()
            .filter(|p| p.is_file())
            .is_some()
    };
    if has_file("Cargo.toml") {
        return Some(format!("cargo test {filter} --quiet"));
    }
    if let Some(dir) = nearest_pkg_dirs(project, changed_files, "Cargo.toml")
        .into_iter()
        .next()
    {
        return Some(format!(
            "cargo test --manifest-path {dir}/Cargo.toml --quiet {filter}"
        ));
    }
    if let Some(dir) = nearest_pkg_dirs(project, changed_files, "package.json")
        .into_iter()
        .next()
    {
        return Some(if dir == "." {
            format!("npm test --silent -- {filter}")
        } else {
            format!("npm --prefix {dir} test --silent -- {filter}")
        });
    }
    if ["pyproject.toml", "pytest.ini", "setup.cfg"]
        .iter()
        .any(|name| has_file(name))
    {
        return Some(format!("python -m pytest -q -k {filter}"));
    }
    None
}

/// Prose files whose change cannot be exercised by compiling or testing code.
fn only_documentation_changed(changed_files: &[String]) -> bool {
    !changed_files.is_empty()
        && changed_files.iter().all(|path| {
            let lower = path.to_ascii_lowercase();
            [".md", ".mdx", ".markdown", ".rst", ".adoc"]
                .iter()
                .any(|ext| lower.ends_with(ext))
        })
}

/// A cargo test invocation that resolves the package from the project root,
/// whether its manifest is the root manifest or a nested one.
fn cargo_test_command(rel_dir: &str, package: &str) -> String {
    if rel_dir == "." {
        format!("cargo test -p {package} --quiet")
    } else {
        format!("cargo test --manifest-path {rel_dir}/Cargo.toml -p {package} --quiet")
    }
}

fn cargo_check_command(rel_dir: &str) -> String {
    if rel_dir == "." {
        "cargo check --quiet".to_owned()
    } else {
        format!("cargo check --manifest-path {rel_dir}/Cargo.toml --quiet")
    }
}

impl VerificationRunner {
    pub fn new(default_timeout_ms: u64) -> Self {
        Self {
            default_timeout_ms,
            confinement: None,
        }
    }

    /// A runner that executes its commands **without** confinement.
    ///
    /// An explicit, named decision for the trusted control plane. The
    /// runner's default is to refuse, so an unconfined verification can
    /// only happen when a caller asks for it by name.
    pub fn unconfined(default_timeout_ms: u64) -> Self {
        Self {
            default_timeout_ms,
            confinement: Some(VerificationConfinement::Unconfined),
        }
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

        // Map changed paths onto the packages that own them. No repository
        // layout is hardcoded — the nearest ancestor carrying a manifest wins,
        // so `apps/desktop`, `packages/web` and a flat root all behave alike.
        let crates_changed: Vec<String> = nearest_pkg_dirs(project, changed_files, "Cargo.toml")
            .iter()
            .map(|dir| cargo_pkg_name(project, dir))
            .fold(Vec::new(), |mut acc, name| {
                if !acc.contains(&name) {
                    acc.push(name);
                }
                acc
            });

        if crate::tools::resolve_in_project(project, "Cargo.toml").is_ok_and(|p| p.is_file()) {
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

        for dir in nearest_pkg_dirs(project, changed_files, "package.json") {
            let manifest_rel = if dir == "." {
                "package.json".to_owned()
            } else {
                format!("{dir}/package.json")
            };
            let Ok(manifest) = crate::tools::resolve_in_project(project, &manifest_rel) else {
                continue;
            };
            let text = fs::read_to_string(&manifest).unwrap_or_default();
            let run_prefix = if dir == "." {
                "npm run ".to_owned()
            } else {
                format!("npm --prefix {dir} run ")
            };
            let test_prefix = if dir == "." {
                "npm test --silent".to_owned()
            } else {
                format!("npm --prefix {dir} test --silent")
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
                            push(&mut cmds, kind, format!("{run_prefix}{name}"));
                        }
                    }
                }
            }
            // Fallback when the manifest declares no test script.
            if !cmds.iter().any(|c| c.kind == VerifyKind::Test) {
                push(&mut cmds, VerifyKind::Test, test_prefix);
            }
        }

        if ["pyproject.toml", "pytest.ini"]
            .iter()
            .any(|name| crate::tools::resolve_in_project(project, name).is_ok_and(|p| p.is_file()))
        {
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
    ///
    /// `repro_commands` are the commands the agent actually used to reproduce
    /// the defect (from the evidence bag). They are the strongest available
    /// regression signal — the same invocation failed before the fix — so they
    /// lead the plan instead of a guessed runner.
    pub fn build_plan(
        project: &Path,
        criteria: &[(String, String)],
        changed_files: &[String],
        task_type: &str,
        repro_commands: &[String],
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

        let cargo_dirs = nearest_pkg_dirs(project, changed_files, "Cargo.toml");
        let mut cargo_packages: Vec<(String, String)> = Vec::new();
        for dir in &cargo_dirs {
            let name = cargo_pkg_name(project, dir);
            if !cargo_packages.iter().any(|(_, n)| n == &name) {
                cargo_packages.push((dir.clone(), name));
            }
        }
        let package = cargo_packages.first().map(|(_, name)| name.clone());
        let docs_only = only_documentation_changed(changed_files);
        let root_manifest =
            crate::tools::resolve_in_project(project, "Cargo.toml").is_ok_and(|p| p.is_file());

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

            // 1a) The commands that reproduced the defect. Re-running the exact
            // invocation that failed before the fix is the strongest regression
            // signal available, and it is language-agnostic.
            if !all_ids.is_empty() {
                for command in repro_commands.iter().take(3) {
                    ctx.push(
                        VerifyKind::Test,
                        command.clone(),
                        VerifyTier::ExactRegression,
                        all_ids.clone(),
                    );
                }
            }

            // 1b) A test named by the criterion, run with the project's own
            // runner. Skipped when the repository has no recognizable runner —
            // `cargo test <name>` must never be injected into a JS project.
            for (cid, filter) in &exact_targets {
                if let Some(command) = test_command_for_filter(project, changed_files, filter) {
                    ctx.push(
                        VerifyKind::Test,
                        command,
                        VerifyTier::ExactRegression,
                        vec![cid.clone()],
                    );
                }
            }

            // 2) Related package tests — generic verification criteria only.
            let generic_for_package: Vec<String> = if exact_targets.is_empty() {
                all_ids.clone()
            } else {
                generic_ids.clone()
            };
            if !docs_only && !generic_for_package.is_empty() {
                for (dir, pkg) in cargo_packages.iter().take(3) {
                    ctx.push(
                        VerifyKind::Test,
                        cargo_test_command(dir, pkg),
                        VerifyTier::Package,
                        generic_for_package.clone(),
                    );
                }
                if cargo_packages.is_empty() && root_manifest {
                    ctx.push(
                        VerifyKind::Test,
                        "cargo test --quiet".to_owned(),
                        VerifyTier::Package,
                        generic_for_package.clone(),
                    );
                }
            }

            // Reuse manifest inference for npm/Python instead of a Rust-only plan.
            if !docs_only && !generic_for_package.is_empty() {
                for command in Self::default()
                    .infer_targeted(project, changed_files)
                    .into_iter()
                    .filter(|c| !c.command.starts_with("cargo "))
                {
                    let tier = match command.kind {
                        VerifyKind::Test => VerifyTier::Package,
                        VerifyKind::Typecheck => VerifyTier::Typecheck,
                        _ => VerifyTier::Broad,
                    };
                    ctx.push(
                        command.kind,
                        command.command,
                        tier,
                        generic_for_package.clone(),
                    );
                }
            }

            // 3) Typecheck / check
            if !docs_only && (!generic_for_package.is_empty() || !exact_targets.is_empty()) {
                let typecheck_ids: Vec<String> = if exact_targets.is_empty() {
                    all_ids.clone()
                } else {
                    generic_ids.clone()
                };
                if !typecheck_ids.is_empty() {
                    if root_manifest {
                        ctx.push(
                            VerifyKind::Build,
                            cargo_check_command("."),
                            VerifyTier::Typecheck,
                            typecheck_ids.clone(),
                        );
                    } else {
                        for (dir, _) in cargo_packages.iter().take(3) {
                            ctx.push(
                                VerifyKind::Build,
                                cargo_check_command(dir),
                                VerifyTier::Typecheck,
                                typecheck_ids.clone(),
                            );
                        }
                    }
                }
            }
        }

        // 4) Broader suite last — only when no package-level test was planned.
        let has_package_cmd = commands.iter().any(|c| c.tier == VerifyTier::Package);
        if !docs_only && !generic_ids.is_empty() && root_manifest && !has_package_cmd {
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
        self.run_plan_with_approval(project, plan, alive, stop_on_fail, &|_| true)
    }

    pub fn run_plan_with_approval(
        &self,
        project: &Path,
        plan: &VerificationPlan,
        alive: &dyn Fn() -> bool,
        stop_on_fail: bool,
        approve: &dyn Fn(&VerifyCommand) -> bool,
    ) -> Vec<VerificationEvidence> {
        let mut out = Vec::new();
        let cwd = project.display().to_string();
        let mut missing_tools: Vec<String> = Vec::new();
        for planned in &plan.commands {
            if !alive() {
                break;
            }
            if !approve(&planned.command) {
                let outcome = blocked_outcome(
                    &planned.command,
                    &cwd,
                    // Denials in a run with no TTY are the default, not a user
                    // decision — say what would have let this through.
                    "验证命令未获批准，已跳过后续验证（交互模式请批准，无人值守请加 --allow-verify）",
                );
                let mut ev = VerificationEvidence::from_outcome(
                    &outcome,
                    &cwd,
                    planned.criterion_ids.clone(),
                    500,
                );
                ev.failure_class = Some(FailureClass::Denied);
                out.push(ev);
                break;
            }
            if !alive() {
                break;
            }
            let binary = first_token(&planned.command.command);
            if missing_tools.contains(&binary) {
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
        //
        // The execution semantics are the same as the agent's: model-planned
        // verification commands are confined, an unconfined run is an
        // explicit choice, and a caller that never established a sandbox
        // gets a refusal instead of an unconfined execution.
        let outcome = match self.confinement.as_ref() {
            Some(VerificationConfinement::Confined(sandbox)) => crate::tools::command_run_confined(
                project,
                &cmd.command,
                alive,
                timeout_secs,
                sandbox,
            ),
            Some(VerificationConfinement::Unconfined) => {
                crate::tools::command_run(project, &cmd.command, alive, timeout_secs)
            }
            None => {
                return VerifyOutcome {
                    command: cmd.clone(),
                    exit_code: Some(126),
                    ok: false,
                    timed_out: false,
                    cancelled: false,
                    cwd: project.display().to_string(),
                    duration_ms: 0,
                    output_tail: "沙箱未初始化，已拒绝执行验证命令".to_owned(),
                    failure: Some(FailureReport {
                        command: cmd.command.clone(),
                        exit_code: Some(126),
                        primary_error: "沙箱未初始化，已拒绝执行验证命令".to_owned(),
                        relevant_files: Vec::new(),
                    }),
                }
            }
        };
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
            if missing_tools.contains(&binary) {
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

    #[test]
    fn a_verification_without_a_sandbox_is_refused_not_run_unconfined() {
        // The verification runner uses the same execution semantics as the
        // agent: a missing sandbox context means the caller never
        // established one, so the command is refused. It must not fall back
        // to an unconfined run.
        let dir =
            std::env::temp_dir().join(format!("kodo-verify-nosandbox-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("must-not-exist.txt");
        let command = VerifyCommand {
            kind: VerifyKind::Test,
            command: format!("echo ran > {}", marker.display()),
            timeout_ms: 5_000,
        };
        let outcome = VerificationRunner::default().run_one(&dir, &command, &|| true);
        assert!(!outcome.ok, "the verification must be refused: {outcome:?}");
        assert!(
            !marker.exists(),
            "the command must never have started: {outcome:?}"
        );
        assert!(
            outcome.output_tail.contains("沙箱未初始化"),
            "the refusal must say why: {outcome:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

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
        let runner = VerificationRunner::unconfined(5_000);
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
        let runner = VerificationRunner::unconfined(10_000);
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
        let runner = VerificationRunner::unconfined(50);
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
        let runner = VerificationRunner::unconfined(5_000);
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
        let runner = VerificationRunner::unconfined(5_000);
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
        let runner = VerificationRunner::unconfined(5_000);
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
        let runner = VerificationRunner::unconfined(30_000);
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
            &[],
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
        let plan = VerificationRunner::build_plan(&dir, &criteria, &[], "bug-fix", &[]);
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
            triage: None,
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
            triage: Some(FailureTriage::Infrastructure),
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
            triage: Some(FailureTriage::Product),
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

    #[test]
    fn triage_reads_process_state_before_any_text() {
        // A deadline or a cancel says nothing about the source.
        assert_eq!(
            triage_verify_failure("cargo test", Some(101), true, false, ""),
            FailureTriage::Transient
        );
        assert_eq!(
            triage_verify_failure("cargo test", Some(101), false, true, ""),
            FailureTriage::Transient
        );
        // 127 is a shell convention about the process, so it outranks a
        // compiler-shaped message in the tail.
        assert_eq!(
            triage_verify_failure(
                "cargo test",
                Some(127),
                false,
                false,
                "error[E0308]: mismatched types",
            ),
            FailureTriage::Infrastructure
        );
    }

    #[test]
    fn triage_environment_faults_block_code_repair() {
        for (cmd, out) in [
            (
                "cargo test",
                "error: linking with `cc` failed: exit status 1",
            ),
            ("cargo test", "rustc: command not found"),
            ("npm run build", "npm ERR! missing script: build"),
            ("cargo test", "No space left on device"),
            (
                "docker build .",
                "docker: Cannot connect to the Docker daemon",
            ),
            (
                "cargo test",
                "error: failed to select a version for the requirement `serde`",
            ),
            (
                "cargo test",
                "error: failed to run custom build command for `ring`",
            ),
            ("node x.js", "Error: Cannot find module 'vite'"),
        ] {
            let triage = triage_verify_failure(cmd, Some(101), false, false, out);
            assert_eq!(triage, FailureTriage::Infrastructure, "{out}");
            assert!(triage.blocks_code_repair(), "{out} must block repair");
        }
    }

    #[test]
    fn triage_transient_faults_are_retried_not_repaired() {
        for (cmd, out) in [
            ("npm install", "read ECONNRESET"),
            ("npm install", "connect ETIMEDOUT 10.0.0.1:443"),
            ("npm install", "502 Bad Gateway"),
            ("curl https://api", "failed with status 429"),
            ("git fetch", "Temporary failure in name resolution"),
            (
                "cargo build",
                "warning: spurious network error (3 tries remaining)",
            ),
        ] {
            let triage = triage_verify_failure(cmd, Some(1), false, false, out);
            assert_eq!(triage, FailureTriage::Transient, "{out}");
            assert!(
                triage.is_retryable() && !triage.blocks_code_repair(),
                "{out}"
            );
        }
    }

    #[test]
    fn triage_product_failures_stay_on_the_repair_path() {
        for out in [
            "error[E0308]: mismatched types\n  --> src/lib.rs:42:5",
            "test result: FAILED. 1 passed; 2 failed",
            "thread 'main' panicked at 'called `Option::unwrap()` on a `None` value'",
            "src/App.tsx(12,7): error TS2322: Type 'string' is not assignable to type 'number'",
            "FAIL src/verify.test.ts > rejects an empty input",
            "assertion failed: expected 3 actual 4",
            "error: test failed, to rerun pass `--no-fail-fast`",
        ] {
            let triage = triage_verify_failure("cargo test", Some(101), false, false, out);
            assert_eq!(triage, FailureTriage::Product, "{out}");
            assert!(!triage.blocks_code_repair(), "{out}");
        }
    }

    #[test]
    fn triage_leaves_an_unattributable_failure_unknown() {
        for out in [
            "",
            "   ",
            "error",
            "exit status 1",
            "exit code 1",
            "FAILED",
            "command failed",
            "???",
            "something went wrong",
        ] {
            let triage = triage_verify_failure("cargo test", Some(1), false, false, out);
            assert_eq!(triage, FailureTriage::Unknown, "{out:?}");
            // Unknown is not an environment verdict, so it must not strand a
            // failure that could still be repaired.
            assert!(!triage.blocks_code_repair(), "{out:?}");
        }
    }

    #[test]
    fn evidence_records_the_triage_from_real_process_state() {
        let failing = VerifyOutcome {
            command: VerifyCommand::test("cargo test"),
            exit_code: Some(101),
            ok: false,
            timed_out: false,
            cancelled: false,
            cwd: "/tmp".into(),
            duration_ms: 1,
            output_tail: "test result: FAILED. 0 passed; 1 failed".into(),
            failure: None,
        };
        let ev = VerificationEvidence::from_outcome(&failing, "/tmp", vec![], 500);
        assert_eq!(ev.triage, Some(FailureTriage::Product));
        assert_eq!(ev.failure_class, Some(FailureClass::Product));

        let timed = VerifyOutcome {
            timed_out: true,
            output_tail: String::new(),
            ..failing
        };
        let ev = VerificationEvidence::from_outcome(&timed, "/tmp", vec![], 500);
        assert_eq!(ev.triage, Some(FailureTriage::Transient));
        assert_eq!(
            ev.failure_class,
            Some(FailureClass::Timeout),
            "the persisted marker keeps its own vocabulary"
        );

        let passed = VerifyOutcome {
            ok: true,
            exit_code: Some(0),
            ..timed
        };
        let ev = VerificationEvidence::from_outcome(&passed, "/tmp", vec![], 500);
        assert_eq!(ev.triage, None);
    }

    #[test]
    fn evidence_written_before_the_triage_field_still_loads() {
        let json = r#"{"command":"cargo test","cwd":"/tmp","exit_code":101,"duration_ms":1,
            "criterion_ids":["c1"],"output_summary":"boom","truncated":false,"ok":false,
            "failure_class":"Product"}"#;
        let ev: VerificationEvidence = serde_json::from_str(json).expect("old record must load");
        assert_eq!(ev.triage, None);
        assert_eq!(ev.failure_class, Some(FailureClass::Product));
    }

    /// The exact evidence of the failed k2k-rust assessment run: the command
    /// could not find a manifest, so nothing about the source was tested. The
    /// legacy marker defaulted to `Product`; the triage read the output
    /// correctly. Repair must follow the triage.
    #[test]
    fn repair_decision_honors_triage_over_the_legacy_marker() {
        let plan = VerificationPlan {
            criterion_ids: vec!["skill_c3".into()],
            changed_files: vec!["k2k-rust/docs/CAPABILITY-ASSESSMENT.md".into()],
            package: Some("k2k-rust".into()),
            task_type: "feature".into(),
            commands: vec![PlannedVerifyCommand {
                command: VerifyCommand::test("cargo test -p k2k-rust --quiet"),
                criterion_ids: vec!["skill_c3".into()],
                tier: VerifyTier::Package,
            }],
        };
        let ev = VerificationEvidence {
            command: "cargo test -p k2k-rust --quiet".into(),
            cwd: "/Users/lijianxue/work/WorkSpace/realtime".into(),
            exit_code: Some(101),
            duration_ms: 96,
            criterion_ids: vec!["skill_c3".into()],
            output_summary: "error: could not find `Cargo.toml` in \
                             `/Users/lijianxue/work/WorkSpace/realtime` or any parent directory"
                .into(),
            truncated: false,
            ok: false,
            failure_class: Some(FailureClass::Product),
            triage: Some(FailureTriage::Infrastructure),
        };
        let decision = RepairDecision::from_evidence(std::slice::from_ref(&ev), &plan, true);
        assert!(
            !decision.can_repair,
            "a manifest resolution failure is not a product regression: {decision:?}"
        );
        assert_eq!(decision.failure_class, FailureClass::Infrastructure);
        assert!(!failure_triage_eligible(std::slice::from_ref(&ev), false));

        // An ambiguous legacy `Product` marker without an explicit triage
        // verdict still goes to repair and remains eligible for assistance.
        let ambiguous = VerificationEvidence {
            exit_code: Some(1),
            output_summary: "something went wrong".into(),
            triage: Some(FailureTriage::Unknown),
            ..ev
        };
        assert!(
            RepairDecision::from_evidence(std::slice::from_ref(&ambiguous), &plan, true).can_repair
        );
        assert!(failure_triage_eligible(&[ambiguous], false));
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
        let runner = VerificationRunner::unconfined(10_000);
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
    #[test]
    fn plan_reuses_npm_python_inference_and_rejects_manifest_aliases() {
        let root =
            std::env::temp_dir().join(format!("kodo-verify-manifests-{}", std::process::id()));
        let project = root.join("repo");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join("package.json"),
            r#"{"scripts":{"test":"echo test","typecheck":"echo check"}}"#,
        )
        .unwrap();
        fs::write(
            project.join("pyproject.toml"),
            "[project]\nname = 'fixture'\n",
        )
        .unwrap();
        let plan = VerificationRunner::build_plan(
            &project,
            &[("c1".into(), "Verification commands pass".into())],
            &["app.js".into()],
            "feature",
            &[],
        );
        for command in ["npm run test", "npm run typecheck", "python -m pytest -q"] {
            assert!(
                plan.commands
                    .iter()
                    .any(|c| c.command.command == command && c.criterion_ids == ["c1"]),
                "missing bound command {command}: {plan:?}"
            );
        }
        #[cfg(unix)]
        {
            let outside = root.join("outside.json");
            fs::write(&outside, r#"{"scripts":{"test":"echo unapproved"}}"#).unwrap();
            fs::remove_file(project.join("package.json")).unwrap();
            std::os::unix::fs::symlink(&outside, project.join("package.json")).unwrap();
            assert!(!VerificationRunner::default()
                .infer(&project)
                .iter()
                .any(|c| c.command.starts_with("npm ")));
            fs::remove_file(project.join("package.json")).unwrap();
            fs::write(
                project.join(".env"),
                r#"{"scripts":{"test":"echo synthetic"}}"#,
            )
            .unwrap();
            std::os::unix::fs::symlink(project.join(".env"), project.join("package.json")).unwrap();
            assert!(!VerificationRunner::default()
                .infer(&project)
                .iter()
                .any(|c| c.command.starts_with("npm ")));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verifier_denial_keeps_bindings_and_never_spawns_the_denied_command() {
        let root =
            std::env::temp_dir().join(format!("kodo-verify-approval-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let plan = VerificationPlan {
            commands: vec![PlannedVerifyCommand {
                command: VerifyCommand::test("echo started > marker.txt"),
                criterion_ids: vec!["c1".into()],
                tier: VerifyTier::ExactRegression,
            }],
            ..Default::default()
        };
        let runner = VerificationRunner::unconfined(5000);
        let denied = runner.run_plan_with_approval(&root, &plan, &|| true, false, &|_| false);
        assert_eq!(denied.len(), 1);
        assert_eq!(denied[0].failure_class, Some(FailureClass::Denied));
        assert_eq!(denied[0].criterion_ids, ["c1"]);
        assert!(!root.join("marker.txt").exists());
        assert!(!RepairDecision::from_evidence(&denied, &plan, true).can_repair);
        let allowed = runner.run_plan_with_approval(&root, &plan, &|| true, false, &|_| true);
        assert!(allowed[0].ok);
        assert!(root.join("marker.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }

    /// A manifest-less repo (the common fixture shape) still gets a plan, as
    /// long as the agent reproduced the defect with a command of its own.
    #[test]
    fn plan_leads_with_repro_commands_when_no_manifest_exists() {
        let root = std::env::temp_dir().join(format!("kodo-verify-repro-{}", std::process::id()));
        let project = root.join("repo");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("app.js"), "module.exports = 1;\n").unwrap();
        fs::write(project.join("test.js"), "console.log('ok');\n").unwrap();

        let plan = VerificationRunner::build_plan(
            &project,
            &[("c1".into(), "Verification commands pass".into())],
            &["app.js".into()],
            "bug-fix",
            &["node test.js".into()],
        );
        assert!(
            plan.commands
                .iter()
                .any(|c| c.command.command == "node test.js"
                    && c.tier == VerifyTier::ExactRegression
                    && c.criterion_ids == ["c1"]),
            "repro command missing from plan: {plan:?}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// A changed file inside a nested crate must not yield `cargo test -p X`
    /// for a project root with no manifest: that invocation failed the
    /// k2k-rust assessment run before a single test was compiled.
    #[test]
    fn nested_crate_plan_uses_a_manifest_path_from_the_project_root() {
        let project = temp_verify_dir("nested-crate");
        fs::create_dir_all(project.join("k2k-rust/src")).unwrap();
        fs::write(
            project.join("k2k-rust/Cargo.toml"),
            "[package]\nname = \"k2k-rust\"\n",
        )
        .unwrap();
        fs::write(project.join("k2k-rust/src/lib.rs"), "").unwrap();
        let plan = VerificationRunner::build_plan(
            &project,
            &[("c1".into(), "Verification commands pass".into())],
            &["k2k-rust/src/lib.rs".into()],
            "bug-fix",
            &[],
        );
        let commands: Vec<&str> = plan
            .commands
            .iter()
            .map(|c| c.command.command.as_str())
            .collect();
        assert!(
            commands
                .contains(&"cargo test --manifest-path k2k-rust/Cargo.toml -p k2k-rust --quiet"),
            "{commands:?}"
        );
        assert!(
            commands.contains(&"cargo check --manifest-path k2k-rust/Cargo.toml --quiet"),
            "{commands:?}"
        );
        assert!(
            !commands.iter().any(|c| c.starts_with("cargo test -p ")),
            "no command may assume the project root owns a manifest: {commands:?}"
        );
        fs::remove_dir_all(project).unwrap();
    }

    /// Writing a report is not a code change: a docs-only change must not
    /// trigger package tests that cannot prove anything about prose.
    #[test]
    fn docs_only_change_plans_no_code_verification() {
        let project = temp_verify_dir("docs-only");
        fs::create_dir_all(project.join("k2k-rust/docs")).unwrap();
        fs::write(
            project.join("k2k-rust/Cargo.toml"),
            "[package]\nname = \"k2k-rust\"\n",
        )
        .unwrap();
        fs::write(
            project.join("k2k-rust/docs/CAPABILITY-ASSESSMENT.md"),
            "# Assessment\n",
        )
        .unwrap();
        let plan = VerificationRunner::build_plan(
            &project,
            &[("c1".into(), "Verification commands pass".into())],
            &["k2k-rust/docs/CAPABILITY-ASSESSMENT.md".into()],
            "feature",
            &[],
        );
        assert!(plan.commands.is_empty(), "plan={:?}", plan.commands);
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn exact_filter_in_a_nested_crate_uses_its_manifest() {
        let project = temp_verify_dir("nested-filter");
        fs::create_dir_all(project.join("k2k-rust/src")).unwrap();
        fs::write(
            project.join("k2k-rust/Cargo.toml"),
            "[package]\nname = \"k2k-rust\"\n",
        )
        .unwrap();
        fs::write(project.join("k2k-rust/src/lib.rs"), "").unwrap();
        assert_eq!(
            test_command_for_filter(
                &project,
                &["k2k-rust/src/lib.rs".into()],
                "test_login_panic"
            )
            .as_deref(),
            Some("cargo test --manifest-path k2k-rust/Cargo.toml --quiet test_login_panic")
        );
        fs::remove_dir_all(project).unwrap();
    }

    /// The exact-regression tier must not invent a Rust command for a project
    /// that has no Cargo.toml — that produced 34 failed tool calls in one run.
    #[test]
    fn exact_regression_filter_never_invents_cargo_for_js_project() {
        let root = std::env::temp_dir().join(format!("kodo-verify-runner-{}", std::process::id()));
        let project = root.join("repo");
        fs::create_dir_all(project.join("web")).unwrap();
        fs::write(
            project.join("web/package.json"),
            r#"{"scripts":{"test":"node --test"}}"#,
        )
        .unwrap();
        fs::write(project.join("web/index.js"), "1\n").unwrap();

        let command =
            test_command_for_filter(&project, &["web/index.js".into()], "test_login_panic");
        assert_eq!(
            command.as_deref(),
            Some("npm --prefix web test --silent -- test_login_panic"),
            "expected the project's own runner"
        );
        // Manifest-less project: no runner can be inferred at all.
        let bare = root.join("bare");
        fs::create_dir_all(&bare).unwrap();
        assert_eq!(
            test_command_for_filter(&bare, &["a.js".into()], "some_test"),
            None
        );
        // Shell metacharacters must not be interpolated into a command.
        assert_eq!(
            test_command_for_filter(&bare, &[], "a; rm -rf /"),
            None,
            "filter must be validated"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
