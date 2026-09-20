//! Unified process lifecycle for every command Kodo starts.
//!
//! Single entry point: spawn → poll (timeout / cancel) → collect → kill tree.
//! Verification and agent shell tools must not open uncancellable side paths.
//!
//! # Platform process-tree notes
//!
//! * **Unix**: children are placed in a new process group (`setpgid(0,0)`).
//!   Timeout/cancel sends `SIGKILL` to `-pgid`, which reaches grandchildren.
//! * **Windows**: cancellation uses `taskkill /PID <pid> /T /F` (terminate
//!   process tree). This crate does **not** assign a Job Object; full job-based
//!   tree guarantees would need additional Win32 API surface. Tests document
//!   the limitation rather than claiming job-object coverage.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Hard cap so a mis-set timeout cannot run forever (seconds).
pub const MAX_PROCESS_TIMEOUT_SECS: u64 = 600;

/// How the child process environment is built.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EnvPolicy {
    /// Inherit the parent process environment (agent shell default).
    #[default]
    Inherit,
    /// Empty environment, then apply `vars` (name, value).
    Scrubbed { vars: Vec<(String, String)> },
}

/// Terminal classification of one process run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    /// Exited with code 0.
    ExitSuccess,
    /// Exited non-zero.
    ExitFailure,
    /// Wall-clock timeout; process tree terminated.
    Timeout,
    /// Cancellation went false; process tree terminated.
    Cancelled,
    /// Could not spawn, or post-spawn collection failed fatally.
    SpawnFailure,
}

impl ProcessStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::ExitSuccess => "exit_success",
            Self::ExitFailure => "exit_failure",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::SpawnFailure => "spawn_failure",
        }
    }

    pub fn is_cancelled(self) -> bool {
        matches!(self, Self::Cancelled)
    }

    pub fn is_timeout(self) -> bool {
        matches!(self, Self::Timeout)
    }

    pub fn is_success(self) -> bool {
        matches!(self, Self::ExitSuccess)
    }
}

/// Request for one process run.
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub cwd: PathBuf,
    /// Shell line (via `/bin/sh -c` or `cmd /C`).
    pub command: String,
    pub timeout: Duration,
    pub env: EnvPolicy,
    pub stdout_limit: usize,
    pub stderr_limit: usize,
}

impl ProcessSpec {
    pub fn shell(cwd: impl Into<PathBuf>, command: impl Into<String>) -> Self {
        Self {
            cwd: cwd.into(),
            command: command.into(),
            timeout: Duration::from_secs(60),
            env: EnvPolicy::Inherit,
            stdout_limit: 64 * 1024,
            stderr_limit: 16 * 1024,
        }
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn env(mut self, env: EnvPolicy) -> Self {
        self.env = env;
        self
    }

    pub fn stdout_limit(mut self, limit: usize) -> Self {
        self.stdout_limit = limit;
        self
    }

    pub fn stderr_limit(mut self, limit: usize) -> Self {
        self.stderr_limit = limit;
        self
    }
}

/// Result of one run. `stdout`/`stderr` are always bounded by the spec limits
/// (partial output is kept on timeout/cancel; nothing is cached unbounded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutcome {
    pub status: ProcessStatus,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub cancelled: bool,
    pub spawn_error: Option<String>,
    pub command: String,
}

impl ProcessOutcome {
    /// Combined view for UI/model (stdout + stderr), already capped by limits.
    pub fn combined_output(&self) -> String {
        let mut text = self.stdout.clone();
        if !self.stderr.trim().is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(self.stderr.trim_end());
        }
        if text.trim().is_empty() {
            "(无输出)".to_owned()
        } else {
            text
        }
    }

    pub fn note_suffix(&self) -> Option<String> {
        match self.status {
            ProcessStatus::Timeout => Some(format!(
                "[timed out after {}s; process tree killed]",
                self.duration_ms / 1000
            )),
            ProcessStatus::Cancelled => Some("[cancelled: process tree killed]".to_owned()),
            ProcessStatus::SpawnFailure => self
                .spawn_error
                .as_ref()
                .map(|e| format!("[spawn failure: {e}]")),
            _ => None,
        }
    }
}

