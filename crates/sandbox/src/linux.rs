//! Linux backend: Bubblewrap (`bwrap`).
//!
//! Bubblewrap builds a mount namespace, so the read view is an *allow-list*
//! by construction: whatever is not bound simply does not exist inside the
//! sandbox. The user's home directory is not mounted, which makes the read
//! axis stronger than the macOS backend — but it also means the toolchain
//! caches have to be bound explicitly or builds break.
//!
//! `--unshare-net` gives a fresh, empty network namespace: no route, no
//! DNS, no loopback service. The Docker socket and the SSH agent socket live
//! under `/var/run` and `/run`, which are masked with an empty tmpfs.
//!
//! `bwrap` needs unprivileged user namespaces, which many hardened kernels
//! and container runtimes disable. The backend therefore probes the host
//! functionally; when the probe fails the backend reports `Unavailable` and
//! restricted runs are refused — never silently executed unconfined.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::backend::{PreparedCommand, SandboxBackend, SandboxError};
use crate::capabilities::{CapabilityReport, EnforcementLevel};
use crate::policy::SandboxPolicy;

/// System directories mounted read-only into the sandbox.
const SYSTEM_ROOTS: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/etc",
    "/opt",
    "/etc/alternatives",
];

/// Directories below the user's home that hold toolchains. They are bound
/// read-only so a build still finds `cargo`, `node` or `java`.
const TOOLCHAIN_ROOTS: &[&str] = &[
    ".cargo", ".rustup", ".nvm", ".fnm", ".volta", ".asdf", ".mise", ".pyenv", ".sdkman", ".go",
    "go", ".m2", ".gradle", ".npm", ".deno", ".bun", ".local",
];

/// Directories that are masked so host sockets stay invisible.
const MASKED_HOST_DIRS: &[&str] = &["/var/run", "/run", "/var/lock"];

#[derive(Debug, Clone)]
struct Probe {
    available: bool,
    file_write: EnforcementLevel,
    file_read: EnforcementLevel,
    network_egress: EnforcementLevel,
    notes: Vec<String>,
}

impl Probe {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            file_write: EnforcementLevel::Unsupported,
            file_read: EnforcementLevel::Unsupported,
            network_egress: EnforcementLevel::Unsupported,
            notes: vec![reason.into()],
        }
    }
}

/// The Linux confinement backend.
pub struct BubblewrapBackend {
    probe: &'static Probe,
    binary: Option<PathBuf>,
}

impl Default for BubblewrapBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl BubblewrapBackend {
    pub fn new() -> Self {
        let (probe, binary) = probe();
        Self { probe, binary }
    }

    pub fn is_available(&self) -> bool {
        self.probe.available
    }
}

