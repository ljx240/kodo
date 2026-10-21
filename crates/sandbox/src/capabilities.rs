//! What a sandbox backend actually enforces — reported, never assumed.
//!
//! File read, file write and network egress are *independent* axes. A backend
//! that confines writes perfectly but only hides `~/.ssh` is not a "full"
//! sandbox, and the UI must not claim it is. Every backend therefore
//! reports each axis separately as `Full`, `Partial` or `Unsupported`.

/// How completely one confinement axis is enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnforcementLevel {
    /// The axis is enforced by the operating system for every descendant
    /// process, including ones the sandboxed code spawns itself.
    Full,
    /// The axis is enforced for the cases that matter most, but a documented
    /// gap remains (for example: sensitive paths are hidden, the rest of the
    /// host is still readable).
    Partial,
    /// The axis cannot be enforced by this backend on this host.
    Unsupported,
}

impl EnforcementLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Partial => "partial",
            Self::Unsupported => "unsupported",
        }
    }

    pub fn is_full(self) -> bool {
        matches!(self, Self::Full)
    }

    /// The weaker of two levels — used when a policy needs a capability floor.
    pub fn weaker(self, other: Self) -> Self {
        if self.rank() <= other.rank() {
            self
        } else {
            other
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Unsupported => 0,
            Self::Partial => 1,
            Self::Full => 2,
        }
    }
}

/// The three confinement axes of one backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnforcementCapabilities {
    pub file_read: EnforcementLevel,
    pub file_write: EnforcementLevel,
    pub network_egress: EnforcementLevel,
}

impl EnforcementCapabilities {
    pub const fn full() -> Self {
        Self {
            file_read: EnforcementLevel::Full,
            file_write: EnforcementLevel::Full,
            network_egress: EnforcementLevel::Full,
        }
    }

    /// True only when every axis is fully enforced.
    pub fn is_full(&self) -> bool {
        self.file_read.is_full() && self.file_write.is_full() && self.network_egress.is_full()
    }

    /// The weakest axis — the number that decides whether a policy's
    /// enforcement floor can be met.
    pub fn weakest(&self) -> EnforcementLevel {
        self.file_read
            .weaker(self.file_write)
            .weaker(self.network_egress)
    }
}

/// What one backend reports about itself on this host, right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityReport {
    /// Backend identifier, e.g. `seatbelt` / `bubblewrap` / `none`.
    pub backend: String,
    pub file_read: EnforcementLevel,
    pub file_write: EnforcementLevel,
    pub network_egress: EnforcementLevel,
    /// Human-readable caveats. Never contains secrets.
    pub notes: Vec<String>,
}

impl CapabilityReport {
    pub fn capabilities(&self) -> EnforcementCapabilities {
        EnforcementCapabilities {
            file_read: self.file_read,
            file_write: self.file_write,
            network_egress: self.network_egress,
        }
    }

    /// One-line summary for the CLI banner and the GUI status row. The text
    /// is derived from the measured levels, never hard-coded per surface.
    pub fn summary(&self) -> String {
        format!(
            "{} (files: read={} write={}, network: {})",
            self.backend,
            self.file_read.label(),
            self.file_write.label(),
            self.network_egress.label()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weakest_axis_drives_the_floor() {
        let caps = EnforcementCapabilities {
            file_read: EnforcementLevel::Partial,
            file_write: EnforcementLevel::Full,
            network_egress: EnforcementLevel::Full,
        };
        assert_eq!(caps.weakest(), EnforcementLevel::Partial);
        assert!(!caps.is_full());
    }

    #[test]
    fn summary_comes_from_the_measured_levels() {
        let report = CapabilityReport {
            backend: "seatbelt".into(),
            file_read: EnforcementLevel::Partial,
            file_write: EnforcementLevel::Full,
            network_egress: EnforcementLevel::Full,
            notes: vec![],
        };
        assert_eq!(
            report.summary(),
            "seatbelt (files: read=partial write=full, network: full)"
        );
    }
}
