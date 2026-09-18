//! Verification commands and failure compression for the repair loop.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Category of a verification command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyKind {
    Build,
    Typecheck,
    Test,
    Lint,
}

impl VerifyKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Typecheck => "typecheck",
            Self::Test => "test",
            Self::Lint => "lint",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyCommand {
    pub kind: VerifyKind,
    pub command: String,
    pub timeout_ms: u64,
}

impl VerifyCommand {
    pub fn test(cmd: impl Into<String>) -> Self {
        Self { kind: VerifyKind::Test, command: cmd.into(), timeout_ms: 120_000 }
    }
}

/// Compressed failure for the model / repair prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureReport {
    pub command: String,
    pub exit_code: Option<i32>,
    pub primary_error: String,
    pub relevant_files: Vec<String>,
}

impl FailureReport {
    pub fn to_prompt_block(&self) -> String {
        format!(
            "command: {}\nexit_code: {}\nprimary_error: {}\nrelevant_files:\n{}\n",
            self.command,
            self.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "none".into()),
            self.primary_error,
            if self.relevant_files.is_empty() {
                "  (none)".to_owned()
            } else {
                self.relevant_files
                    .iter()
                    .map(|f| format!("  {f}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyOutcome {
    pub command: VerifyCommand,
    pub exit_code: Option<i32>,
    pub ok: bool,
    pub timed_out: bool,
    /// Truncated raw output (for notes only).
    pub output_tail: String,
    pub failure: Option<FailureReport>,
}

/// Final verification status for the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalStatus {
    Verified,
    PartiallyVerified,
    VerificationFailed,
    NotVerified,
}

impl FinalStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Verified => "Verified",
            Self::PartiallyVerified => "Partially verified",
            Self::VerificationFailed => "Verification failed",
            Self::NotVerified => "Not verified",
        }
    }
}

/// Infers and runs build/typecheck/test/lint with timeouts.
pub struct VerificationRunner {
    pub default_timeout_ms: u64,
}

impl Default for VerificationRunner {
    fn default() -> Self {
        Self { default_timeout_ms: 90_000 }
    }
}

impl VerificationRunner {
    pub fn new(default_timeout_ms: u64) -> Self {
        Self { default_timeout_ms }
    }

    /// Infer candidate commands from project layout and package scripts.
    pub fn infer(&self, project: &Path) -> Vec<VerifyCommand> {
        let mut cmds: Vec<VerifyCommand> = Vec::new();
        let push = |cmds: &mut Vec<VerifyCommand>, kind: VerifyKind, command: String| {
            if !cmds.iter().any(|c| c.command == command) {
                cmds.push(VerifyCommand { kind, command, timeout_ms: self.default_timeout_ms });
            }
        };

        if project.join("Cargo.toml").is_file() {
            push(&mut cmds, VerifyKind::Build, "cargo check --quiet".into());
            push(&mut cmds, VerifyKind::Test, "cargo test --workspace --quiet".into());
        }

        if project.join("package.json").is_file() {
            let text = fs::read_to_string(project.join("package.json")).unwrap_or_default();
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(scripts) = value.get("scripts").and_then(|s| s.as_object()) {
                    // Prefer explicit script names.
                    let mapping = [
                        ("typecheck", VerifyKind::Typecheck),
                        ("type-check", VerifyKind::Typecheck),
                        ("tsc", VerifyKind::Typecheck),
                        ("build", VerifyKind::Build),
                        ("test", VerifyKind::Test),
                        ("lint", VerifyKind::Lint),
                    ];
                    for (name, kind) in mapping {
                        if scripts.contains_key(name) {
                            push(&mut cmds, kind, format!("npm run {name}"));
                        }
                    }
                }
            }
            // Fallbacks when scripts missing
            if !cmds.iter().any(|c| c.kind == VerifyKind::Test) {
                push(&mut cmds, VerifyKind::Test, "npm test --silent".into());
            }
        }

