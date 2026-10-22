//! macOS backend: Seatbelt (`sandbox-exec`).
//!
//! `sandbox-exec` is deprecated by Apple, but it is still present and
//! functional on the supported macOS versions, and it is the only confinement
//! mechanism that needs no extra dependency. The backend therefore *probes*
//! the host instead of trusting that the binary exists: a host where the
//! profile language has changed reports `Unsupported` and restricted runs are
//! refused rather than silently executed unconfined.
//!
//! # Enforcement model
//!
//! * **Write** — strict allow-list. Every write is denied, then the
//!   workspace, the private scratch area and a few device nodes are re-allowed.
//!   Git metadata and Kodo state are denied *after* the allow-list, so they
//!   stay protected even though they live inside the workspace.
//! * **Read** — deny-list. The user's home directory is denied and only the
//!   workspace plus detected toolchain caches are re-allowed. This is
//!   reported as `Partial`: secrets stored outside those roots remain
//!   readable, and the report says so instead of claiming full isolation.
//! * **Network** — `deny network*`, verified against a live loopback socket,
//!   so DNS, IPv6, loopback and Unix sockets are all covered by one rule.
//!
//! Paths must be resolved before they enter a profile: Seatbelt matches the
//! canonical path, so `/tmp` and `/private/tmp` are different rules on macOS.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::backend::{PreparedCommand, SandboxBackend, SandboxError};
use crate::capabilities::{CapabilityReport, EnforcementLevel};
use crate::policy::{normalize_root, SandboxPolicy};

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Paths inside the user's home that hold secrets or private documents.
///
/// Read access to the whole home directory is denied; these entries document
/// what that denial is protecting, and they are re-denied explicitly when a
/// broader rule (the toolchain allow-list) would otherwise re-grant them.
const HOME_SENSITIVE: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".kube",
    ".docker",
    ".azure",
    ".gcloud",
    ".config",
    ".terraform.d",
    ".password-store",
    ".local/share/keyrings",
    ".zsh_history",
    ".bash_history",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".pgpass",
    "Library/Keychains",
    "Library/Application Support",
    "Documents",
    "Downloads",
    "Desktop",
    "Music",
    "Movies",
    "Pictures",
    "Public",
];

/// Home directories that hold toolchains and must stay readable for builds
/// to work. Everything else under the home directory stays unreadable.
const TOOLCHAIN_READ_ROOTS: &[&str] = &[
    ".cargo",
    ".rustup",
    ".nvm",
    ".fnm",
    ".volta",
    ".asdf",
    ".mise",
    ".pyenv",
    ".sdkman",
    ".go",
    "go",
    ".m2",
    ".gradle",
    ".npm",
    ".deno",
    ".bun",
    ".local",
    ".gitconfig",
    ".config/git",
    ".config/nvim",
    ".config/gh",
];

/// Credential files that live *inside* a toolchain directory and must be
/// re-denied after the toolchain allow-list re-grants it.
const TOOLCHAIN_CREDENTIALS: &[&str] = &[
    ".cargo/credentials.toml",
    ".cargo/config.toml",
    ".m2/settings.xml",
    ".gradle/gradle.properties",
    ".npmrc",
];

/// Device nodes a build legitimately writes to.
const WRITABLE_DEVICES: &[&str] = &[
    "/dev/null",
    "/dev/zero",
    "/dev/random",
    "/dev/urandom",
    "/dev/tty",
    "/dev/ptmx",
    "/dev/fd",
];

/// What a functional probe found on this host.
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

/// The macOS confinement backend.
pub struct SeatbeltBackend {
    probe: &'static Probe,
}

impl Default for SeatbeltBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SeatbeltBackend {
    pub fn new() -> Self {
        Self { probe: probe() }
    }

    /// True when this host can confine commands.
    pub fn is_available(&self) -> bool {
        self.probe.available
    }
}

impl SandboxBackend for SeatbeltBackend {
    fn id(&self) -> &'static str {
        "seatbelt"
    }

    fn inspect(&self) -> CapabilityReport {
        CapabilityReport {
            backend: "seatbelt".to_owned(),
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
        if !self.probe.available {
            return Err(SandboxError::Unavailable {
                reason: self
                    .probe
                    .notes
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "sandbox-exec 不可用".to_owned()),
            });
        }
        if policy.network != crate::policy::NetworkPolicy::DenyAll {
            return Err(SandboxError::Unsupported {
                reason: "macOS Seatbelt 无法强制执行域名白名单".to_owned(),
            });
        }
        // Per-axis enforcement floor. A policy that promises no outbound
        // traffic needs a *verified* network isolation; a weak backend is
        // refused instead of being used with a smaller boundary than the
        // policy promises.
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

        let profile = render_profile(policy);
        let profile_path = write_private_profile(&profile)?;
        let mut argv = vec![
            OsString::from("-f"),
            profile_path.as_os_str().to_os_string(),
            program.as_os_str().to_os_string(),
        ];
        argv.extend(args.iter().cloned());

        Ok(PreparedCommand {
            program: PathBuf::from(SANDBOX_EXEC),
            args: argv,
            env: Vec::new(),
            cwd: cwd.to_path_buf(),
            profile_path: Some(profile_path),
        })
    }
}

