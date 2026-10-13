//! Source-URI validation and helper shellouts.
//!
//! All shellouts go through this module so the rest of the crate never
//! reaches for `Command` directly. That makes it easy to mock, easy to
//! audit, and easy to forbid in the hot path.

use std::path::Path;
use std::process::Command;

use crate::error::{MemoryError, MemoryResult};

/// Allowed source-URI schemes. Anything else is rejected at the door — the
/// memory engine never reads files based on a user-supplied URI; the only
/// sources are session logs (kodo://) and git objects (git://).
pub const ALLOWED_SCHEMES: &[&str] = &["kodo://", "git://"];

pub fn validate_source_uri(uri: &str) -> MemoryResult<()> {
    let lower = uri.to_ascii_lowercase();
    if ALLOWED_SCHEMES.iter().any(|s| lower.starts_with(s)) {
        Ok(())
    } else {
        Err(MemoryError::InvalidEvent(format!(
            "source uri must start with one of {ALLOWED_SCHEMES:?}: {uri:?}"
        )))
    }
}

/// Find an executable on PATH. Returns the absolute path or `None`.
/// Cheap: walks PATH once, no allocations beyond the result string.
pub fn which(binary: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for entry in std::env::split_paths(&path) {
        let candidate = entry.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Run a command under a working directory and return trimmed stdout.
/// Stderr is captured but not returned; non-zero exit is `InvalidEvent` so
/// the caller can decide whether to degrade.
pub fn run_capture(cwd: &Path, program: &str, args: &[&str]) -> MemoryResult<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| MemoryError::InvalidEvent(format!("{program} launch failed: {e}")))?;
    if !output.status.success() {
        return Err(MemoryError::InvalidEvent(format!(
            "{program} {} exited with status {:?}",
            args.join(" "),
            output.status.code()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok(stdout.trim_end().to_owned())
}
