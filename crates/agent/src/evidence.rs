//! Criterion-specific evidence. Tool success alone never proves a criterion.

use crate::protocol::{ToolErrorCode, ToolName, ToolResult};
use serde::{Deserialize, Serialize};

/// How a command result is interpreted for evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandExpectation {
    /// Command must exit 0.
    ExpectedSuccess,
    /// Command must exit non-zero (bug reproduction).
    ExpectedFailure,
    /// Command must exit non-zero AND output must contain the signature.
    ExpectedFailureSignature { signature: String },
    /// Command must exit 0 AND output must contain the needle.
    ExpectedOutputMatch { needle: String },
}

impl CommandExpectation {
    pub fn label(&self) -> String {
        match self {
            Self::ExpectedSuccess => "exit 0".into(),
            Self::ExpectedFailure => "non-zero exit".into(),
            Self::ExpectedFailureSignature { signature } => format!("fail + /{signature}/"),
            Self::ExpectedOutputMatch { needle } => format!("pass + /{needle}/"),
        }
    }

    /// Evaluate a command outcome against this expectation.
    pub fn evaluate(&self, exit_code: Option<i32>, output: &str) -> bool {
        match self {
            Self::ExpectedSuccess => exit_code == Some(0),
            Self::ExpectedFailure => exit_code.map(|c| c != 0).unwrap_or(true),
            Self::ExpectedFailureSignature { signature } => {
                // Signature match alone can prove reproduction when the test
                // harness already failed (non-zero) OR when the output shows
                // the failure signature even if exit metadata is missing.
                output
                    .to_ascii_lowercase()
                    .contains(&signature.to_ascii_lowercase())
                    || signature.eq_ignore_ascii_case("fail")
            }
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
    CommandFailedAsExpected {
        command: String,
        exit_code: Option<i32>,
        expectation: String,
        signature_matched: bool,
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
    /// Failure reproduced — needs an expected-failure command outcome.
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
            EvidenceKind::CommandFailedAsExpected {
                command,
                exit_code,
                expectation,
                ..
            } => format!(
                "cmd-expected-fail:{command}#{} ({expectation})",
                exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into())
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
                        | EvidenceKind::CommandFailedAsExpected { .. }
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
                if matches!(
                    expectation,
                    CommandExpectation::ExpectedFailure
                        | CommandExpectation::ExpectedFailureSignature { .. }
                ) {
                    false
                } else if command_contains.is_empty() {
                    expectation.evaluate(Some(*exit_code), command)
                } else {
                    command.contains(command_contains.as_str())
                        && expectation.evaluate(Some(*exit_code), command)
                }
            }
            (
                Self::CommandOutcome {
                    command_contains,
                    expectation,
                },
                EvidenceKind::CommandFailedAsExpected {
                    command,
                    exit_code,
                    signature_matched,
                    ..
                },
            ) => {
                let cmd_ok =
                    command_contains.is_empty() || command.contains(command_contains.as_str());
                if !cmd_ok {
                    return false;
                }
                match expectation {
                    CommandExpectation::ExpectedFailure => true,
                    CommandExpectation::ExpectedFailureSignature { signature } => {
                        *signature_matched
                            && expectation.evaluate(
                                *exit_code,
                                if *signature_matched {
                                    signature.as_str()
                                } else {
                                    ""
                                },
                            )
                    }
                    _ => false,
                }
            }
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
                (SemanticTarget::Reproduction, EvidenceKind::CommandFailedAsExpected { .. }) => {
                    true
                }
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
                let exit_code = result
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
                    .or(Some(1));
                let expectation = expectation
                    .cloned()
                    .unwrap_or(CommandExpectation::ExpectedSuccess);
                let err_text = result
                    .error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("");
                let signature_matched = match &expectation {
                    CommandExpectation::ExpectedFailureSignature { signature } => {
                        err_text
                            .to_ascii_lowercase()
                            .contains(&signature.to_ascii_lowercase())
                            || result
                                .output
                                .to_ascii_lowercase()
                                .contains(&signature.to_ascii_lowercase())
                    }
                    CommandExpectation::ExpectedFailure => true,
                    _ => false,
                };
                let expects_failure = matches!(
                    expectation,
                    CommandExpectation::ExpectedFailure
                        | CommandExpectation::ExpectedFailureSignature { .. }
                );
                let signature_ok = match &expectation {
                    CommandExpectation::ExpectedFailureSignature { .. } => signature_matched,
                    _ => true,
                };
                if !expects_failure || !signature_ok {
                    // Unexpected failure, or failed without the expected signature.
                    return None;
                }
                EvidenceKind::CommandFailedAsExpected {
                    command: command.clone(),
                    exit_code,
                    expectation: expectation.label(),
                    signature_matched,
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
    pub verify_commands: Vec<String>,
    pub model_claimed_done: bool,
    pub denied: Vec<String>,
    pub files_written: Vec<String>,
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
        if result.ok {
            if ToolName::parse(&result.name)
                .map(|n| n.is_mutation())
                .unwrap_or(false)
            {
                self.files_written.push(result.input.clone());
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
        if let Some(item) = evidence_from_tool_result(result, expectation) {
            self.push(item);
        }
    }

    pub fn mark_verify(&mut self, ok: bool, commands: Vec<String>) {
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
            expectation: CommandExpectation::ExpectedFailureSignature {
                signature: "assertion failed".into(),
            },
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
        let expectation = CommandExpectation::ExpectedFailureSignature {
            signature: "assertion failed".into(),
        };
        let mut bag = EvidenceBag::default();
        bag.absorb_tool_result(&cmd_ok("cargo test"), Some(&expectation));
        // ok command + expected failure → no CommandFailedAsExpected evidence
        assert!(!bag.satisfies(&requirement), "bag={:?}", bag.items);
    }

    #[test]
    fn failing_reproduction_can_be_successful_evidence() {
        let expectation = CommandExpectation::ExpectedFailureSignature {
            signature: "assertion failed".into(),
        };
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
            .any(|i| matches!(i.kind, EvidenceKind::CommandFailedAsExpected { .. })));
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
        let expectation = CommandExpectation::ExpectedFailureSignature {
            signature: "NullPointerException".into(),
        };
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
        // Reproduction: only expected-failure outcomes.
        let repro = EvidenceRequirement::SemanticProof {
            target: SemanticTarget::Reproduction,
        };
        let expectation = CommandExpectation::ExpectedFailureSignature {
            signature: "assertion failed".into(),
        };
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
}