impl SandboxBackend for BubblewrapBackend {
    fn id(&self) -> &'static str {
        "bubblewrap"
    }

    fn inspect(&self) -> CapabilityReport {
        CapabilityReport {
            backend: "bubblewrap".to_owned(),
            file_read: self.probe.file_read,
            file_write: self.probe.file_write,
            network_egress: self.probe.network_egress,
            notes: self.probe.notes.clone(),
        }
    }

    fn prepare(
        &self,
        program: &Path,
        args: &[OsString],
        policy: &SandboxPolicy,
        cwd: &Path,
    ) -> Result<PreparedCommand, SandboxError> {
        let binary = self
            .binary
            .as_ref()
            .ok_or_else(|| SandboxError::Unavailable {
                reason: self
                    .probe
                    .notes
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "未找到 bwrap".to_owned()),
            })?;
        if !self.probe.available {
            return Err(SandboxError::Unavailable {
                reason: self
                    .probe
                    .notes
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "bubblewrap 探测失败".to_owned()),
            });
        }
        if policy.network != crate::policy::NetworkPolicy::DenyAll {
            return Err(SandboxError::Unsupported {
                reason: "Bubblewrap 无法强制执行域名白名单".to_owned(),
            });
        }
        if policy.require_full_enforcement {
            let report = self.inspect();
            if !report.capabilities().is_full() {
                return Err(SandboxError::PartialEnforcement {
                    reason: format!("本机强制能力为 {}，不满足完整强制要求", report.summary()),
                    capabilities: report.capabilities(),
                });
            }
        }

        let mut argv: Vec<OsString> = vec![OsString::from("--")];
        let mut bwrap_args: Vec<OsString> = Vec::new();
        let mut push = |value: &str| bwrap_args.push(OsString::from(value));

        // Namespaces: a fresh user, pid, network, ipc and uts namespace.
        push("--unshare-all");
        push("--die-with-parent");
        push("--new-session");
        push("--proc");
        push("/proc");
        push("--dev");
        push("/dev");

        for root in SYSTEM_ROOTS {
            if Path::new(root).exists() {
                push("--ro-bind");
                push(root);
                push(root);
            }
        }
        if Path::new("/var").exists() {
            push("--ro-bind");
            push("/var");
            push("/var");
        }
        // Mask host sockets: the Docker daemon, the SSH agent and friends.
        for dir in MASKED_HOST_DIRS {
            if Path::new(dir).exists() {
                push("--tmpfs");
                push(dir);
            }
        }
        push("--tmpfs");
        push("/tmp");

        // Toolchain caches from the user's home, read-only.
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            for relative in TOOLCHAIN_ROOTS {
                let path = home.join(relative);
                if path.exists() {
                    let target = format!("/home/{}/{}", user_name(), relative);
                    push("--ro-bind");
                    push(&path.display().to_string());
                    push(&target);
                }
            }
        }

        // The workspace: writable in WorkspaceWrite mode, read-only otherwise.
        let workspace = &policy.workspace_root;
        if !workspace.as_os_str().is_empty() && workspace.exists() {
            if policy.workspace_writable() {
                push("--bind");
                push(&workspace.display().to_string());
                push(&workspace.display().to_string());
            } else {
                push("--ro-bind");
                push(&workspace.display().to_string());
                push(&workspace.display().to_string());
            }
        }

        // Writable scratch roots.
        for root in policy.effective_write_roots() {
            if root == *workspace {
                continue;
            }
            if root.exists() {
                push("--bind");
                push(&root.display().to_string());
                push(&root.display().to_string());
            }
        }

        // Git metadata and Kodo state stay read-only even when the workspace
        // itself is writable. A later mount masks the earlier one.
        for root in policy.denied_write_roots.iter() {
            if root.exists() && root.starts_with(workspace) {
                push("--ro-bind");
                push(&root.display().to_string());
                push(&root.display().to_string());
            }
        }

        // Home inside the sandbox points at a directory that is not mounted;
        // tools that need a writable home get one through the write roots.
        push("--setenv");
        push("HOME");
        push(&format!("/home/{}", user_name()));
        push("--chdir");
        push(&sandbox_cwd(policy, cwd).display().to_string());

        argv.splice(0..1, bwrap_args);
        argv.push(program.as_os_str().to_os_string());
        argv.extend(args.iter().cloned());

        Ok(PreparedCommand {
            program: binary.clone(),
            args: argv,
            env: Vec::new(),
            cwd: cwd.to_path_buf(),
            profile_path: None,
        })
    }
}

/// The working directory the confined child starts in.
///
/// Bubblewrap can only enter a directory that is mounted inside the sandbox,
/// so a cwd outside the workspace falls back to the workspace root.
fn sandbox_cwd(policy: &SandboxPolicy, cwd: &Path) -> PathBuf {
    let bound = cwd.starts_with(&policy.workspace_root)
        || policy
            .effective_write_roots()
            .iter()
            .any(|root| cwd.starts_with(root));
    if bound && cwd.exists() {
        cwd.to_path_buf()
    } else {
        policy.workspace_root.clone()
    }
}

/// The login name of the current user, used for the in-sandbox home path.
fn user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "kodo".to_owned())
}

