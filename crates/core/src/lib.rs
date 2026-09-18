//! Kodo Core.
//!
//! Headless agent runtime. It knows nothing about Tauri, React or transport,
//! so it stays free of dependencies.

pub mod session;
pub mod settings;
pub mod workspace;

mod line;

#[cfg(test)]
mod test_support;

/// Identity of the running core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreInfo {
    pub name: &'static str,
    pub version: &'static str,
}

/// Returns the identity of this core build.
pub const fn info() -> CoreInfo {
    CoreInfo {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_reports_this_crate() {
        assert_eq!(info().name, "kodo-core");
        assert_eq!(info().version, env!("CARGO_PKG_VERSION"));
    }
}
