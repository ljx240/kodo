//! Functional probes that measure what the sandbox actually enforces.
//!
//! A capability is only worth reporting if it was *observed*. Two probes
//! matter, and both used to be approximated:
//!
//! * **Network** — "the sandbox blocks the connection" must not be inferred
//!   from an HTTP client that failed for an unrelated reason. The probe
//!   therefore starts a real TCP listener, proves that the *same* address is
//!   reachable **without** the sandbox, and only then concludes that the
//!   sandbox blocked it.
//! * **File read** — "the sandbox hides the file" must not be inferred from
//!   a command that exited non-zero for another reason. The probe reads the
//!   file outside the sandbox (it must succeed) and inside it (it must be
//!   refused).
//!
//! Both probes run a tiny helper program, `kodo-sandbox-probe`, that speaks
//! a two-word protocol and reports the outcome as an exit code. The helper
//! uses nothing but the standard library, so it adds no dependency to the
//! product and behaves identically on macOS and Linux.
//!
//! The classification is deliberately based on the raw operating-system
//! error number: `Errno::PermissionDenied` alone would miss a refused
//! socket and would happily call a broken probe "isolated".

use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The argument that turns a process into a probe helper.
pub const PROBE_FLAG: &str = "--kodo-sandbox-probe";

/// Exit code: the operation succeeded.
pub const EXIT_OK: i32 = 0;
/// Exit code: the peer actively refused the connection.
pub const EXIT_REFUSED: i32 = 10;
/// Exit code: the connection attempt timed out.
pub const EXIT_TIMEOUT: i32 = 11;
/// Exit code: the target does not exist.
pub const EXIT_NOT_FOUND: i32 = 12;
/// Exit code: the operating system refused the access.
pub const EXIT_DENIED: i32 = 13;
/// Exit code: the network is unreachable (no route, interface down).
pub const EXIT_UNREACHABLE: i32 = 15;
/// Exit code: any other failure. The message is written to stderr.
pub const EXIT_OTHER: i32 = 14;

// ── Raw error numbers, per platform. No `libc` dependency is needed for a
//    handful of constants. ──
#[cfg(target_os = "macos")]
mod errno {
    pub const EPERM: i32 = 1;
    pub const EACCES: i32 = 13;
    pub const ENOENT: i32 = 2;
    pub const ETIMEDOUT: i32 = 60;
    pub const ECONNREFUSED: i32 = 61;
    pub const ENETDOWN: i32 = 50;
    pub const ENETUNREACH: i32 = 51;
    pub const EHOSTUNREACH: i32 = 65;
}
#[cfg(target_os = "linux")]
mod errno {
    pub const EPERM: i32 = 1;
    pub const EACCES: i32 = 13;
    pub const ENOENT: i32 = 2;
    pub const ETIMEDOUT: i32 = 110;
    pub const ECONNREFUSED: i32 = 111;
    pub const ENETDOWN: i32 = 100;
    pub const ENETUNREACH: i32 = 101;
    pub const EHOSTUNREACH: i32 = 113;
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod errno {
    pub const EPERM: i32 = 1;
    pub const EACCES: i32 = 13;
    pub const ENOENT: i32 = 2;
    pub const ETIMEDOUT: i32 = 60;
    pub const ECONNREFUSED: i32 = 61;
    pub const ENETDOWN: i32 = 50;
    pub const ENETUNREACH: i32 = 51;
    pub const EHOSTUNREACH: i32 = 65;
}

/// What a probe observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The operation succeeded.
    Ok,
    /// The peer actively refused the connection.
    Refused,
    /// The operation timed out.
    Timeout,
    /// The target does not exist.
    NotFound,
    /// The operating system denied the access.
    Denied,
    /// The network is unreachable — no route, interface down.
    Unreachable,
    /// Any other failure.
    Other(String),
}

impl ProbeOutcome {
    /// The exit code this outcome maps to.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Ok => EXIT_OK,
            Self::Refused => EXIT_REFUSED,
            Self::Timeout => EXIT_TIMEOUT,
            Self::NotFound => EXIT_NOT_FOUND,
            Self::Denied => EXIT_DENIED,
            Self::Unreachable => EXIT_UNREACHABLE,
            Self::Other(_) => EXIT_OTHER,
        }
    }

    /// The outcome for a helper exit code.
    pub fn from_exit_code(code: i32, stderr: &str) -> Self {
        match code {
            EXIT_OK => Self::Ok,
            EXIT_REFUSED => Self::Refused,
            EXIT_TIMEOUT => Self::Timeout,
            EXIT_NOT_FOUND => Self::NotFound,
            EXIT_DENIED => Self::Denied,
            EXIT_UNREACHABLE => Self::Unreachable,
            _ => Self::Other(format!("probe exit {code}: {}", stderr.trim())),
        }
    }

    /// True when the access was blocked — refused, denied or simply not
    /// reachable. All three mean the same thing for a confinement probe.
    pub fn is_blocked(&self) -> bool {
        matches!(
            self,
            Self::Refused | Self::Denied | Self::NotFound | Self::Unreachable
        )
    }

    /// True when the probe itself could not produce a verdict.
    pub fn is_anomaly(&self) -> bool {
        matches!(self, Self::Other(_))
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Refused => "refused",
            Self::Timeout => "timeout",
            Self::NotFound => "not_found",
            Self::Denied => "denied",
            Self::Unreachable => "unreachable",
            Self::Other(_) => "other",
        }
    }
}

