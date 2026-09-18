//! Test-only helpers. Never compiled into the shipped core.
//!
//! Nothing here touches process-wide state such as `HOME`: the tests run in
//! parallel threads, so every function under test takes the paths it works on
//! and the environment is read only by the few outer wrappers in `settings`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A directory under the system temp dir, removed when it goes out of scope —
/// including while unwinding from a failed assertion.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before the epoch")
            .subsec_nanos();
        let unique = format!(
            "kodo-core-{tag}-{}-{}-{nanos}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("cannot create the temp dir");
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Creates a directory `name` inside, returning its path.
    pub fn dir(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).expect("cannot create the temp subdirectory");
        path
    }

    /// Creates a file `name` with `contents` inside, returning its path.
    pub fn file(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("cannot create the temp parent directory");
        }
        fs::write(&path, contents).expect("cannot write the temp file");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
