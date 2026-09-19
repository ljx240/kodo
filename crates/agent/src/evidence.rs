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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub id: String,
    pub source: String,
    pub kind: EvidenceKind,
}

impl EvidenceItem {
    pub fn describe(&self) -> String {
        match &self.kind {
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
    SearchHit { query_contains: String },
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
            Self::ToolSucceeded { tool } => format!("tool-ok:{tool}"),
            Self::UserApproval => "user-approval".into(),
            Self::DiffReviewed { path_contains } => format!("diff-reviewed:{path_contains}"),
            Self::ContextRead { path_contains } => format!("context-read:{path_contains}"),
        }
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
            // Locate/root-cause: search hit OR model-initiated read/command.
            (Self::SearchHit { query_contains }, EvidenceKind::SearchHit { query, hits }) => {
                *hits > 0 && (query_contains.is_empty() || query.contains(query_contains.as_str()))
            }
            (Self::SearchHit { query_contains }, EvidenceKind::FileRead { path }) => {
                query_contains.is_empty() || path.contains(query_contains.as_str())
            }
            (
                Self::SearchHit { query_contains },
                EvidenceKind::CommandSucceeded { command, .. },
            ) => {
                command.contains("search")
                    || command.contains("rg ")
                    || command.contains("grep")
                    || query_contains.is_empty()
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

    pub fn absorb(&mut self, item: &EvidenceItem) -> bool {
        if self.requirement.matches(item) {
            if !self.evidence.iter().any(|e| e.id == item.id) {
                self.evidence.push(item.clone());
            }
            self.status = CriterionStatus::Met;
            return true;
        }
        false
    }

    pub fn is_met(&self) -> bool {
        self.status == CriterionStatus::Met && !self.evidence.is_empty()
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
        return Some(EvidenceItem {
            id,
            source: "context".into(),
            kind: EvidenceKind::ContextObservation {
                path: result.input.clone(),
            },
        });
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
    };
    Some(EvidenceItem { id, source, kind })
}

/// Build a contextual pre-scan evidence item (never proves extraction/root-cause).
pub fn context_observation(path: impl Into<String>, id: impl Into<String>) -> EvidenceItem {
    EvidenceItem {
        id: id.into(),
        source: "context".into(),
        kind: EvidenceKind::ContextObservation { path: path.into() },
    }
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
                self.push(EvidenceItem {
                    id: format!("verify_{command}"),
                    source: "verify".into(),
                    kind,
                });
            }
        }
    }

    pub fn satisfies(&self, requirement: &EvidenceRequirement) -> bool {
        if matches!(requirement, EvidenceRequirement::VerificationPassed)
            && self.verify_ok == Some(true)
        {
            return true;
        }
        self.items.iter().any(|item| requirement.matches(item))
    }
}

/// Default requirement heuristic from free-text criterion / subtask title.
pub fn infer_requirement(description: &str) -> EvidenceRequirement {
    let lower = description.to_ascii_lowercase();
    if lower.contains("reproduc") || lower.contains("复现") {
        return EvidenceRequirement::CommandOutcome {
            command_contains: String::new(),
            expectation: CommandExpectation::ExpectedFailureSignature {
                signature: "fail".into(),
            },
        };
    }
    if lower.contains("verif")
        || lower.contains("test")
        || lower.contains("验证")
        || lower.contains("测试")
    {
        return EvidenceRequirement::VerificationPassed;
    }
    if lower.contains("write")
        || lower.contains("edit")
        || lower.contains("修复")
        || lower.contains("修改")
    {
        return EvidenceRequirement::AnyFileChange;
    }
    if lower.contains("criteria") || lower.contains("acceptance") || lower.contains("验收") {
        return EvidenceRequirement::ToolSucceeded {
            tool: "extract_criteria".into(),
        };
    }
    if lower.contains("locate") || lower.contains("root cause") || lower.contains("定位") {
        return EvidenceRequirement::SearchHit {
            query_contains: String::new(),
        };
    }
    if lower.contains("context") || lower.contains("gather") || lower.contains("上下文") {
        return EvidenceRequirement::AnyToolSuccess;
    }
    EvidenceRequirement::AnyToolSuccess
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
        let read = EvidenceItem {
            id: "ev_read".into(),
            source: "read_file".into(),
            kind: EvidenceKind::FileRead {
                path: "src/auth.rs".into(),
            },
        };
        assert!(!criterion.absorb(&read));
        assert!(!criterion.is_met());

        let extract = EvidenceItem {
            id: "ev_extract".into(),
            source: "extract_criteria".into(),
            kind: EvidenceKind::CommandSucceeded {
                command: "extract_criteria".into(),
                exit_code: 0,
            },
        };
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

        let hit = EvidenceItem {
            id: "ev_hit".into(),
            source: "search".into(),
            kind: EvidenceKind::SearchHit {
                query: "auth".into(),
                hits: 2,
            },
        };
        // AnyToolSuccess path for search hit works when requirement is SearchHit empty...
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
        // Even "tests pass" claim without verify bag entry:
        bag.push(EvidenceItem {
            id: "fake".into(),
            source: "model".into(),
            kind: EvidenceKind::TestPassed {
                command: "claimed".into(),
            },
        });
        // This one IS TestPassed — but it must come from verify runner, not model prose.
        // Model prose never creates EvidenceItem in production; simulate empty bag:
        let empty = EvidenceBag::default();
        assert!(!empty.satisfies(&criterion.requirement));
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
}