/// Classify a raw operating-system error number.
pub fn classify_errno(raw: Option<i32>) -> ProbeOutcome {
    match raw {
        None => ProbeOutcome::Other("未知错误".to_owned()),
        Some(code) => {
            if code == errno::ECONNREFUSED {
                ProbeOutcome::Refused
            } else if code == errno::ETIMEDOUT {
                ProbeOutcome::Timeout
            } else if code == errno::EPERM || code == errno::EACCES {
                ProbeOutcome::Denied
            } else if code == errno::ENETUNREACH
                || code == errno::ENETDOWN
                || code == errno::EHOSTUNREACH
            {
                ProbeOutcome::Unreachable
            } else if code == errno::ENOENT {
                ProbeOutcome::NotFound
            } else {
                ProbeOutcome::Other(format!("errno {code}"))
            }
        }
    }
}

/// Attempt a TCP connection and classify the result.
pub fn connect(host: &str, port: u16, timeout: Duration) -> ProbeOutcome {
    let addresses = match (host, port).to_socket_addrs() {
        Ok(addresses) => addresses.collect::<Vec<_>>(),
        Err(error) => {
            return ProbeOutcome::Other(format!("地址无法解析：{error}"));
        }
    };
    if addresses.is_empty() {
        return ProbeOutcome::Other("地址无法解析".to_owned());
    }
    let mut last = ProbeOutcome::Other("地址无法解析".to_owned());
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(_) => return ProbeOutcome::Ok,
            Err(error) => last = classify_errno(error.raw_os_error()),
        }
    }
    last
}

/// Read a file and classify the result.
pub fn read_path(path: &Path) -> ProbeOutcome {
    match std::fs::read(path) {
        Ok(_) => ProbeOutcome::Ok,
        Err(error) => classify_errno(error.raw_os_error()),
    }
}

/// Run the probe protocol for the given arguments.
///
/// Returns the exit code when `args` is a probe request, and `None` when the
/// arguments are something else — so a host program can call this at the
/// very top of `main` and simply continue when it returns `None`.
pub fn serve(args: &[String]) -> Option<i32> {
    let rest = args.strip_prefix(&[PROBE_FLAG.to_owned()])?;
    Some(run_command(rest))
}

/// Run the probe protocol for a dedicated helper binary.
///
/// The dedicated binary is *only* a probe helper, so its arguments are the
/// command itself — no magic flag is needed.
pub fn serve_direct(args: &[String]) -> i32 {
    run_command(args)
}

fn run_command(rest: &[String]) -> i32 {
    let outcome = match rest.first().map(String::as_str) {
        Some("connect") => {
            let host = rest.get(1).map(String::as_str).unwrap_or("127.0.0.1");
            let port = rest
                .get(2)
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(0);
            let millis = rest
                .get(3)
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(2_000);
            connect(host, port, Duration::from_millis(millis))
        }
        Some("read") => {
            let path = rest.get(1).map(String::as_str).unwrap_or_default();
            read_path(Path::new(path))
        }
        Some(other) => ProbeOutcome::Other(format!("未知探测命令：{other}")),
        None => ProbeOutcome::Other("缺少探测命令".to_owned()),
    };
    if let ProbeOutcome::Other(message) = &outcome {
        eprintln!("{message}");
    }
    outcome.exit_code()
}