/// Render the Seatbelt profile for a policy.
///
/// Rule order matters: Seatbelt honours the *last* matching rule, so the
/// broad denials come first and the narrow grants come after them.
fn render_profile(policy: &SandboxPolicy) -> String {
    let mut out = String::new();
    out.push_str("(version 1)\n");
    out.push_str("(allow default)\n");

    // ── Write confinement: deny everything, then re-allow a short list. ──
    out.push_str("\n;; write allow-list\n");
    out.push_str("(deny file-write*)\n");
    for root in policy.effective_write_roots() {
        out.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            escape(&root)
        ));
    }
    for device in WRITABLE_DEVICES {
        out.push_str(&format!("(allow file-write* (subpath \"{device}\"))\n"));
    }
    // Protected paths are denied *after* the allow-list so a workspace-local
    // `.git` or `.kodo` cannot be written through the workspace grant.
    for root in policy.denied_write_roots.iter() {
        out.push_str(&format!(
            "(deny file-write* (subpath \"{}\"))\n",
            escape(root)
        ));
    }
    if !policy.workspace_writable() && !policy.workspace_root.as_os_str().is_empty() {
        out.push_str(&format!(
            "(deny file-write* (subpath \"{}\"))\n",
            escape(&policy.workspace_root)
        ));
    }

    // ── Read confinement: deny the home directory, re-allow the workspace
    //    and the toolchain caches the build needs. ──
    out.push_str("\n;; read deny-list\n");
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(home) = &home {
        let home = normalize_root(home);
        out.push_str(&format!(
            "(deny file-read* (subpath \"{}\"))\n",
            escape(&home)
        ));
        // Directory traversal is still allowed, otherwise the sandboxed
        // process cannot even reach the workspace it is confined to.
        out.push_str(&format!(
            "(allow file-read-metadata (subpath \"{}\"))\n",
            escape(&home)
        ));
        for relative in TOOLCHAIN_READ_ROOTS {
            let path = normalize_root(&home.join(relative));
            if path.exists() {
                out.push_str(&format!(
                    "(allow file-read* (subpath \"{}\"))\n",
                    escape(&path)
                ));
            }
        }
        for relative in TOOLCHAIN_CREDENTIALS {
            let path = normalize_root(&home.join(relative));
            out.push_str(&format!(
                "(deny file-read* (subpath \"{}\"))\n",
                escape(&path)
            ));
        }
        for relative in HOME_SENSITIVE {
            let path = normalize_root(&home.join(relative));
            out.push_str(&format!(
                "(deny file-read* (subpath \"{}\"))\n",
                escape(&path)
            ));
        }
    }
    for root in policy.allowed_read_roots.iter() {
        out.push_str(&format!(
            "(allow file-read* (subpath \"{}\"))\n",
            escape(root)
        ));
    }
    if !policy.workspace_root.as_os_str().is_empty() {
        out.push_str(&format!(
            "(allow file-read* (subpath \"{}\"))\n",
            escape(&policy.workspace_root)
        ));
    }
    for root in policy.denied_read_roots.iter() {
        out.push_str(&format!(
            "(deny file-read* (subpath \"{}\"))\n",
            escape(root)
        ));
    }

    // ── Network: no outbound traffic at all. One rule covers DNS, TCP,
    //    loopback and Unix sockets. ──
    out.push_str("\n;; no outbound network\n");
    out.push_str("(deny network*)\n");
    out
}

/// Escape a path for a Seatbelt profile string literal.
fn escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Write the profile to a private file the confined process cannot rewrite.
///
/// The file lives in the session's private runtime directory, which is not
/// part of the sandbox's write allow-list, so a hostile process cannot edit
/// the rules that confine it.
fn write_private_profile(profile: &str) -> Result<PathBuf, SandboxError> {
    let dir = crate::runtime::private_dir("profile").map_err(|error| SandboxError::Io {
        reason: error.to_string(),
    })?;
    let path = dir.join("profile.sb");
    std::fs::write(&path, profile).map_err(|error| SandboxError::Io {
        reason: format!("写入沙箱配置失败：{error}"),
    })?;
    set_private_mode(&path);
    Ok(path)
}

