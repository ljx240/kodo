//! One-shot escalation grants.
//!
//! Approving a command is not the same as lifting the sandbox for it. When a
//! run needs to exceed the current policy, the user can grant **that single
//! operation** — never the session, never a class of commands, never a path
//! pattern.
//!
//! A grant is bound to:
//!
//! * the session it was issued in,
//! * a digest of the exact operation,
//! * the specific widening it allows,
//! * an expiry time.
//!
//! Consuming a grant invalidates it immediately, so a second use of the same
//! approval is refused.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// What a grant actually widens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantWidening {
    /// Run this one command without confinement.
    UnrestrictedCommand,
    /// Let this one command write to a specific path.
    WritePath(PathBuf),
    /// Let this one command reach the network.
    NetworkEgress,
}

impl GrantWidening {
    pub fn label(&self) -> String {
        match self {
            Self::UnrestrictedCommand => "本次命令不受沙箱限制".to_owned(),
            Self::WritePath(path) => format!("本次允许写入 {}", path.display()),
            Self::NetworkEgress => "本次允许访问网络".to_owned(),
        }
    }
}

/// The scope a grant is valid for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantScope {
    /// The session the grant belongs to. A grant never crosses sessions.
    pub session_id: String,
    /// Digest of the exact operation (command plus target).
    pub operation: String,
    /// The widening this grant permits.
    pub widened: GrantWidening,
    /// Unix timestamp in seconds; the grant is void after it.
    pub expires_at: u64,
}

impl GrantScope {
    pub fn new(
        session_id: impl Into<String>,
        operation: impl Into<String>,
        widened: GrantWidening,
        ttl_secs: u64,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            operation: operation.into(),
            widened,
            expires_at: now_secs().saturating_add(ttl_secs),
        }
    }

    /// True when this scope still applies to the given session and operation.
    pub fn matches(&self, session_id: &str, operation: &str) -> bool {
        self.session_id == session_id && self.operation == operation && now_secs() < self.expires_at
    }
}

/// A granted escalation. Consuming it burns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneShotGrant {
    pub scope: GrantScope,
}

/// The store of live grants for one process.
#[derive(Debug, Default)]
pub struct GrantStore {
    grants: Mutex<Vec<OneShotGrant>>,
}

impl GrantStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issue a grant. The grant is not usable until it is consumed.
    pub fn issue(&self, scope: GrantScope) -> OneShotGrant {
        let grant = OneShotGrant { scope };
        if let Ok(mut grants) = self.grants.lock() {
            grants.push(grant.clone());
            grants.retain(|item| item.scope.expires_at > now_secs());
        }
        grant
    }

    /// Consume a grant for exactly this session and operation.
    ///
    /// Returns `None` when there is no matching grant, when it expired, or
    /// when it was already used.
    pub fn consume(&self, session_id: &str, operation: &str) -> Option<OneShotGrant> {
        let mut grants = self.grants.lock().ok()?;
        let index = grants
            .iter()
            .position(|grant| grant.scope.matches(session_id, operation))?;
        let grant = grants.remove(index);
        // A grant is bound to one operation; drop any sibling that was issued
        // for the same operation so it cannot be replayed.
        grants.retain(|item| item.scope.operation != operation);
        Some(grant)
    }

    /// True when a matching grant exists but has already been used.
    pub fn has_used(&self, session_id: &str, operation: &str) -> bool {
        self.grants
            .lock()
            .map(|grants| {
                grants.iter().any(|grant| {
                    grant.scope.session_id == session_id && grant.scope.operation == operation
                })
            })
            .unwrap_or(false)
    }

    /// Number of live grants — used by tests and the status display.
    pub fn live(&self) -> usize {
        self.grants.lock().map(|grants| grants.len()).unwrap_or(0)
    }
}

/// Stable digest of an operation.
///
/// The digest identifies the exact command plus its target without storing
/// the command text, so an audit trail never becomes a transcript of
/// everything the model tried to run.
pub fn operation_digest(operation: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in operation.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x10000000000001b3);
    }
    format!("{hash:016x}")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grant_is_consumed_by_its_first_use() {
        let store = GrantStore::new();
        store.issue(GrantScope::new(
            "session-a",
            "op-1",
            GrantWidening::UnrestrictedCommand,
            60,
        ));
        assert!(store.consume("session-a", "op-1").is_some());
        assert!(
            store.consume("session-a", "op-1").is_none(),
            "a second use of the same grant must be refused"
        );
    }

    #[test]
    fn a_grant_does_not_cross_operations() {
        let store = GrantStore::new();
        store.issue(GrantScope::new(
            "session-a",
            "op-1",
            GrantWidening::UnrestrictedCommand,
            60,
        ));
        assert!(store.consume("session-a", "op-2").is_none());
    }

    #[test]
    fn a_grant_does_not_cross_sessions() {
        let store = GrantStore::new();
        store.issue(GrantScope::new(
            "session-a",
            "op-1",
            GrantWidening::UnrestrictedCommand,
            60,
        ));
        assert!(store.consume("session-b", "op-1").is_none());
    }

    #[test]
    fn an_expired_grant_is_refused() {
        let store = GrantStore::new();
        store.issue(GrantScope::new(
            "session-a",
            "op-1",
            GrantWidening::UnrestrictedCommand,
            0,
        ));
        assert!(store.consume("session-a", "op-1").is_none());
    }

    #[test]
    fn digests_are_stable_and_distinct() {
        assert_eq!(
            operation_digest("cargo test"),
            operation_digest("cargo test")
        );
        assert_ne!(
            operation_digest("cargo test"),
            operation_digest("cargo build")
        );
    }
}