/// Locate `bwrap` on `PATH`.
fn find_bwrap() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("bwrap");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn probe() -> (&'static Probe, Option<PathBuf>) {
    static PROBE: OnceLock<(Probe, Option<PathBuf>)> = OnceLock::new();
    let (probe, binary) = PROBE.get_or_init(run_probe);
    (probe, binary.clone())
}

fn run_probe() -> (Probe, Option<PathBuf>) {
    let Some(binary) = find_bwrap() else {
        return (Probe::unavailable("未找到 bwrap，Linux 沙箱不可用"), None);
    };
    let scratch = match crate::runtime::private_dir("probe") {
        Ok(dir) => dir,
        Err(error) => {
            return (
                Probe::unavailable(format!("无法创建探测目录：{error}")),
                Some(binary),
            )
        }
    };
    let inside = scratch.join("inside");
    let outside = scratch.join("outside");
    let _ = std::fs::create_dir_all(&inside);
    let _ = std::fs::create_dir_all(&outside);
    let _ = std::fs::write(inside.join("secret.txt"), "probe-secret");

    // The probe bind-mounts the scratch area and checks that a write inside
    // it succeeds while a write outside it fails.
    let write_ok = run_bwrap(
        &binary,
        &[
            "--unshare-all",
            "--die-with-parent",
            "--ro-bind",
            "/usr",
            "/usr",
            "--ro-bind",
            "/bin",
            "/bin",
            "--ro-bind",
            "/lib",
            "/lib",
            "--ro-bind",
            "/lib64",
            "/lib64",
            "--ro-bind",
            "/etc",
            "/etc",
            "--tmpfs",
            "/tmp",
            "--bind",
            &inside.display().to_string(),
            &inside.display().to_string(),
            "--",
            "/bin/sh",
            "-c",
            &format!("echo probe > {}/ok.txt", inside.display()),
        ],
    );
    let write_denied = run_bwrap(
        &binary,
        &[
            "--unshare-all",
            "--die-with-parent",
            "--ro-bind",
            "/usr",
            "/usr",
            "--ro-bind",
            "/bin",
            "/bin",
            "--ro-bind",
            "/lib",
            "/lib",
            "--ro-bind",
            "/lib64",
            "/lib64",
            "--ro-bind",
            "/etc",
            "/etc",
            "--tmpfs",
            "/tmp",
            "--bind",
            &inside.display().to_string(),
            &inside.display().to_string(),
            "--",
            "/bin/sh",
            "-c",
            &format!("echo probe > {}/blocked.txt", outside.display()),
        ],
    );
    let file_write = if write_ok && !write_denied && !outside.join("blocked.txt").exists() {
        EnforcementLevel::Full
    } else {
        EnforcementLevel::Unsupported
    };

    // Read confinement: a path that is not bound must not be readable.
    let read_denied = run_bwrap(
        &binary,
        &[
            "--unshare-all",
            "--die-with-parent",
            "--ro-bind",
            "/usr",
            "/usr",
            "--ro-bind",
            "/bin",
            "/bin",
            "--ro-bind",
            "/lib",
            "/lib",
            "--ro-bind",
            "/lib64",
            "/lib64",
            "--ro-bind",
            "/etc",
            "/etc",
            "--tmpfs",
            "/tmp",
            "--",
            "/bin/sh",
            "-c",
            &format!("cat {}/secret.txt", inside.display()),
        ],
    );
    let file_read = if read_denied {
        EnforcementLevel::Full
    } else {
        EnforcementLevel::Unsupported
    };

    // Network: a fresh network namespace must not reach a live loopback port.
    let network_egress = match probe_network(&binary) {
        Ok(true) => EnforcementLevel::Full,
        Ok(false) => EnforcementLevel::Unsupported,
        Err(error) => {
            return (
                Probe {
                    available: file_write != EnforcementLevel::Unsupported,
                    file_write,
                    file_read,
                    network_egress: EnforcementLevel::Unsupported,
                    notes: vec![error, "bwrap 存在但网络命名空间探测失败".to_owned()],
                },
                Some(binary),
            )
        }
    };

    let mut notes = Vec::new();
    if file_write == EnforcementLevel::Unsupported {
        notes.push("bubblewrap 写入限制探测失败".to_owned());
    }
    if file_read == EnforcementLevel::Unsupported {
        notes.push("bubblewrap 读取限制探测失败".to_owned());
    }

    let _ = std::fs::remove_dir_all(&scratch);
    (
        Probe {
            available: file_write != EnforcementLevel::Unsupported,
            file_write,
            file_read,
            network_egress,
            notes,
        },
        Some(binary),
    )
}