        if project.join("pyproject.toml").is_file() || project.join("pytest.ini").is_file() {
            push(&mut cmds, VerifyKind::Test, "python -m pytest -q".into());
        }

        // Cap: run most relevant first (test > typecheck > build > lint).
        cmds.sort_by_key(|c| match c.kind {
            VerifyKind::Test => 0,
            VerifyKind::Typecheck => 1,
            VerifyKind::Build => 2,
            VerifyKind::Lint => 3,
        });
        cmds.truncate(4);
        cmds
    }

    /// Run one command with timeout. `alive` can cancel earlier.
    pub fn run_one(
        &self,
        project: &Path,
        cmd: &VerifyCommand,
        alive: &dyn Fn() -> bool,
    ) -> VerifyOutcome {
        let timeout = Duration::from_millis(cmd.timeout_ms.max(1_000));
        let shell = if cfg!(windows) { "cmd" } else { "/bin/sh" };
        let mut child = Command::new(shell);
        if cfg!(windows) {
            child.arg("/C").arg(&cmd.command);
        } else {
            child.arg("-c").arg(&cmd.command);
        }
        let child = child
            .current_dir(project)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let began = Instant::now();
        let mut proc = match child.spawn() {
            Ok(p) => p,
            Err(e) => {
                let report = FailureReport {
                    command: cmd.command.clone(),
                    exit_code: Some(127),
                    primary_error: format!("spawn failed: {e}"),
                    relevant_files: vec![],
                };
                return VerifyOutcome {
                    command: cmd.clone(),
                    exit_code: Some(127),
                    ok: false,
                    timed_out: false,
                    output_tail: report.primary_error.clone(),
                    failure: Some(report),
                };
            }
        };

        let mut timed_out = false;
        let mut cancelled = false;
        loop {
            if !alive() {
                let _ = proc.kill();
                cancelled = true;
                break;
            }
            if began.elapsed() > timeout {
                let _ = proc.kill();
                timed_out = true;
                break;
            }
            match proc.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }

        let output = proc.wait_with_output().ok();
        let (stdout, stderr, code) = match output {
            Some(o) => {
                let so = String::from_utf8_lossy(&o.stdout).into_owned();
                let se = String::from_utf8_lossy(&o.stderr).into_owned();
                (so, se, o.status.code())
            }
            None => (String::new(), String::new(), None),
        };
        let mut combined = stdout;
        if !stderr.trim().is_empty() {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&stderr.trim());
        }
        if timed_out {
            combined.push_str(&format!(
                "\n[timed out after {}ms]",
                cmd.timeout_ms
            ));
        }
        if cancelled {
            combined.push_str("\n[cancelled]");
        }
        let ok = !timed_out && !cancelled && code == Some(0);
        let output_tail = tail_chars(&combined, 2500);
        let failure = if ok {
            None
        } else {
            Some(FailureAnalyzer::analyze(&cmd.command, code, &combined))
        };
        VerifyOutcome {
            command: cmd.clone(),
            exit_code: code,
            ok,
            timed_out,
            output_tail,
            failure,
        }
    }

    /// Run all inferred commands; stop early on first hard failure optional.
    pub fn run_all(
        &self,
        project: &Path,
        alive: &dyn Fn() -> bool,
        stop_on_fail: bool,
    ) -> Vec<VerifyOutcome> {
        let cmds = self.infer(project);
        let mut out = Vec::new();
        for cmd in cmds {
            if !alive() {
                break;
            }
            let outcome = self.run_one(project, &cmd, alive);
            let failed = !outcome.ok;
            out.push(outcome);
            if failed && stop_on_fail {
                break;
            }
        }
        out
    }

    /// Combine outcomes into a final status.
    pub fn final_status(outcomes: &[VerifyOutcome]) -> FinalStatus {
        if outcomes.is_empty() {
            return FinalStatus::NotVerified;
        }
        let passed = outcomes.iter().filter(|o| o.ok).count();
        let total = outcomes.len();
        if passed == total {
            FinalStatus::Verified
        } else if passed > 0 {
            FinalStatus::PartiallyVerified
        } else {
            FinalStatus::VerificationFailed
        }
    }
}

