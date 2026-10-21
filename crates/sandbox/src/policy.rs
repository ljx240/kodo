//! The resolved, immutable sandbox policy.
//!
//! A policy is built once per session from user configuration and the
//! project root, then handed to a backend. It is never mutated by the model:
//! widening a policy requires an explicit, user-visible action outside the
//! agent loop (see `kodo_sandbox::grant`).
//!
//! Path handling matters more than it looks. Seatbelt and Bubblewrap match
//! on the *resolved* path, so `/tmp` (a symlink to `/private/tmp` on macOS)
//! and `/private/tmp` are different rules. Every root in a policy is
//! normalized through [`normalize_root`] before it reaches a backend.

use std::path::{Component, Path, PathBuf};

/// Execution space granted to untrusted project code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxMode {
    /// The host workspace is read-only. Only a private scratch area is
    /// writable, so tools that need a temp dir still work.
    ReadOnly,
    /// The host workspace may be written; everything else is confined.
    #[default]
    WorkspaceWrite,
    /// No confinement. Requires an explicit, user-visible confirmation and is
    /// recorded in the audit log as `sandbox_bypassed=true`.
    Unrestricted,
}

impl SandboxMode {
    /// Parse a user-supplied mode. Unknown values return `None` so the caller
    /// can fall back to the safe default with a diagnostic instead of
    /// silently widening the sandbox.
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "read-only" | "readonly" | "ro" => Some(Self::ReadOnly),
            "workspace-write" | "workspacewrite" | "ww" => Some(Self::WorkspaceWrite),
            "unrestricted" | "none" => Some(Self::Unrestricted),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::Unrestricted => "unrestricted",
        }
    }

    /// Short Chinese label for the CLI banner and the GUI status row.
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "工作区只读",
            Self::WorkspaceWrite => "工作区写入",
            Self::Unrestricted => "不受限",
        }
    }

    /// False only for [`SandboxMode::Unrestricted`].
    pub fn confines(self) -> bool {
        !matches!(self, Self::Unrestricted)
    }
}

/// Outbound network policy for untrusted code.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum NetworkPolicy {
    /// No outbound network at all. This is the P0 default and the only mode
    /// the shipped backends can enforce.
    #[default]
    DenyAll,
    /// Domain allow-list.
    ///
    /// P0 backends cannot enforce a domain allow-list: a proxy environment
    /// variable is advisory, and native clients ignore it. Backends reject
    /// this policy instead of pretending to honour it — see
    /// `kodo_sandbox::backend::SandboxError::Unsupported`.
    AllowDomains(Vec<String>),
}

/// The resolved policy for one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxPolicy {
    pub mode: SandboxMode,
    /// The project root. Empty when the session has no project.
    pub workspace_root: PathBuf,
    /// Extra roots the sandboxed process may read.
    pub allowed_read_roots: Vec<PathBuf>,
    /// Roots the sandboxed process may never read (credentials, keychains…).
    pub denied_read_roots: Vec<PathBuf>,
    /// Roots the sandboxed process may write.
    pub allowed_write_roots: Vec<PathBuf>,
    /// Roots the sandboxed process may never write (git metadata, Kodo state).
    pub denied_write_roots: Vec<PathBuf>,
    pub network: NetworkPolicy,
    /// When true, the backend must reach `EnforcementLevel::Full` on every
    /// axis or the run is refused. A partial backend may still be used when
    /// this is false, but the report says so out loud.
    pub require_full_enforcement: bool,
}

impl SandboxPolicy {
    /// Default policy for a project: `WorkspaceWrite`, no network, workspace
    /// as the only writable root.
    pub fn for_workspace(root: impl Into<PathBuf>) -> Self {
        let root = normalize_root(&root.into());
        Self {
            mode: SandboxMode::WorkspaceWrite,
            workspace_root: root.clone(),
            allowed_read_roots: Vec::new(),
            denied_read_roots: sensitive_read_roots(),
            allowed_write_roots: vec![root],
            denied_write_roots: Vec::new(),
            network: NetworkPolicy::DenyAll,
            require_full_enforcement: false,
        }
    }

    pub fn with_mode(mut self, mode: SandboxMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_read_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.allowed_read_roots.push(normalize_root(&path.into()));
        self
    }