fn run_bwrap(binary: &Path, args: &[&str]) -> bool {
    Command::new(binary)
        .args(args)
        .output()
        .map(|result| result.status.success())
        .unwrap_or(false)
}

/// Verify that `--unshare-net` really blocks a live loopback connection.
fn probe_network(binary: &Path) -> Result<bool, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("无法监听回环端口：{error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("无法读取监听地址：{error}"))?
        .port();
    let _guard = listener;
    let connected = Command::new(binary)
        .args([
            "--unshare-all",
            "--die-with-parent",
            "--ro-bind",
            "/usr",
            "/usr",
            "--ro-bind",
            "/bin",
            "/bin",
            "--ro-bind",
            "/lib",
            "/lib",
            "--ro-bind",
            "/lib64",
            "/lib64",
            "--ro-bind",
            "/etc",
            "/etc",
            "--tmpfs",
            "/tmp",
            "--",
            "/bin/sh",
            "-c",
            &format!(
                "exec 3<>/dev/tcp/127.0.0.1/{port} 2>/dev/null && echo connected || echo blocked"
            ),
        ])
        .output()
        .map_err(|error| format!("网络探测执行失败：{error}"))?;
    let stdout = String::from_utf8_lossy(&connected.stdout);
    Ok(stdout.contains("blocked"))
}

/// A backend that confines nothing — used on platforms without a P0 backend.
pub struct UnavailableBackend {
    reason: &'static str,
}

impl UnavailableBackend {
    pub fn new(reason: &'static str) -> Self {
        Self { reason }
    }
}

impl SandboxBackend for UnavailableBackend {
    fn id(&self) -> &'static str {
        "none"
    }

    fn inspect(&self) -> CapabilityReport {
        CapabilityReport {
            backend: "none".to_owned(),
            file_read: EnforcementLevel::Unsupported,
            file_write: EnforcementLevel::Unsupported,
            network_egress: EnforcementLevel::Unsupported,
            notes: vec![match self.reason {
                "unsupported-platform" => "当前平台没有可用的沙箱后端".to_owned(),
                other => other.to_owned(),
            }],
        }
    }

    fn prepare(
        &self,
        _program: &Path,
        _args: &[OsString],
        _policy: &SandboxPolicy,
        _cwd: &Path,
    ) -> Result<PreparedCommand, SandboxError> {
        Err(SandboxError::Unavailable {
            reason: match self.reason {
                "unsupported-platform" => "当前平台没有可用的沙箱后端".to_owned(),
                other => other.to_owned(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_backend_always_refuses() {
        let backend = UnavailableBackend::new("unsupported-platform");
        let policy = SandboxPolicy::for_workspace("/tmp/kodo-sb-ws");
        let error = backend
            .prepare(
                Path::new("/bin/sh"),
                &[OsString::from("-c"), OsString::from("echo hi")],
                &policy,
                Path::new("/tmp"),
            )
            .expect_err("must refuse");
        assert_eq!(error.code(), "sandbox_unavailable");
    }

    #[test]
    fn unavailable_backend_reports_no_enforcement() {
        let backend = UnavailableBackend::new("unsupported-platform");
        let report = backend.inspect();
        assert_eq!(report.file_write, EnforcementLevel::Unsupported);
        assert_eq!(report.network_egress, EnforcementLevel::Unsupported);
    }
}