/// Compress command output into a small structured failure.
pub struct FailureAnalyzer;

impl FailureAnalyzer {
    pub fn analyze(command: &str, exit_code: Option<i32>, output: &str) -> FailureReport {
        let primary_error = primary_error(output);
        let relevant_files = relevant_files(output);
        FailureReport {
            command: command.to_owned(),
            exit_code,
            primary_error,
            relevant_files,
        }
    }
}

fn primary_error(output: &str) -> String {
    // Prefer structured compiler / test patterns.
    let patterns: [&str; 12] = [
        "error[E",
        "error:",
        "error TS",
        "FAILED",
        "AssertionError",
        "panicked at",
        "ERR!",
        "error: test failed",
        "test result: FAILED",
        "Traceback (most recent call last)",
        "Compile error",
        "failed to compile",
    ];
    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        for p in patterns {
            if t.contains(p) {
                return clip(t, 240);
            }
        }
    }
    // Fallback: first non-empty error-looking line or last line.
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    if let Some(last) = lines.last() {
        return clip(last.trim(), 240);
    }
    "(no error line found)".to_owned()
}

fn relevant_files(output: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in output.lines() {
        // path-like tokens with an extension
        for token in line.split_whitespace() {
            let clean = token.trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == ':' || c == ',');
            if looks_like_source_path(clean) && !files.iter().any(|f: &String| f == clean) {
                files.push(clean.to_owned());
            }
        }
        if files.len() >= 8 {
            break;
        }
    }
    files
}

fn looks_like_source_path(s: &str) -> bool {
    // Strip rustc/cargo style `:line:col` or `:line` suffixes.
    let mut s = s.trim();
    // e.g. src/lib.rs:10:5
    if let Some(idx) = s.find(".rs:") {
        s = &s[..idx + 3];
    } else if let Some(idx) = s.find(".ts:") {
        s = &s[..idx + 3];
    } else if let Some(idx) = s.find(".js:") {
        s = &s[..idx + 3];
    } else if let Some(idx) = s.find(".py:") {
        s = &s[..idx + 3];
    } else if s.contains(':') {
        // generic path:line
        let (head, tail) = s.rsplit_once(':').unwrap_or((s, ""));
        if tail.chars().all(|c| c.is_ascii_digit()) && head.contains('.') {
            s = head;
        }
    }
    if s.len() < 5 || s.len() > 200 || s.contains(' ') {
        return false;
    }
    if !(s.contains('/') || s.contains('\\')) {
        return false;
    }
    const EXT: [&str; 16] = [
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".java", ".json", ".toml", ".md",
        ".css", ".html", ".yml", ".yaml", ".toml",
    ];
    EXT.iter().any(|e| s.ends_with(e))
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut o: String = s.chars().take(max - 1).collect();
        o.push('…');
        o
    }
}

fn tail_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let skip = s.chars().count() - max;
    let mut out: String = s.chars().skip(skip).collect();
    out.insert_str(0, "…");
    out
}