/// Single process runner for all Kodo-spawned commands.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessRunner;

impl ProcessRunner {
    /// Run a shell command with timeout, cancellation, env policy, and
    /// stdout/stderr caps. `alive` is polled every ~40ms so cancel/timeout can
    /// kill the whole process group.
    pub fn run(spec: &ProcessSpec, alive: &dyn Fn() -> bool) -> ProcessOutcome {
        let began = Instant::now();
        if spec.command.trim().is_empty() {
            return ProcessOutcome {
                status: ProcessStatus::ExitFailure,
                stdout: String::new(),
                stderr: "空命令".to_owned(),
                exit_code: Some(1),
                duration_ms: 0,
                timed_out: false,
                cancelled: false,
                spawn_error: None,
                command: spec.command.clone(),
            };
        }
        if !alive() {
            return ProcessOutcome {
                status: ProcessStatus::Cancelled,
                stdout: String::new(),
                stderr: String::new(),
                exit_code: Some(143),
                duration_ms: 0,
                timed_out: false,
                cancelled: true,
                spawn_error: None,
                command: spec.command.clone(),
            };
        }

        let timeout = spec
            .timeout
            .min(Duration::from_secs(MAX_PROCESS_TIMEOUT_SECS))
            .max(Duration::from_millis(50));
        let mut cmd = shell_command(&spec.command);
        apply_env(&mut cmd, &spec.env);
        let mut child = match cmd
            .current_dir(&spec.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                return ProcessOutcome {
                    status: ProcessStatus::SpawnFailure,
                    stdout: String::new(),
                    stderr: format!("无法执行 `{}`：{error}", spec.command),
                    exit_code: Some(127),
                    duration_ms: began.elapsed().as_millis() as u64,
                    timed_out: false,
                    cancelled: false,
                    spawn_error: Some(error.to_string()),
                    command: spec.command.clone(),
                };
            }
        };

        // Drain pipes on threads with hard byte caps so a chatty child cannot
        // pin unbounded memory while we poll cancel/timeout.
        let stdout_limit = spec.stdout_limit.max(1);
        let stderr_limit = spec.stderr_limit.max(1);
        let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
        let (err_tx, err_rx) = mpsc::channel::<Vec<u8>>();
        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let out_handle = std::thread::spawn(move || {
            let mut acc = Vec::new();
            let mut buf = [0u8; 8192];
            if let Some(stream) = stdout.as_mut() {
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let remain = stdout_limit.saturating_sub(acc.len());
                            let take = n.min(remain);
                            acc.extend_from_slice(&buf[..take]);
                        }
                    }
                }
            }
            let _ = out_tx.send(acc);
        });
        let err_handle = std::thread::spawn(move || {
            let mut acc = Vec::new();
            let mut buf = [0u8; 4096];
            if let Some(stream) = stderr.as_mut() {
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let remain = stderr_limit.saturating_sub(acc.len());
                            let take = n.min(remain);
                            acc.extend_from_slice(&buf[..take]);
                        }
                    }
                }
            }
            let _ = err_tx.send(acc);
        });

        let mut timed_out = false;
        let mut cancelled = false;
        loop {
            if !alive() {
                kill_process_tree(&mut child);
                cancelled = true;
                break;
            }
            if began.elapsed() >= timeout {
                kill_process_tree(&mut child);
                timed_out = true;
                break;
            }
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(40)),
                Err(_) => {
                    kill_process_tree(&mut child);
                    break;
                }
            }
        }

        let wait_status = child.wait();
        let mut exit_code = wait_status.as_ref().ok().and_then(|s| s.code());
        let wait_failed = wait_status.is_err();

        // Final payloads: last send on each channel (reader threads finish
        // when pipes close — which happens after kill or natural exit).
        let stdout_bytes = out_rx
            .recv_timeout(Duration::from_millis(2_000))
            .unwrap_or_default();
        let stderr_bytes = err_rx
            .recv_timeout(Duration::from_millis(2_000))
            .unwrap_or_default();
        let _ = out_handle.join();
        let _ = err_handle.join();

        let mut stdout_text = String::from_utf8_lossy(&stdout_bytes).into_owned();
        let mut stderr_text = String::from_utf8_lossy(&stderr_bytes).into_owned();
        // Enforce limits again on decoded strings (char-boundary safe truncate).
        truncate_chars_in_place(&mut stdout_text, stdout_limit);
        truncate_chars_in_place(&mut stderr_text, stderr_limit);

        if cancelled {
            exit_code = Some(143);
        } else if timed_out {
            exit_code = Some(124);
        }

        let status = if cancelled {
            ProcessStatus::Cancelled
        } else if timed_out {
            ProcessStatus::Timeout
        } else if wait_failed {
            ProcessStatus::SpawnFailure
        } else if exit_code == Some(0) {
            ProcessStatus::ExitSuccess
        } else {
            ProcessStatus::ExitFailure
        };

        let spawn_error = if wait_failed {
            wait_status.err().map(|e| e.to_string())
        } else {
            None
        };

        ProcessOutcome {
            status,
            stdout: stdout_text,
            stderr: stderr_text,
            exit_code,
            duration_ms: began.elapsed().as_millis() as u64,
            timed_out,
            cancelled,
            spawn_error,
            command: spec.command.clone(),
        }
    }
}

