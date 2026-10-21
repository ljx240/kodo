//! Structured sandbox audit events.
//!
//! The audit trail answers three questions without ever recording a secret:
//! *what was confined*, *what was refused*, and *what was explicitly
//! escalated*. Events carry a digest of the operation rather than the
//! command text, and never carry environment variables, credentials or
//! network details.

use crate::capabilities::EnforcementCapabilities;
use crate::policy::SandboxMode;

/// What happened to a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxEventKind {
    /// The run was confined by the OS sandbox.
    Confined,
    /// The run was refused — the sandbox could not be established.
    Denied,
    /// The run used a one-shot escalation grant.
    Escalated,
    /// The run was executed without confinement by explicit user consent.
    Bypassed,
}

impl SandboxEventKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Confined => "confined",
            Self::Denied => "denied",
            Self::Escalated => "escalated",
            Self::Bypassed => "bypassed",
        }
    }
}

/// One audit record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxEvent {
    pub kind: SandboxEventKind,
    /// Backend that handled the run, e.g. `seatbelt`.
    pub backend: String,
    /// Sandbox mode in force.
    pub mode: SandboxMode,
    /// What the backend enforced.
    pub capabilities: EnforcementCapabilities,
    /// Digest of the operation — never the command text.
    pub operation: String,
    /// Short, already-redacted explanation. Never contains secrets.
    pub detail: Option<String>,
}

impl SandboxEvent {
    pub fn confined(
        backend: impl Into<String>,
        mode: SandboxMode,
        capabilities: EnforcementCapabilities,
        operation: impl Into<String>,
    ) -> Self {
        Self {
            kind: SandboxEventKind::Confined,
            backend: backend.into(),
            mode,
            capabilities,
            operation: operation.into(),
            detail: None,
        }
    }

    pub fn denied(
        backend: impl Into<String>,
        mode: SandboxMode,
        capabilities: EnforcementCapabilities,
        operation: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind: SandboxEventKind::Denied,
            backend: backend.into(),
            mode,
            capabilities,
            operation: operation.into(),
            detail: Some(detail.into()),
        }
    }

    pub fn escalated(
        backend: impl Into<String>,
        mode: SandboxMode,
        capabilities: EnforcementCapabilities,
        operation: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind: SandboxEventKind::Escalated,
            backend: backend.into(),
            mode,
            capabilities,
            operation: operation.into(),
            detail: Some(detail.into()),
        }
    }

    pub fn bypassed(
        backend: impl Into<String>,
        mode: SandboxMode,
        capabilities: EnforcementCapabilities,
        operation: impl Into<String>,
    ) -> Self {
        Self {
            kind: SandboxEventKind::Bypassed,
            backend: backend.into(),
            mode,
            capabilities,
            operation: operation.into(),
            detail: None,
        }
    }

    /// One-line rendering for the trace panel and the CLI log.
    pub fn summary(&self) -> String {
        let mut text = format!(
            "沙箱[{}] {} · {}",
            self.kind.label(),
            self.mode.label(),
            self.backend
        );
        if let Some(detail) = &self.detail {
            text.push_str(" · ");
            text.push_str(detail);
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_render_a_readable_summary() {
        let event = SandboxEvent::denied(
            "seatbelt",
            SandboxMode::WorkspaceWrite,
            EnforcementCapabilities::full(),
            "abc123",
            "沙箱不可用",
        );
        let summary = event.summary();
        assert!(summary.contains("denied"), "{summary}");
        assert!(summary.contains("工作区写入"), "{summary}");
        assert!(summary.contains("沙箱不可用"), "{summary}");
    }

    #[test]
    fn events_carry_no_command_text() {
        let event = SandboxEvent::confined(
            "seatbelt",
            SandboxMode::WorkspaceWrite,
            EnforcementCapabilities::full(),
            crate::grant::operation_digest("curl https://secret.example"),
        );
        assert!(!event.operation.contains("secret.example"));
    }
}