/// Failure report helper re-export used by callers that only need detect.
pub fn detect_verify_command(project: &Path) -> Option<String> {
    VerificationRunner::default().infer(project).first().map(|c| c.command.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture_with_failing_test() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kodo-verify-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Tiny cargo project would be heavy; use package.json scripts instead.
        fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"typecheck":"node -e \"process.exit(1)\"","test":"node -e \"console.error('AssertionError: expected 1 got 2 at src/app.test.js'); process.exit(1)\"","lint":"node -e \"process.exit(0)\"","build":"node -e \"process.exit(0)\""}}"#,
        )
        .unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/app.test.js"), "// test\n").unwrap();
        dir
    }

    #[test]
    fn infer_reads_package_scripts() {
        let root = fixture_with_failing_test();
        let runner = VerificationRunner::new(5_000);
        let cmds = runner.infer(&root);
        assert!(cmds.iter().any(|c| c.kind == VerifyKind::Test && c.command.contains("test")));
        assert!(cmds.iter().any(|c| c.kind == VerifyKind::Typecheck));
        assert!(cmds.iter().any(|c| c.kind == VerifyKind::Build));
        assert!(cmds.iter().any(|c| c.kind == VerifyKind::Lint));
        for c in &cmds {
            assert!(c.timeout_ms > 0, "each command needs a timeout");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn infer_cargo_project() {
        let dir = std::env::temp_dir().join(format!("kodo-cv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let cmds = VerificationRunner::default().infer(&dir);
        assert!(cmds.iter().any(|c| c.command.contains("cargo test")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn analyzer_compresses_failure() {
        let out = r#"
error[E0308]: mismatched types
  --> src/lib.rs:10:5
   |
10 |     let x = 1;
   |             ^ expected `&str`
"#;
        let report = FailureAnalyzer::analyze("cargo check --quiet", Some(101), out);
        assert_eq!(report.exit_code, Some(101));
        assert_eq!(report.command, "cargo check --quiet");
        assert!(report.primary_error.contains("error[E0308]"), "{}", report.primary_error);
        assert!(
            report.relevant_files.iter().any(|f| f.contains("src/lib.rs")),
            "files={:?}",
            report.relevant_files
        );
        let block = report.to_prompt_block();
        assert!(block.contains("exit_code: 101"));
        assert!(block.contains("primary_error:"));
    }

    #[test]
    fn run_failing_command_produces_failure_report() {
        let root = fixture_with_failing_test();
        let runner = VerificationRunner::new(10_000);
        let cmds = runner.infer(&root);
        let test_cmd = cmds
            .iter()
            .find(|c| c.kind == VerifyKind::Test)
            .cloned()
            .expect("test cmd");
        let outcome = runner.run_one(&root, &test_cmd, &|| true);
        assert!(!outcome.ok);
        assert_ne!(outcome.exit_code, Some(0));
        let f = outcome.failure.expect("failure");
        assert!(!f.primary_error.is_empty());
        assert_eq!(f.command, test_cmd.command);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn timeout_marks_failed() {
        let dir = std::env::temp_dir().join(format!("kodo-to-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let runner = VerificationRunner::new(50);
        let cmd = VerifyCommand { kind: VerifyKind::Test, command: "sleep 2".into(), timeout_ms: 100 };
        let outcome = runner.run_one(&dir, &cmd, &|| true);
        assert!(outcome.timed_out || !outcome.ok);
        assert!(!outcome.ok);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn final_status_classification() {
        let pass = VerifyOutcome {
            command: VerifyCommand::test("true"),
            exit_code: Some(0),
            ok: true,
            timed_out: false,
            output_tail: String::new(),
            failure: None,
        };
        let fail = VerifyOutcome {
            command: VerifyCommand::test("false"),
            exit_code: Some(1),
            ok: false,
            timed_out: false,
            output_tail: String::new(),
            failure: None,
        };
        assert_eq!(VerificationRunner::final_status(&[]), FinalStatus::NotVerified);
        assert_eq!(
            VerificationRunner::final_status(&[pass.clone()]),
            FinalStatus::Verified
        );
        assert_eq!(
            VerificationRunner::final_status(&[pass.clone(), fail.clone()]),
            FinalStatus::PartiallyVerified
        );
        let fail2 = VerifyOutcome {
            command: VerifyCommand::test("false"),
            exit_code: Some(1),
            ok: false,
            timed_out: false,
            output_tail: String::new(),
            failure: None,
        };
        assert_eq!(
            VerificationRunner::final_status(&[fail2, fail]),
            FinalStatus::VerificationFailed
        );
        let _ = pass;
    }

    #[test]
    fn successful_command_is_ok() {
        let dir = std::env::temp_dir().join(format!("kodo-ok-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let runner = VerificationRunner::new(5_000);
        let cmd = VerifyCommand::test("true");
        let outcome = runner.run_one(&dir, &cmd, &|| true);
        assert!(outcome.ok);
        assert!(outcome.failure.is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