fn set_private_mode(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

/// Run the functional probe once per process.
fn probe() -> &'static Probe {
    static PROBE: OnceLock<Probe> = OnceLock::new();
    PROBE.get_or_init(run_probe)
}

fn run_probe() -> Probe {
    if !Path::new(SANDBOX_EXEC).exists() {
        return Probe::unavailable("未找到 sandbox-exec，macOS 沙箱不可用");
    }
    let scratch = match crate::runtime::private_dir("probe") {
        Ok(dir) => dir,
        Err(error) => return Probe::unavailable(format!("无法创建探测目录：{error}")),
    };
    // The probe paths must be resolved: Seatbelt matches the canonical path,
    // and the temp directory is a symlink (`/var` → `/private/var`).
    let inside = normalize_root(&scratch.join("inside"));
    let outside = normalize_root(&scratch.join("outside"));
    let _ = std::fs::create_dir_all(&inside);
    let _ = std::fs::create_dir_all(&outside);
    let _ = std::fs::write(inside.join("secret.txt"), "probe-secret");

    // ── Write confinement: allowed inside the root, denied outside it. ──
    let write_profile = format!(
        "(version 1)\n(allow default)\n(deny file-write*)\n(allow file-write* (subpath \"{}\"))\n",
        escape(&inside)
    );
    let allowed = run_sandbox(
        &write_profile,
        &format!("echo probe > {}/ok.txt", inside.display()),
    );
    let denied = run_sandbox(
        &write_profile,
        &format!("echo probe > {}/blocked.txt", outside.display()),
    );
    let file_write =
        if allowed.success() && denied.failed() && !outside.join("blocked.txt").exists() {
            EnforcementLevel::Full
        } else {
            EnforcementLevel::Unsupported
        };
    let mut notes = Vec::new();
    if file_write != EnforcementLevel::Full {
        notes.push(format!(
            "Seatbelt 写入限制探测失败（允许写入={}，拒绝写入={}）",
            allowed.success(),
            denied.failed()
        ));
    }

    // ── Read confinement: the denied path must stay unreadable. ──
    //
    // The control read (outside the sandbox) must succeed, otherwise the
    // probe would "prove" isolation for a file that was simply missing.
    let read_profile = format!(
        "(version 1)\n(allow default)\n(deny file-read* (subpath \"{}\"))\n",
        escape(&inside)
    );
    let file_read = match probe_read(&read_profile, &inside.join("secret.txt")) {
        Ok(()) => {
            // The deny-list model hides known sensitive roots but cannot
            // prove every secret on the host is covered, hence `partial`.
            EnforcementLevel::Partial
        }
        Err(error) => {
            notes.push(format!("Seatbelt 读取限制探测失败：{error}"));
            EnforcementLevel::Unsupported
        }
    };

    // ── Network confinement: a live loopback socket must stay unreachable. ──
    let network_egress = match probe_network() {
        Ok(()) => EnforcementLevel::Full,
        Err(error) => {
            notes.push(format!("网络隔离未通过探测：{error}"));
            EnforcementLevel::Unsupported
        }
    };

    let _ = std::fs::remove_dir_all(&scratch);

    Probe {
        available: file_write != EnforcementLevel::Unsupported,
        file_write,
        file_read,
        network_egress,
        notes,
    }
}

/// A sandboxed probe run.
struct ProbeRun {
    success: bool,
}

impl ProbeRun {
    fn success(&self) -> bool {
        self.success
    }
    fn failed(&self) -> bool {
        !self.success
    }
}

fn run_sandbox(profile: &str, command: &str) -> ProbeRun {
    let output = Command::new(SANDBOX_EXEC)
        .arg("-p")
        .arg(profile)
        .arg("/bin/sh")
        .arg("-c")
        .arg(command)
        .output();
    match output {
        Ok(result) => ProbeRun {
            success: result.status.success(),
        },
        Err(_) => ProbeRun { success: false },
    }
}

/// Verify that a denied path really is unreadable inside the sandbox.
///
/// The control read happens outside the sandbox and must succeed; only a
/// blocked read under the profile proves the boundary.
fn probe_read(profile: &str, target: &Path) -> Result<(), String> {
    let helper = crate::probe::ProbeHelper::locate()
        .ok_or_else(|| "未找到读取探测辅助程序，无法验证读取隔离".to_owned())?;
    crate::probe::measure_read(&helper, &SeatbeltReadRunner { profile }, target)
}

