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
}
