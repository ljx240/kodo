//! Approval decision type and the stable fingerprints shared by every shell.

use kodo_agent::StepKind;

/// One approval decision from the user. `AllowSession` remembers the command
/// fingerprint for the rest of the conversation (codex-style graduated allow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalChoice {
    Deny,
    AllowOnce,
    AllowSession,
}

impl ApprovalChoice {
    pub fn from_parts(approved: bool, session_wide: bool) -> Self {
        match (approved, session_wide) {
            (false, _) => Self::Deny,
            (true, true) => Self::AllowSession,
            (true, false) => Self::AllowOnce,
        }
    }

    pub fn allows(self) -> bool {
        matches!(self, Self::AllowOnce | Self::AllowSession)
    }

    /// The choice a surface must act on for the given step kind.
    ///
    /// A sandbox escalation is a one-shot decision by construction: it
    /// lifts the boundary for exactly one operation. `AllowSession` is
    /// therefore downgraded to `AllowOnce` — the session-allow cache must
    /// never widen a sandbox grant.
    pub fn for_kind(self, kind: StepKind) -> Self {
        match (self, kind) {
            (Self::AllowSession, StepKind::SandboxEscalation) => Self::AllowOnce,
            (choice, _) => choice,
        }
    }

    /// True when this choice may be remembered for the session.
    pub fn is_session_wide(self, kind: StepKind) -> bool {
        matches!(self.for_kind(kind), Self::AllowSession)
    }
}

/// Stable fingerprint for "this exact step" when the user picks Allow for session.
pub fn approval_fingerprint(kind: StepKind, command: &str) -> String {
    format!("{}::{command}", step_kind_label(kind))
}

pub fn step_kind_label(kind: StepKind) -> &'static str {
    match kind {
        StepKind::Reasoning => "thinking",
        StepKind::Search => "search",
        StepKind::FileRead => "read file",
        StepKind::Command => "run command",
        StepKind::ModelCall => "call model",
        StepKind::FileChange => "edit files",
        StepKind::AgentMessage => "draft answer",
        // Approval kinds that never gate (per `StepKind` doc), but still need a
        // stable label so `approval_fingerprint` keeps producing unique strings.
        StepKind::ExternalTool => "external tool",
        StepKind::WebSearch => "web search",
        StepKind::Thinking => "thinking",
        StepKind::Compaction => "compact history",
        StepKind::Failover => "failover",
        StepKind::PlanStep => "plan step",
        StepKind::Permission => "permission",
        StepKind::ContextBudget => "context budget",
        StepKind::SandboxEscalation => "sandbox escalation",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_match_the_step_kind_labels() {
        assert_eq!(
            approval_fingerprint(StepKind::Command, "rm -rf target"),
            "run command::rm -rf target"
        );
        assert_eq!(
            approval_fingerprint(StepKind::FileChange, "src/main.rs"),
            "edit files::src/main.rs"
        );
    }

    #[test]
    fn choice_helpers() {
        assert_eq!(
            ApprovalChoice::from_parts(false, true),
            ApprovalChoice::Deny
        );
        assert_eq!(
            ApprovalChoice::from_parts(true, true),
            ApprovalChoice::AllowSession
        );
        assert_eq!(
            ApprovalChoice::from_parts(true, false),
            ApprovalChoice::AllowOnce
        );
        assert!(ApprovalChoice::AllowOnce.allows());
        assert!(!ApprovalChoice::Deny.allows());
    }

    #[test]
    fn a_sandbox_escalation_is_never_remembered_for_the_session() {
        // "本会话允许" must not turn into a standing permission to run
        // outside the sandbox.
        let choice = ApprovalChoice::AllowSession.for_kind(StepKind::SandboxEscalation);
        assert_eq!(choice, ApprovalChoice::AllowOnce);
        assert!(!choice.is_session_wide(StepKind::SandboxEscalation));
        // Ordinary commands keep the graduated allow.
        assert!(ApprovalChoice::AllowSession.is_session_wide(StepKind::Command));
        assert_eq!(
            ApprovalChoice::AllowSession.for_kind(StepKind::Command),
            ApprovalChoice::AllowSession
        );
    }

    #[test]
    fn an_escalation_has_its_own_fingerprint_space() {
        let escalation = approval_fingerprint(StepKind::SandboxEscalation, "cargo build");
        let command = approval_fingerprint(StepKind::Command, "cargo build");
        assert_ne!(escalation, command);
        assert!(escalation.starts_with("sandbox escalation::"));
    }
}
