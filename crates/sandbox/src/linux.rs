//! Linux backend: Bubblewrap (`bwrap`).
//!
//! Bubblewrap builds a mount namespace, so the read view is an *allow-list*
//! by construction: whatever is not bound simply does not exist inside the
//! sandbox. The user's home directory is not mounted, which makes the read
//! axis stronger than the macOS backend — but it also means the toolchain
//! caches have to be bound explicitly or builds break.
//!
//! `--unshare-all` gives fresh user, pid, network, ipc and uts namespaces:
//! no route, no DNS, no loopback service. The Docker socket and the SSH
//! agent socket live under `/var/run` and `/run`, which are masked with an
//! empty tmpfs.
//!
//! `bwrap` needs unprivileged user namespaces, which many hardened kernels
//! and container runtimes disable. The backend therefore probes the host
//! functionally; when the probe fails the backend reports `Unavailable` and
//! restricted runs are refused — never silently executed unconfined.
//!
//! # Home and toolchains
//!
//! The confined process gets a **private, writable** home (the session's
//! scratch directory), not the user's real one. The toolchain caches stay
//! readable — bound read-only at a fixed in-sandbox path — and the
//! environment points the tools at them, so `cargo`, `node`, `java` and
//! `python` keep working while the home itself stays empty and writable.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::backend::{PreparedCommand, SandboxBackend, SandboxError};
use crate::capabilities::{CapabilityReport, EnforcementLevel};
use crate::policy::SandboxPolicy;

/// System directories mounted read-only into the sandbox.
const SYSTEM_ROOTS: &[&str] = &[
    "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/etc", "/opt",
];

/// Paths below `/etc` that hold host secrets and are masked even though
/// `/etc` itself is readable.
const MASKED_ETC_PATHS: &[&str] = &["/etc/ssh", "/etc/ssl/private"];

/// Directories that are masked so host sockets stay invisible.
const MASKED_HOST_DIRS: &[&str] = &["/var/run", "/run", "/var/lock"];

/// Toolchain directories bound read-only into the sandbox, and the
/// environment variable that points a tool at the in-sandbox copy.
///
/// The variable is only set when the directory exists on the host, so a
/// missing toolchain never produces a dangling path.
const TOOLCHAIN_ROOTS: &[(&str, &str)] = &[
    (".cargo", "CARGO_HOME"),
    (".rustup", "RUSTUP_HOME"),
    (".nvm", "NVM_DIR"),
    (".fnm", "FNM_DIR"),
    (".volta", "VOLTA_HOME"),
    (".asdf", "ASDF_DIR"),
    (".mise", "MISE_DATA_DIR"),
    (".pyenv", "PYENV_ROOT"),
    (".sdkman", "SDKMAN_DIR"),
    (".m2", "M2_HOME"),
    (".gradle", "GRADLE_USER_HOME"),
    (".npm", "NPM_CONFIG_CACHE"),
    (".deno", "DENO_DIR"),
    (".bun", "BUN_INSTALL"),
    (".go", "GOPATH"),
    ("go", "GOROOT"),
];

/// Credential files that live *inside* a toolchain directory. They are
/// masked with an empty file so the toolchain stays usable while the
/// tokens stay unreadable.
const TOOLCHAIN_CREDENTIALS: &[&str] = &[
    ".cargo/credentials.toml",
    ".cargo/config.toml",
    ".m2/settings.xml",
    ".npmrc",
];

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
        // Per-axis enforcement floor: the policy promises no outbound
        // traffic, so a network isolation that could not be verified refuses
        // the run instead of quietly shrinking the boundary.
        let report = self.inspect();
        let required = policy.required_enforcement();
        if !report.capabilities().meets(&required) {
            let missing = report.capabilities().missing(&required);
            return Err(SandboxError::PartialEnforcement {
                reason: format!(
                    "本机强制能力为 {}，不满足策略要求（缺少：{}）",
                    report.summary(),
                    missing.join("、")
                ),
                capabilities: report.capabilities(),
            });
        }

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
        // Mask host sockets and host secrets.
        for dir in MASKED_HOST_DIRS {
            if Path::new(dir).exists() {
                push("--tmpfs");
                push(dir);
            }
        }
        for path in MASKED_ETC_PATHS {
            if Path::new(path).exists() {
                push("--tmpfs");
                push(path);
            }
        }
        push("--tmpfs");
        push("/tmp");

        // The in-sandbox home. It only holds the toolchain caches; the
        // process' own HOME is the private scratch directory, which the
        // runtime points at through the environment.
        let home = sandbox_home();
        push("--dir");
        push(&home);

        // Toolchain caches from the user's home, read-only.
        let host_home = std::env::var_os("HOME").map(PathBuf::from);
        let mut env: Vec<(String, String)> = Vec::new();
        if let Some(host_home) = host_home.as_ref() {
            for (relative, variable) in TOOLCHAIN_ROOTS {
                let path = host_home.join(relative);
                if !path.exists() {
                    continue;
                }
                let target = format!("{home}/{relative}");
                push("--ro-bind");
                push(&path.display().to_string());
                push(&target);
                env.push(((*variable).to_owned(), target));
            }
            // Credentials inside the toolchain caches stay unreadable.
            for relative in TOOLCHAIN_CREDENTIALS {
                let path = host_home.join(relative);
                if !path.exists() {
                    continue;
                }
                let target = format!("{home}/{relative}");
                push("--ro-bind");
                push("/dev/null");
                push(&target);
            }
            // Git's global configuration is not a secret and the repository
            // needs it for author information.
            let git_config = host_home.join(".gitconfig");
            if git_config.exists() {
                env.push((
                    "GIT_CONFIG_GLOBAL".to_owned(),
                    git_config.display().to_string(),
                ));
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

        push("--chdir");
        push(&sandbox_cwd(policy, cwd).display().to_string());

        let mut argv = bwrap_args;
        argv.push(program.as_os_str().to_os_string());
        argv.extend(args.iter().cloned());

        Ok(PreparedCommand {
            program: binary.clone(),
            args: argv,
            env,
            cwd: cwd.to_path_buf(),
            profile_path: None,
        })
    }
}