/// Runs the probe helper under a Seatbelt profile.
struct SeatbeltReadRunner<'a> {
    profile: &'a str,
}

impl crate::probe::SandboxProbeRunner for SeatbeltReadRunner<'_> {
    fn run_in_sandbox(&self, helper: &Path, args: &[OsString]) -> crate::probe::ProbeOutcome {
        let mut argv = vec![OsString::from("-p"), OsString::from(self.profile)];
        argv.push(helper.as_os_str().to_os_string());
        argv.extend(args.iter().cloned());
        let output = Command::new(SANDBOX_EXEC).args(&argv).output();
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
            Err(error) => crate::probe::ProbeOutcome::Other(format!("读取探测执行失败：{error}")),
        }
    }
}

/// Verify that `deny network*` really blocks a live loopback connection.
///
/// The measurement uses a real TCP listener and a real connect attempt. The
/// control run — the same address, without confinement — must succeed
/// first; only a blocked connect *inside* the sandbox proves the isolation.
/// A missing helper or an unreachable listener is reported as "cannot
/// verify" rather than as "isolated".
fn probe_network() -> Result<(), String> {
    let helper = crate::probe::ProbeHelper::locate()
        .ok_or_else(|| "未找到网络探测辅助程序，无法验证网络隔离".to_owned())?;
    crate::probe::measure_network(&helper, &SeatbeltProbeRunner)
}

/// Runs the probe helper under Seatbelt with the production network rule.
struct SeatbeltProbeRunner;

impl crate::probe::SandboxProbeRunner for SeatbeltProbeRunner {
    fn run_in_sandbox(&self, helper: &Path, args: &[OsString]) -> crate::probe::ProbeOutcome {
        // The probe profile allows everything except the network, so the
        // helper itself stays executable inside the sandbox.
        let profile = "(version 1)\n(allow default)\n(deny network*)\n";
        let mut argv = vec![OsString::from("-p"), OsString::from(profile)];
        argv.push(helper.as_os_str().to_os_string());
        argv.extend(args.iter().cloned());
        let output = Command::new(SANDBOX_EXEC).args(&argv).output();
        match output {
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                let code = output.status.code().unwrap_or(-1);
                // The helper never started (the launcher could not exec it):
                // that is a probe anomaly, not a blocked connection.
                if code == 126 || code == 127 {
                    return crate::probe::ProbeOutcome::Other(format!(
                        "探测辅助程序无法在沙箱内启动（exit {code}）"
                    ));
                }
                crate::probe::ProbeOutcome::from_exit_code(code, &stderr)
            }
            Err(error) => crate::probe::ProbeOutcome::Other(format!("网络探测执行失败：{error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_denies_writes_before_granting_them() {
        let policy =
            SandboxPolicy::for_workspace("/tmp/kodo-sb-ws").with_write_root("/tmp/kodo-sb-scratch");
        let profile = render_profile(&policy);
        let deny = profile.find("(deny file-write*)").expect("write denial");
        // The workspace path is normalized, so the rule matches the canonical
        // location (`/tmp` is a symlink to `/private/tmp` on macOS).
        let workspace = normalize_root(Path::new("/tmp/kodo-sb-ws"));
        let allow = profile
            .find(&format!(
                "(allow file-write* (subpath \"{}\"))",
                escape(&workspace)
            ))
            .expect("workspace grant");
        assert!(deny < allow, "deny must come before the grant:\n{profile}");
    }

    #[test]
    fn profile_denies_network() {
        let policy = SandboxPolicy::for_workspace("/tmp/kodo-sb-ws");
        let profile = render_profile(&policy);
        assert!(profile.contains("(deny network*)"), "{profile}");
    }

    #[test]
    fn read_only_mode_does_not_grant_the_workspace_write() {
        let policy = SandboxPolicy::for_workspace("/tmp/kodo-sb-ws")
            .with_mode(crate::policy::SandboxMode::ReadOnly);
        let profile = render_profile(&policy);
        let workspace = normalize_root(Path::new("/tmp/kodo-sb-ws"));
        assert!(
            !profile.contains(&format!(
                "(allow file-write* (subpath \"{}\"))",
                escape(&workspace)
            )),
            "read-only must not grant workspace writes:\n{profile}"
        );
    }

    #[test]
    fn profile_escapes_quotes_in_paths() {
        assert_eq!(escape(Path::new("/a/b\"c")), "/a/b\\\"c");
    }
}