/// The helper binary that runs the probe protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeHelper {
    program: PathBuf,
}
impl ProbeHelper {
    /// Locate the helper on this host.
    ///
    /// The lookup order is deliberate: an explicit override first, then the
    /// helper shipped next to the running program, then the one in the
    /// build directory, then the running program itself when it registered
    /// as a probe-capable host.
    pub fn locate() -> Option<Self> {
        if let Some(path) = std::env::var_os("KODO_SANDBOX_PROBE_BIN") {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Some(Self { program: path });
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            let mut candidates: Vec<PathBuf> = Vec::new();
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join(helper_name()));
                if let Some(parent) = dir.parent() {
                    candidates.push(parent.join(helper_name()));
                }
            }
            for candidate in candidates {
                if candidate.is_file() {
                    return Some(Self { program: candidate });
                }
            }
        }
        if let Some(path) = which(helper_name()) {
            return Some(Self { program: path });
        }
        if self_host_is_probe_capable() {
            if let Ok(exe) = std::env::current_exe() {
                return Some(Self { program: exe });
            }
        }
        None
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    /// The directory the helper lives in — a backend binds it into the
    /// sandbox so the helper stays executable there.
    pub fn directory(&self) -> Option<&Path> {
        self.program.parent()
    }
}

fn helper_name() -> &'static str {
    if cfg!(windows) {
        "kodo-sandbox-probe.exe"
    } else {
        "kodo-sandbox-probe"
    }
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

static SELF_PROBE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Runs the probe helper **inside** the sandbox.
///
/// Each backend implements this with its own confinement mechanism, so the
/// measurement logic below stays platform independent.
pub trait SandboxProbeRunner {
    /// Run the helper with `args` under confinement and report the outcome.
    ///
    /// An implementation must distinguish "the helper ran and was blocked"
    /// from "the helper never started" — the latter is a probe anomaly and
    /// must not be reported as a successful isolation.
    fn run_in_sandbox(&self, helper: &Path, args: &[std::ffi::OsString]) -> ProbeOutcome;
}

/// Run the helper outside any confinement.
fn run_unconfined(helper: &Path, args: &[std::ffi::OsString]) -> ProbeOutcome {
    let output = std::process::Command::new(helper).args(args).output().ok();
    match output {
        Some(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            ProbeOutcome::from_exit_code(output.status.code().unwrap_or(-1), &stderr)
        }
        None => ProbeOutcome::Other("探测辅助程序无法启动".to_owned()),
    }
}

/// The loopback addresses the network probe covers.
///
/// IPv6 is optional: a host without IPv6 support simply skips it, which is
/// why the list is a `Vec` rather than a fixed pair.
fn probe_addresses() -> Vec<(&'static str, std::net::SocketAddr)> {
    let mut addresses = Vec::new();
    if let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:0") {
        if let Ok(addr) = listener.local_addr() {
            addresses.push(("127.0.0.1", addr));
        }
    }
    if let Ok(listener) = std::net::TcpListener::bind("[::1]:0") {
        if let Ok(addr) = listener.local_addr() {
            addresses.push(("::1", addr));
        }
    }
    addresses
}

/// Measure whether the sandbox really blocks outbound connections.
///
/// The measurement always runs a **control** first: the same address must
/// be reachable *without* the sandbox. Only then does a blocked connection
/// inside the sandbox mean "the sandbox isolated it" — a broken probe or a
/// dead listener can never be mistaken for isolation.
pub fn measure_network(
    helper: &ProbeHelper,
    runner: &dyn SandboxProbeRunner,
) -> Result<(), String> {
    let addresses = probe_addresses();
    if addresses.is_empty() {
        return Err("无法监听回环端口，网络隔离无法验证".to_owned());
    }
    for (host, addr) in addresses {
        let port = addr.port();
        let listener = std::net::TcpListener::bind(addr)
            .map_err(|error| format!("无法监听回环端口：{error}"))?;
        let args = vec![
            std::ffi::OsString::from("connect"),
            std::ffi::OsString::from(host),
            std::ffi::OsString::from(port.to_string()),
            std::ffi::OsString::from("2000"),
        ];
        // Control: the listener must be reachable without confinement.
        let control = run_unconfined(helper.program(), &args);
        if control != ProbeOutcome::Ok {
            return Err(format!(
                "网络探测异常：沙箱外无法连接 {host}:{port}（{}），无法验证网络隔离",
                control.label()
            ));
        }
        // Measurement: the same address under confinement.
        let measured = runner.run_in_sandbox(helper.program(), &args);
        match measured {
            ProbeOutcome::Ok => {
                return Err(format!("沙箱内仍可连接 {host}:{port}，网络隔离未生效"));
            }
            outcome if outcome.is_anomaly() => {
                return Err(format!(
                    "网络探测异常：沙箱内探测未得出结论（{}），无法验证网络隔离",
                    outcome.label()
                ));
            }
            _ => {
                // Refused, denied, timed out or unreachable: the sandbox
                // stopped the connection.
                let _ = listener;
            }
        }
    }
    Ok(())
}