/// The in-sandbox home directory that holds the toolchain caches.
fn sandbox_home() -> String {
    format!("/home/{}", user_name())
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

    // ── Write confinement: allowed inside the root, denied outside it. ──
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

    // ── Read confinement: a path that is not bound must not be readable. ──
    //
    // The probe reads the file with a helper instead of inferring the
    // result from a command's exit status: the control read happens
    // *outside* the sandbox and must succeed, so a missing file can never
    // be mistaken for a working isolation.
    let helper = match helper() {
        Ok(helper) => helper,
        Err(error) => {
            return (
                Probe {
                    available: file_write != EnforcementLevel::Unsupported,
                    file_write,
                    file_read: EnforcementLevel::Unsupported,
                    network_egress: EnforcementLevel::Unsupported,
                    notes: vec![error],
                },
                Some(binary),
            )
        }
    };
    let runner = BubblewrapProbeRunner {
        binary: binary.clone(),
    };
    let file_read = match crate::probe::measure_read(&helper, &runner, &inside.join("secret.txt")) {
        Ok(()) => EnforcementLevel::Full,
        Err(error) => {
            return (
                Probe {
                    available: file_write != EnforcementLevel::Unsupported,
                    file_write,
                    file_read: EnforcementLevel::Unsupported,
                    network_egress: EnforcementLevel::Unsupported,
                    notes: vec![error, "bubblewrap 读取隔离未通过探测".to_owned()],
                },
                Some(binary),
            )
        }
    };

    // ── Network: a fresh network namespace must not reach a live loopback port. ──
    let network_egress = match crate::probe::measure_network(&helper, &runner) {
        Ok(()) => EnforcementLevel::Full,
        Err(error) => {
            return (
                Probe {
                    available: file_write != EnforcementLevel::Unsupported,
                    file_write,
                    file_read,
                    network_egress: EnforcementLevel::Unsupported,
                    notes: vec![error, "bubblewrap 网络隔离未通过探测".to_owned()],
                },
                Some(binary),
            )
        }
    };

    let mut notes = vec!["沙箱内可读：/etc、/var、/opt（只读），工具链缓存只读".to_owned()];
    if file_write == EnforcementLevel::Unsupported {
        notes.push("bubblewrap 写入限制探测失败".to_owned());
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

/// The probe helper. A host without it cannot verify the network and read
/// isolation, so the backend refuses instead of guessing.
fn helper() -> Result<crate::probe::ProbeHelper, String> {
    crate::probe::ProbeHelper::locate()
        .ok_or_else(|| "未找到沙箱探测辅助程序，无法验证隔离能力".to_owned())
}

fn run_bwrap(binary: &Path, args: &[&str]) -> bool {
    Command::new(binary)
        .args(args)
        .output()
        .map(|result| result.status.success())
        .unwrap_or(false)
}

/// Runs the probe helper inside a Bubblewrap namespace.
struct BubblewrapProbeRunner {
    binary: PathBuf,
}

impl crate::probe::SandboxProbeRunner for BubblewrapProbeRunner {
    fn run_in_sandbox(&self, helper: &Path, args: &[OsString]) -> crate::probe::ProbeOutcome {
        let mut bwrap: Vec<OsString> = Vec::new();
        let mut push = |value: &str| bwrap.push(OsString::from(value));
        push("--unshare-all");
        push("--die-with-parent");
        for root in ["/usr", "/bin", "/lib", "/lib64", "/etc"] {
            if Path::new(root).exists() {
                push("--ro-bind");
                push(root);
                push(root);
            }
        }
        push("--tmpfs");
        push("/tmp");
        // The helper must stay executable inside the namespace.
        push("--ro-bind");
        push(&helper.display().to_string());
        push(&helper.display().to_string());
        bwrap.push(OsString::from("--"));
        bwrap.push(helper.as_os_str().to_os_string());
        bwrap.extend(args.iter().cloned());

        let output = Command::new(&self.binary).args(&bwrap).output();
        match output {
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                let code = output.status.code().unwrap_or(-1);
                if code == 126 || code == 127 {
                    return crate::probe::ProbeOutcome::Other(format!(
                        "探测辅助程序无法在沙箱内启动（exit {code}）"
                    ));
                }
                crate::probe::ProbeOutcome::from_exit_code(code, &stderr)
            }
            Err(error) => crate::probe::ProbeOutcome::Other(format!("沙箱探测执行失败：{error}")),
        }
    }
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

    #[test]
    fn the_in_sandbox_home_is_a_fixed_path() {
        assert!(sandbox_home().starts_with("/home/"));
    }
}