    pub fn with_denied_read_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.denied_read_roots.push(normalize_root(&path.into()));
        self
    }

    pub fn with_write_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.allowed_write_roots.push(normalize_root(&path.into()));
        self
    }

    pub fn with_denied_write_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.denied_write_roots.push(normalize_root(&path.into()));
        self
    }

    pub fn with_network(mut self, network: NetworkPolicy) -> Self {
        self.network = network;
        self
    }

    pub fn requiring_full_enforcement(mut self) -> Self {
        self.require_full_enforcement = true;
        self
    }

    /// True when the mode confines the process at all.
    pub fn confines(&self) -> bool {
        self.mode.confines()
    }

    /// Write roots effective for this mode.
    ///
    /// `ReadOnly` keeps the workspace readable but not writable; only
    /// explicitly added scratch roots stay writable. `WorkspaceWrite` allows
    /// the workspace plus the extra write roots.
    pub fn effective_write_roots(&self) -> Vec<PathBuf> {
        match self.mode {
            SandboxMode::Unrestricted => Vec::new(),
            SandboxMode::ReadOnly => self
                .allowed_write_roots
                .iter()
                .filter(|root| !root.starts_with(&self.workspace_root))
                .cloned()
                .collect(),
            SandboxMode::WorkspaceWrite => self.allowed_write_roots.clone(),
        }
    }

    /// Whether the workspace itself is writable under this mode.
    pub fn workspace_writable(&self) -> bool {
        matches!(self.mode, SandboxMode::WorkspaceWrite)
    }

    /// Whether a confined process may write to `path`.
    ///
    /// This is the application-layer half of the boundary: the operating
    /// system enforces it for spawned processes, and the built-in file tools
    /// consult it so a write can never reach further than the sandbox would
    /// allow.
    pub fn allows_write(&self, path: &Path) -> bool {
        if !self.mode.confines() {
            return true;
        }
        if self
            .denied_write_roots
            .iter()
            .any(|root| is_within_resolved(root, path))
        {
            return false;
        }
        self.effective_write_roots()
            .iter()
            .any(|root| is_within_resolved(root, path))
    }

    /// Whether a confined process may read from `path`.
    pub fn allows_read(&self, path: &Path) -> bool {
        if !self.mode.confines() {
            return true;
        }
        !self
            .denied_read_roots
            .iter()
            .any(|root| is_within_resolved(root, path))
    }
}

/// Resolve a path to the form the OS sandbox will actually match.
///
/// Existing components are canonicalized (so symlinks are followed and
/// `/tmp` becomes `/private/tmp`); a non-existent tail is appended verbatim
/// so a path that will be created later still gets a stable rule.
pub fn normalize_root(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    let mut existing = absolute.as_path();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                existing = parent;
            }
            _ => break,
        }
    }
    let mut resolved = existing
        .canonicalize()
        .unwrap_or_else(|_| existing.to_path_buf());
    for part in tail.into_iter().rev() {
        resolved.push(part);
    }
    resolved
}

/// Normalize a path for *comparison* only — it must not require the path to
/// exist, and it must not follow a final symlink out of the workspace when
/// the caller is checking a write target.
pub fn normalize_for_compare(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// True when `candidate` is inside `root` (or is `root` itself).
pub fn is_within(root: &Path, candidate: &Path) -> bool {
    normalize_for_compare(candidate).starts_with(normalize_for_compare(root))
}

/// True when `candidate` is inside `root`, comparing *resolved* paths.
///
/// Roots are canonicalized when the policy is built, so a lexical comparison
/// would wrongly reject a candidate that only differs by a symlinked parent
/// (`/var` vs `/private/var` on macOS). Resolving both sides also makes a
/// symlink that points out of the workspace compare as "outside", which is
/// what the boundary is supposed to enforce.
pub fn is_within_resolved(root: &Path, candidate: &Path) -> bool {
    is_within(&normalize_root(root), &normalize_root(candidate))
}

/// Credential-bearing locations the sandboxed process must never read.
///
/// This is a deny-list, not a proof: a secret stored somewhere unusual is
/// still readable. The capability report states that limitation instead of
/// claiming the read axis is fully confined.
pub fn sensitive_read_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    let mut roots = Vec::new();
    let mut push = |path: PathBuf| {
        let normalized = normalize_root(&path);
        if !roots.contains(&normalized) {
            roots.push(normalized);
        }
    };

    // Credentials, keys and agent sockets.
    for relative in [
        ".ssh",
        ".aws",
        ".gnupg",
        ".kube",
        ".docker",
        ".azure",
        ".gcloud",
        ".config/gcloud",
        ".config/gh",
        ".config/huggingface",
        ".terraform.d",
        ".config/op",
        ".password-store",
        ".local/share/keyrings",
        ".netrc",
        ".npmrc",
        ".pypirc",
        ".cargo/credentials.toml",
        ".cargo/config.toml",
        ".zsh_history",
        ".bash_history",
        ".sh_history",
        ".pgpass",
        ".mcp.json",
    ] {
        push(home.join(relative));
    }
    // macOS keychains and the per-user application support directory that
    // holds GUI app secrets.
    push(home.join("Library/Keychains"));
    push(home.join("Library/Application Support"));
    push(home.join("Library/Containers/com.apple.TCC"));
    // Kodo's own runtime state: settings, credentials and session logs.
    push(home.join(".kodo"));
    push(PathBuf::from("/var/db"));
    push(PathBuf::from("/private/var/db"));
    roots
}