/// Measure whether the sandbox really hides a path from the confined
/// process.
///
/// The control read must succeed outside the sandbox; only a blocked read
/// inside it proves the boundary.
pub fn measure_read(
    helper: &ProbeHelper,
    runner: &dyn SandboxProbeRunner,
    path: &Path,
) -> Result<(), String> {
    let args = vec![
        std::ffi::OsString::from("read"),
        std::ffi::OsString::from(path),
    ];
    let control = run_unconfined(helper.program(), &args);
    if control != ProbeOutcome::Ok {
        return Err(format!(
            "读取探测异常：沙箱外无法读取 {}（{}），无法验证读取隔离",
            path.display(),
            control.label()
        ));
    }
    let measured = runner.run_in_sandbox(helper.program(), &args);
    match measured {
        ProbeOutcome::Ok => Err(format!("沙箱内仍可读取 {}", path.display())),
        outcome if outcome.is_anomaly() => Err(format!(
            "读取探测异常：沙箱内探测未得出结论（{}），无法验证读取隔离",
            outcome.label()
        )),
        _ => Ok(()),
    }
}
/// Declare that the running program can serve the probe protocol.
///
/// A host program calls this once at start-up and then calls [`serve`] with
/// its own arguments. The sandbox runtime may then re-invoke the host
/// program inside the sandbox instead of shipping a second binary.
pub fn register_self_probe() {
    SELF_PROBE.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn self_host_is_probe_capable() -> bool {
    SELF_PROBE.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_map_to_stable_exit_codes() {
        assert_eq!(ProbeOutcome::Ok.exit_code(), EXIT_OK);
        assert_eq!(ProbeOutcome::Refused.exit_code(), EXIT_REFUSED);
        assert_eq!(ProbeOutcome::Timeout.exit_code(), EXIT_TIMEOUT);
        assert_eq!(ProbeOutcome::Denied.exit_code(), EXIT_DENIED);
        assert_eq!(ProbeOutcome::NotFound.exit_code(), EXIT_NOT_FOUND);
        assert_eq!(ProbeOutcome::Unreachable.exit_code(), EXIT_UNREACHABLE);
    }

    #[test]
    fn exit_codes_round_trip() {
        for outcome in [
            ProbeOutcome::Ok,
            ProbeOutcome::Refused,
            ProbeOutcome::Timeout,
            ProbeOutcome::NotFound,
            ProbeOutcome::Denied,
            ProbeOutcome::Unreachable,
        ] {
            assert_eq!(
                ProbeOutcome::from_exit_code(outcome.exit_code(), ""),
                outcome
            );
        }
    }

    #[test]
    fn a_block_is_recognised() {
        assert!(ProbeOutcome::Refused.is_blocked());
        assert!(ProbeOutcome::Denied.is_blocked());
        assert!(ProbeOutcome::Unreachable.is_blocked());
        assert!(!ProbeOutcome::Ok.is_blocked());
        assert!(!ProbeOutcome::Timeout.is_blocked());
        assert!(ProbeOutcome::Other("x".into()).is_anomaly());
    }

    #[test]
    fn the_protocol_only_answers_probe_requests() {
        assert!(serve(&["kodo".to_owned()]).is_none());
        assert!(serve(&[PROBE_FLAG.to_owned()]).is_some());
    }

    #[test]
    fn errno_numbers_map_to_the_right_outcome() {
        assert_eq!(
            classify_errno(Some(errno::ECONNREFUSED)),
            ProbeOutcome::Refused
        );
        assert_eq!(classify_errno(Some(errno::EPERM)), ProbeOutcome::Denied);
        assert_eq!(classify_errno(Some(errno::EACCES)), ProbeOutcome::Denied);
        assert_eq!(
            classify_errno(Some(errno::ENETUNREACH)),
            ProbeOutcome::Unreachable
        );
        assert_eq!(classify_errno(Some(errno::ENOENT)), ProbeOutcome::NotFound);
        assert!(classify_errno(Some(9999)).is_anomaly());
    }

    #[test]
    fn a_connect_probe_reports_a_real_refusal() {
        // Bind and immediately drop a listener: the port is almost certainly
        // closed, so the connect must fail rather than succeed.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.local_addr().expect("addr").port()
        };
        let outcome = connect("127.0.0.1", port, Duration::from_millis(500));
        assert_ne!(outcome, ProbeOutcome::Ok, "{outcome:?}");
    }

    #[test]
    fn a_connect_probe_succeeds_against_a_live_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let outcome = connect("127.0.0.1", port, Duration::from_millis(2_000));
        assert_eq!(outcome, ProbeOutcome::Ok, "{outcome:?}");
    }

    #[test]
    fn a_read_probe_classifies_a_missing_file() {
        let outcome = read_path(Path::new("/definitely/not/here/kodo-sandbox-probe"));
        assert_eq!(outcome, ProbeOutcome::NotFound, "{outcome:?}");
    }
}