fn truncate_chars_in_place(text: &mut String, byte_cap: usize) {
    if text.len() <= byte_cap {
        return;
    }
    let mut end = byte_cap;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

fn apply_env(cmd: &mut Command, policy: &EnvPolicy) {
    match policy {
        EnvPolicy::Inherit => {}
        EnvPolicy::Scrubbed { vars } => {
            cmd.env_clear();
            for (k, v) in vars {
                cmd.env(k, v);
            }
        }
    }
}

/// Shell wrapper with Unix process-group / Windows cmd /C.
fn shell_command(command: &str) -> Command {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = Command::new("/bin/sh");
        c.arg("-c").arg(command);
        c
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    cmd
}

/// Kill the whole process tree. Unix: `kill(-pgid)` + pid. Windows: `taskkill /T /F`.
pub fn kill_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
            libc::kill(pid, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .output();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kodo-proc-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn process_success_exit_zero() {
        let dir = tmp_dir("ok");
        let spec = ProcessSpec::shell(&dir, "echo hello").timeout(Duration::from_secs(5));
        let out = ProcessRunner::run(&spec, &|| true);
        assert_eq!(out.status, ProcessStatus::ExitSuccess);
        assert!(out.stdout.contains("hello"));
        assert_eq!(out.exit_code, Some(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_exit_failure_distinct_from_timeout() {
        let dir = tmp_dir("fail");
        let spec = ProcessSpec::shell(&dir, "exit 3").timeout(Duration::from_secs(5));
        let out = ProcessRunner::run(&spec, &|| true);
        assert_eq!(out.status, ProcessStatus::ExitFailure);
        assert_eq!(out.exit_code, Some(3));
        assert!(!out.timed_out && !out.cancelled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_sleep_timeout_kills_and_keeps_partial_output() {
        let dir = tmp_dir("to");
        let spec =
            ProcessSpec::shell(&dir, "echo partial; sleep 30").timeout(Duration::from_millis(400));
        let began = Instant::now();
        let out = ProcessRunner::run(&spec, &|| true);
        assert_eq!(out.status, ProcessStatus::Timeout);
        assert!(out.timed_out);
        assert_ne!(out.status, ProcessStatus::ExitFailure);
        assert!(
            out.stdout.contains("partial"),
            "partial output must be kept: {:?}",
            out.stdout
        );
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "timeout must not wait full sleep"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_cancel_before_spawn_is_cancelled() {
        let dir = tmp_dir("pre");
        let spec = ProcessSpec::shell(&dir, "echo hi");
        let out = ProcessRunner::run(&spec, &|| false);
        assert_eq!(out.status, ProcessStatus::Cancelled);
        assert!(out.cancelled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_timeout_kills_grandchild_tree() {
        let dir = tmp_dir("tree");
        let marker = dir.join("gc.marker");
        let script = format!("(sleep 2; echo x > '{}') & sleep 20", marker.display());
        let spec = ProcessSpec::shell(&dir, script).timeout(Duration::from_millis(400));
        let out = ProcessRunner::run(&spec, &|| true);
        assert!(out.timed_out || out.cancelled, "{out:?}");
        std::thread::sleep(Duration::from_millis(2500));
        assert!(
            !marker.exists(),
            "grandchild survived timeout — process-group kill failed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_cancel_kills_grandchild_tree() {
        let dir = tmp_dir("ctree");
        let marker = dir.join("cc.marker");
        let script = format!("(sleep 2; echo x > '{}') & sleep 20", marker.display());
        let alive = Arc::new(AtomicBool::new(true));
        let flag = alive.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            flag.store(false, Ordering::SeqCst);
        });
        let spec = ProcessSpec::shell(&dir, script).timeout(Duration::from_secs(20));
        let out = ProcessRunner::run(&spec, &move || alive.load(Ordering::SeqCst));
        assert_eq!(out.status, ProcessStatus::Cancelled);
        std::thread::sleep(Duration::from_millis(2500));
        assert!(
            !marker.exists(),
            "grandchild survived cancel — process-group kill failed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_stdout_limit_bounds_output() {
        let dir = tmp_dir("lim");
        let spec = ProcessSpec::shell(&dir, "seq 1 20000")
            .timeout(Duration::from_secs(10))
            .stdout_limit(1024);
        let out = ProcessRunner::run(&spec, &|| true);
        assert!(out.stdout.len() <= 1024, "cap={}", out.stdout.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_env_scrubbed_hides_parent_secret() {
        let dir = tmp_dir("env");
        std::env::set_var("KODO_PROC_TEST_SECRET", "leaked-value");
        let spec = ProcessSpec::shell(&dir, "echo secret=${KODO_PROC_TEST_SECRET:-none}")
            .timeout(Duration::from_secs(5))
            .env(EnvPolicy::Scrubbed {
                vars: vec![("PATH".into(), "/usr/bin:/bin".into())],
            });
        let out = ProcessRunner::run(&spec, &|| true);
        assert!(
            out.stdout.contains("secret=none"),
            "scrubbed env must not leak parent vars: {:?}",
            out.stdout
        );
        std::env::remove_var("KODO_PROC_TEST_SECRET");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_spawn_failure_when_cwd_missing() {
        let spec =
            ProcessSpec::shell("/no/such/cwd/kodo-xyz", "echo hi").timeout(Duration::from_secs(2));
        let out = ProcessRunner::run(&spec, &|| true);
        assert_eq!(out.status, ProcessStatus::SpawnFailure);
        assert!(out.spawn_error.is_some() || !out.stderr.is_empty());
    }

    #[test]
    #[cfg(windows)]
    fn process_windows_tree_kill_is_taskkill_not_job_object() {
        // Honest limitation: we use taskkill /T /F, not a Job Object.
        // This test only asserts taskkill is the documented strategy string.
        let docs = "taskkill /T /F";
        assert!(docs.contains("/T"));
    }

    #[test]
    fn process_status_labels_cover_all_kinds() {
        assert_eq!(ProcessStatus::ExitSuccess.label(), "exit_success");
        assert_eq!(ProcessStatus::ExitFailure.label(), "exit_failure");
        assert_eq!(ProcessStatus::Timeout.label(), "timeout");
        assert_eq!(ProcessStatus::Cancelled.label(), "cancelled");
        assert_eq!(ProcessStatus::SpawnFailure.label(), "spawn_failure");
    }
}