/// Locate the Git metadata that belongs to `workspace`.
///
/// A linked worktree stores a `.git` *file* pointing at
/// `<common>/worktrees/<name>`, and the real object store and refs live in the
/// common directory. Opening the workspace root for writes must not hand the
/// sandboxed process write access to that shared metadata, so both paths are
/// returned.
pub fn git_metadata_roots(workspace: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let dot_git = workspace.join(".git");
    let git_dir = if dot_git.is_dir() {
        Some(dot_git)
    } else if dot_git.is_file() {
        std::fs::read_to_string(&dot_git)
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.trim().strip_prefix("gitdir:"))
                    .map(|rest| {
                        let raw = PathBuf::from(rest.trim());
                        if raw.is_absolute() {
                            raw
                        } else {
                            workspace.join(raw)
                        }
                    })
            })
            .filter(|path| path.exists())
    } else {
        None
    };

    let Some(git_dir) = git_dir else {
        return roots;
    };
    let normalized = normalize_root(&git_dir);
    roots.push(normalized.clone());

    // `commondir` points at the shared metadata for linked worktrees.
    if let Ok(text) = std::fs::read_to_string(git_dir.join("commondir")) {
        let raw = PathBuf::from(text.trim());
        let common = if raw.is_absolute() {
            raw
        } else {
            git_dir.join(raw)
        };
        let normalized = normalize_root(&common);
        if !roots.contains(&normalized) {
            roots.push(normalized);
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_parsing_rejects_unknown_values() {
        assert_eq!(SandboxMode::parse("read-only"), Some(SandboxMode::ReadOnly));
        assert_eq!(
            SandboxMode::parse("Workspace_Write"),
            Some(SandboxMode::WorkspaceWrite)
        );
        assert_eq!(
            SandboxMode::parse("unrestricted"),
            Some(SandboxMode::Unrestricted)
        );
        assert_eq!(SandboxMode::parse("yolo"), None);
        assert_eq!(SandboxMode::parse(""), None);
    }

    #[test]
    fn default_mode_is_workspace_write() {
        assert_eq!(SandboxMode::default(), SandboxMode::WorkspaceWrite);
    }

    #[test]
    fn read_only_mode_drops_the_workspace_from_write_roots() {
        let policy = SandboxPolicy::for_workspace("/tmp/does-not-exist-ws")
            .with_mode(SandboxMode::ReadOnly)
            .with_write_root("/tmp/kodo-scratch");
        let roots = policy.effective_write_roots();
        assert!(
            !roots.iter().any(|root| root.ends_with("does-not-exist-ws")),
            "read-only must not keep the workspace writable: {roots:?}"
        );
        assert!(roots.iter().any(|root| root.ends_with("kodo-scratch")));
    }

    #[test]
    fn workspace_write_mode_keeps_the_workspace_writable() {
        let policy = SandboxPolicy::for_workspace("/tmp/ws-workspace");
        assert!(policy.workspace_writable());
        assert!(!policy.effective_write_roots().is_empty());
    }

    #[test]
    fn unrestricted_mode_writes_nothing_into_the_allow_list() {
        let policy = SandboxPolicy::for_workspace("/tmp/ws-unrestricted")
            .with_mode(SandboxMode::Unrestricted);
        assert!(policy.effective_write_roots().is_empty());
        assert!(!policy.confines());
    }

    #[test]
    fn comparison_normalizes_dot_segments() {
        assert!(is_within(Path::new("/a/b"), Path::new("/a/b/../b/c")));
        assert!(!is_within(Path::new("/a/b"), Path::new("/a/bc")));
    }

    #[test]
    fn sensitive_roots_include_ssh_and_kodo_state() {
        let roots = sensitive_read_roots();
        let rendered: Vec<String> = roots
            .iter()
            .map(|root| root.to_string_lossy().into_owned())
            .collect();
        assert!(
            rendered.iter().any(|path| path.ends_with("/.ssh")),
            "ssh keys must be denied: {rendered:?}"
        );
        assert!(
            rendered.iter().any(|path| path.ends_with("/.kodo")),
            "kodo runtime state must be denied: {rendered:?}"
        );
    }

    #[test]
    fn git_metadata_roots_resolve_a_plain_repository() {
        let dir = std::env::temp_dir().join(format!("kodo-sb-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let roots = git_metadata_roots(&dir);
        assert_eq!(roots.len(), 1, "{roots:?}");
        assert!(roots[0].ends_with(".git"), "{roots:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_metadata_roots_resolve_a_linked_worktree() {
        let base = std::env::temp_dir().join(format!("kodo-sb-wt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let common = base.join("main/.git");
        let worktree_git = common.join("worktrees/feature");
        std::fs::create_dir_all(&worktree_git).unwrap();
        std::fs::write(worktree_git.join("commondir"), "../..").unwrap();

        let tree = base.join("feature");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(
            tree.join(".git"),
            format!("gitdir: {}\n", worktree_git.display()),
        )
        .unwrap();

        let roots = git_metadata_roots(&tree);
        assert_eq!(roots.len(), 2, "{roots:?}");
        assert!(
            roots.iter().any(|root| root.ends_with("main/.git")),
            "common dir must be protected: {roots:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
