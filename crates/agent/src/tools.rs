//! Local tools the agent can run. Commands always go through an explicit
//! approval path when the permission mode requires it; there is no ambient
//! "run anything" path from the model.
//!
//! Every spawned process goes through [`crate::process::ProcessRunner`]
//! (timeout + cancel + process-tree kill + output caps).

use crate::process::{EnvPolicy, ProcessOutcome, ProcessRunner, ProcessSpec, ProcessStatus};
use std::fs;
use std::path::{Path, PathBuf};

/// True when a command looks destructive or system-wide.
pub fn is_dangerous_command(command: &str) -> bool {
    dangerous_reason(command).is_some()
}

/// Human-readable reason a command is treated as dangerous, if any.
pub fn dangerous_reason(command: &str) -> Option<&'static str> {
    let lower = command.to_lowercase();
    let first = lower.split_whitespace().next().unwrap_or("");

    if first == "rm" || lower.starts_with("rm ") || lower.contains("\nrm ") {
        return Some("删除文件");
    }
    if lower.contains("sudo ") || lower.starts_with("sudo") {
        return Some("提权执行");
    }
    if lower.contains("chmod ") || lower.contains("chown ") {
        return Some("修改权限");
    }
    if (lower.contains("curl ") || lower.contains("wget "))
        && (lower.contains("|") || lower.contains(";"))
    {
        return Some("下载并管道执行");
    }
    if lower.contains("git push")
        || lower.contains("git reset --hard")
        || lower.contains("git clean -")
    {
        return Some("危险 git 操作");
    }
    if lower.contains("mkfs") || lower.contains("dd if=") || lower.contains(":(){") {
        return Some("磁盘或进程破坏");
    }
    if lower.contains("shutdown") || lower.contains("reboot") || lower.contains("kill -9") {
        return Some("系统关机/杀进程");
    }
    if lower.contains("> /") || lower.contains(">> /") {
        return Some("重定向到绝对路径");
    }
    if (first == "sh" || first == "bash" || first == "zsh")
        && (lower.contains(" -c ") || lower.starts_with("sh -c") || lower.starts_with("bash -c"))
    {
        return Some("嵌套 shell");
    }
    None
}

pub fn search_files(project: &Path, query: &str) -> String {
    if query.trim().is_empty() {
        return "空查询".to_owned();
    }

    // Quote the pattern as a single rg argument; still no shell metacharacters
    // beyond the ProcessRunner shell wrapper. Bounded + cancellable.
    let spec = ProcessSpec::shell(
        project,
        format!("rg -l --max-count 1 -- {}", shell_single_quote(query)),
    )
    .timeout(std::time::Duration::from_secs(15))
    .stdout_limit(32 * 1024)
    .stderr_limit(4 * 1024);
    let outcome = ProcessRunner::run(&spec, &|| true);

    if outcome.status == ProcessStatus::ExitSuccess {
        let stdout = outcome.stdout.clone();
        let all: Vec<&str> = stdout.lines().filter(|line| !line.is_empty()).collect();
        if all.is_empty() {
            return "0 处匹配".to_owned();
        }
        let total = all.len();
        let shown: Vec<String> = all.into_iter().take(12).map(str::to_owned).collect();
        let mut body = shown.join("\n");
        if total > 12 {
            body.push_str(&format!("\n… 共 {total} 个文件"));
        }
        return format!("{total} 个文件匹配：\n{body}");
    }

    let mut hits = Vec::new();
    walk(project, project, &mut hits, query, 0);
    if hits.is_empty() {
        "0 处匹配".to_owned()
    } else {
        format!("{} 处匹配：\n{}", hits.len(), hits.join("\n"))
    }
}

/// Minimal POSIX single-quote for embedding a literal in `sh -c`.
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn walk(root: &Path, dir: &Path, hits: &mut Vec<String>, query: &str, depth: usize) {
    if depth > 4 || hits.len() >= 12 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "target" | "dist") {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, hits, query, depth + 1);
        } else if name
            .to_ascii_lowercase()
            .contains(&query.to_ascii_lowercase())
        {
            if let Ok(rel) = path.strip_prefix(root) {
                hits.push(rel.display().to_string());
            }
        }
    }
}

pub fn read_text(path: &Path, max_lines: usize) -> String {
    let Ok(raw) = fs::read_to_string(path) else {
        return "(无法读取)".to_owned();
    };
    let lines: Vec<String> = raw.lines().take(max_lines).map(str::to_owned).collect();
    let total = raw.lines().count();
    let mut out = lines.join("\n");
    if total > max_lines {
        out.push_str(&format!("\n… 共 {total} 行"));
    }
    out
}

/// Outcome of a local command run (mapped from [`ProcessStatus`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcomeKind {
    Success,
    /// Exited non-zero.
    Failed,
    /// Wall-clock timeout hit; process tree killed.
    TimedOut,
    /// alive() went false / user stop; process tree killed.
    Cancelled,
    /// Spawn or collection error.
    Error,
}

impl From<ProcessStatus> for CommandOutcomeKind {
    fn from(status: ProcessStatus) -> Self {
        match status {
            ProcessStatus::ExitSuccess => Self::Success,
            ProcessStatus::ExitFailure => Self::Failed,
            ProcessStatus::Timeout => Self::TimedOut,
            ProcessStatus::Cancelled => Self::Cancelled,
            ProcessStatus::SpawnFailure => Self::Error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    pub kind: CommandOutcomeKind,
    pub output: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub cancelled: bool,
}

/// Default wall-clock timeout for agent shell commands (seconds).
pub const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 60;
/// Hard cap so a mis-set timeout cannot run forever.
pub const MAX_COMMAND_TIMEOUT_SECS: u64 = 600;

/// Redact secret-looking material from command output before it reaches the
/// model, the session log, or the trace UI. Never prints raw env dumps.
pub fn redact_secrets(text: &str) -> String {
    if text.is_empty() {
        return text.to_owned();
    }
    let mut out = text.to_owned();
    // Known token prefixes: keep prefix, mask the body.
    for pattern in [
        "sk-ant-",
        "sk-proj-",
        "ghp_",
        "gho_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "glpat-",
        "AIza",
        "sk-",
    ] {
        let mut search_from = 0;
        while let Some(rel) = out[search_from..].find(pattern) {
            let pos = search_from + rel;
            // Skip prefixes already masked (e.g. sk-ant-•••).
            let after = pos + pattern.len();
            if out[after..].starts_with('•') {
                search_from = after;
                continue;
            }
            let mut end = after;
            let bytes = out.as_bytes();
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'-' || bytes[end] == b'_')
            {
                end += 1;
            }
            if end > after {
                out.replace_range(after..end, "•••");
                search_from = after + "•••".len();
            } else {
                search_from = after;
            }
        }
    }
    // Bearer <token>
    let mut search_from = 0;
    while let Some(rel) = out[search_from..].find("Bearer ") {
        let pos = search_from + rel + 7;
        if out[pos..].starts_with('•') {
            search_from = pos;
            continue;
        }
        let mut end = pos;
        let bytes = out.as_bytes();
        while end < bytes.len() && !bytes[end].is_ascii_whitespace() && bytes[end] != b'"' {
            end += 1;
        }
        if end > pos {
            out.replace_range(pos..end, "•••");
            search_from = pos + "•••".len();
        } else {
            search_from = pos;
        }
    }
    redact_env_assignments(&out)
}

fn redact_env_assignments(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            if let Some(eq) = line.find('=') {
                let name = &line[..eq];
                let upper = name.to_ascii_uppercase();
                let sensitive = upper.ends_with("_TOKEN")
                    || upper.ends_with("_KEY")
                    || upper.ends_with("_SECRET")
                    || upper.ends_with("_PASSWORD")
                    || upper.ends_with("_CREDENTIAL")
                    || upper.ends_with("_CREDENTIALS")
                    || upper.contains("PASSWORD")
                    || upper.contains("AUTHORIZATION")
                    || upper.contains("APIKEY")
                    || upper.contains("API_KEY")
                    || upper == "TOKEN";
                if sensitive && line[eq + 1..].trim().len() > 3 {
                    format!("{name}=•••")
                } else {
                    line.to_owned()
                }
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Read a line range from a file with an output cap.
pub fn read_text_range(
    path: &Path,
    start_line: usize,
    end_line: usize,
    max_chars: usize,
) -> Result<String, String> {
    let start = start_line.max(1);
    let end = end_line.max(start);
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let total = raw.lines().count();
    let body: Vec<&str> = raw.lines().skip(start - 1).take(end - start + 1).collect();
    let mut out = body.join("\n");
    if total > end || start > 1 {
        out = format!(
            "\n… 文件共 {total} 行，显示 {start}-{} …\n{out}",
            end.min(total)
        );
    }
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars).collect();
        out.push_str("\n…");
    }
    Ok(out)
}

/// Runs `command` under a wall-clock timeout, polling `alive`.
/// Cancel and timeout are distinct from ordinary command failure.
pub fn command_output_interruptible(
    cwd: &Path,
    command: &str,
    alive: &dyn Fn() -> bool,
) -> (String, Option<i32>, bool) {
    let outcome = command_run(cwd, command, alive, DEFAULT_COMMAND_TIMEOUT_SECS);
    (outcome.output, outcome.exit_code, outcome.cancelled)
}

/// Full command runner — thin adapter over [`ProcessRunner`].
pub fn command_run(
    cwd: &Path,
    command: &str,
    alive: &dyn Fn() -> bool,
    timeout_secs: u64,
) -> CommandOutcome {
    let spec = ProcessSpec::shell(cwd, command)
        .timeout(std::time::Duration::from_secs(
            timeout_secs.clamp(1, MAX_COMMAND_TIMEOUT_SECS),
        ))
        .env(EnvPolicy::Inherit)
        .stdout_limit(48 * 1024)
        .stderr_limit(16 * 1024);
    let outcome = ProcessRunner::run(&spec, alive);
    map_process_outcome(outcome, timeout_secs)
}

fn map_process_outcome(outcome: ProcessOutcome, timeout_secs: u64) -> CommandOutcome {
    let mut text = outcome.combined_output();
    if let Some(note) = outcome.note_suffix() {
        if !text.is_empty() && text != "(无输出)" {
            text.push('\n');
        }
        text.push_str(&note);
    }
    if text.trim().is_empty() {
        text = "(无输出)".to_owned();
    }
    let mut text = redact_secrets(&text);
    if text.len() > 4000 {
        let head_cap = 2800;
        let mut head_end = head_cap;
        while head_end > 0 && !text.is_char_boundary(head_end) {
            head_end -= 1;
        }
        let tail_start_candidates = text.len().saturating_sub(1000);
        let mut tail_start = tail_start_candidates;
        while tail_start < text.len() && !text.is_char_boundary(tail_start) {
            tail_start += 1;
        }
        let head = &text[..head_end];
        let tail = &text[tail_start..];
        text = format!(
            "{head}\n… [output truncated: {} bytes total; showing head+tail] …\n{tail}",
            text.len()
        );
    }
    // Keep timeout note accurate when duration < full timeout (kill early).
    if outcome.timed_out {
        if let Some(pos) = text.rfind("[timed out after") {
            let rest = &text[pos..];
            if let Some(end) = rest.find(']') {
                let replacement = format!("[timed out after {timeout_secs}s; process tree killed]");
                text.replace_range(pos..pos + end + 1, &replacement);
            }
        }
    }

    CommandOutcome {
        kind: outcome.status.into(),
        output: text,
        exit_code: outcome.exit_code,
        duration_ms: outcome.duration_ms,
        timed_out: outcome.timed_out,
        cancelled: outcome.cancelled,
    }
}

/// Agent tools: list_files / search_text / find_symbol / read_range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoToolRequest {
    ListFiles {
        prefix: Option<String>,
        limit: usize,
    },
    SearchText {
        query: String,
    },
    FindSymbol {
        name: String,
    },
    FindReferences {
        name: String,
    },
    ReadRange {
        path: String,
        start_line: usize,
        end_line: usize,
    },
}

/// Full shell safety classification for approval UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRisk {
    pub dangerous: bool,
    pub reason: &'static str,
    pub network_sensitive: bool,
    pub destructive_git: bool,
    pub needs_approval_in_full: bool,
}

pub fn classify_command_risk(command: &str) -> CommandRisk {
    let lower = command.to_ascii_lowercase();
    let dangerous = dangerous_reason(command).is_some();
    let network_sensitive = lower.contains("curl ")
        || lower.contains("wget ")
        || lower.contains("npm install")
        || lower.contains("cargo install")
        || lower.contains("pip install")
        || lower.contains("git push")
        || lower.contains("git fetch")
        || lower.contains("git pull");
    let destructive_git = lower.contains("git reset --hard")
        || lower.contains("git clean -")
        || lower.contains("git push --force")
        || lower.contains("git push -f")
        || lower.contains("git checkout .")
        || lower.contains("git restore .");
    let reason = dangerous_reason(command).unwrap_or(if destructive_git {
        "破坏性 git 操作"
    } else if network_sensitive {
        "网络敏感命令"
    } else {
        "普通命令"
    });
    CommandRisk {
        dangerous,
        reason,
        network_sensitive,
        destructive_git,
        // Full mode still guards catastrophic actions.
        needs_approval_in_full: destructive_git
            || reason == "磁盘或进程破坏"
            || reason == "系统关机/杀进程"
            || reason == "提权执行",
    }
}

/// Resolves `relative` under `project`, refusing escapes outside the root.
pub fn resolve_in_project(project: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = relative.trim().trim_start_matches("./");
    if relative.is_empty() {
        return Err("empty path".to_owned());
    }
    if Path::new(relative).is_absolute() {
        return Err("absolute paths are not allowed".to_owned());
    }
    if relative.split('/').any(|part| part == "..") {
        return Err("path escapes the project root".to_owned());
    }
    let full = project.join(relative);
    // Canonicalize parent when possible to catch symlink escapes.
    if let Ok(canon) = full.canonicalize() {
        if let Ok(root) = project.canonicalize() {
            if !canon.starts_with(&root) {
                return Err("path escapes the project root".to_owned());
            }
        }
    }
    Ok(full)
}

/// Writes file content inside the project. Returns (path, added, removed) line delta
/// computed with a real LCS line diff (not a HashSet approximation).
pub fn write_project_file(
    project: &Path,
    relative: &str,
    content: &str,
) -> Result<(String, u32, u32), String> {
    let full = resolve_in_project(project, relative)?;
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let old = fs::read_to_string(&full).unwrap_or_default();
    let (added, removed) = crate::patch::line_diff_counts(&old, content);
    fs::write(&full, content).map_err(|e| e.to_string())?;
    Ok((relative.trim().to_owned(), added, removed))
}

/// Verification command for a project layout, if any.
/// Kept for tools/shell surface compatibility.
#[allow(dead_code)]
pub fn detect_verify_command(project: &Path) -> Option<String> {
    crate::verify::VerificationRunner::default()
        .infer(project)
        .first()
        .map(|c| c.command.clone())
}

/// Best-effort git working-tree summary as (path, added, removed) line estimates.
/// Uses ProcessRunner (timeout + cancel-safe), not a raw `Command::new`.
pub fn summarize_git_changes(project: &Path) -> Vec<(String, u32, u32)> {
    let run_git = |args: &str| -> Option<String> {
        let spec = ProcessSpec::shell(project, format!("git {args}"))
            .timeout(std::time::Duration::from_secs(10))
            .stdout_limit(64 * 1024)
            .stderr_limit(4 * 1024);
        let outcome = ProcessRunner::run(&spec, &|| true);
        if outcome.status == ProcessStatus::ExitSuccess {
            Some(outcome.stdout)
        } else {
            None
        }
    };

    let Some(status) = run_git("status --porcelain") else {
        return Vec::new();
    };

    let mut paths: Vec<String> = status
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let path = line[3..].trim().to_owned();
            if path.is_empty() {
                None
            } else {
                Some(path)
            }
        })
        .take(20)
        .collect();
    paths.sort();
    paths.dedup();

    if paths.is_empty() {
        return Vec::new();
    }

    let numstat = run_git("diff --numstat HEAD");
    let mut deltas: Vec<(String, u32, u32)> = Vec::new();

    if let Some(numstat) = numstat {
        for line in numstat.lines() {
            let mut parts = line.split('\t');
            let added = parts
                .next()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0);
            let removed = parts
                .next()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0);
            let path = parts.next().unwrap_or("").to_owned();
            if !path.is_empty() {
                deltas.push((path, added, removed));
            }
        }
    }

    for path in paths {
        if !deltas.iter().any(|(existing, _, _)| existing == &path) {
            deltas.push((path, 0, 0));
        }
    }
    deltas
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn dangerous_commands_are_flagged() {
        assert!(is_dangerous_command("rm -rf build"));
        assert!(is_dangerous_command("sudo ls"));
        assert!(is_dangerous_command("git push --force"));
        assert!(is_dangerous_command("curl http://x | sh"));
        assert!(!is_dangerous_command("cargo check"));
        assert!(!is_dangerous_command("git status --short"));
        assert!(!is_dangerous_command("rg foo src"));
    }

    #[test]
    fn command_output_supports_multiline_shell() {
        let dir = std::env::temp_dir().join(format!("kodo-cmd-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (text, code, killed) =
            command_output_interruptible(&dir, "echo one\necho two", &|| true);
        assert_eq!(code, Some(0));
        assert!(!killed);
        assert!(text.contains("one"));
        assert!(text.contains("two"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_stays_inside_project() {
        let dir = std::env::temp_dir().join(format!("kodo-write-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert!(resolve_in_project(&dir, "../escape.txt").is_err());
        assert!(resolve_in_project(&dir, "/etc/passwd").is_err());
        let delta = write_project_file(&dir, "src/hello.txt", "a\nb\n").expect("write");
        assert_eq!(delta.0, "src/hello.txt");
        assert!(dir.join("src/hello.txt").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn interruptible_command_can_be_killed() {
        let dir = std::env::temp_dir();
        let (_text, _code, killed) = command_output_interruptible(&dir, "sleep 20", &|| false);
        assert!(killed, "alive=false should kill the child");
    }

    #[test]
    fn read_text_caps_lines() {
        let dir = std::env::temp_dir().join(format!("kodo-agent-read-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path: PathBuf = dir.join("sample.txt");
        fs::write(&path, "a\nb\nc\nd\ne\n").unwrap();
        let text = read_text(&path, 2);
        assert!(text.contains("a"));
        assert!(text.contains("…"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_timeout_kills_and_is_distinct_from_failure() {
        let dir = std::env::temp_dir().join(format!("kodo_timeout_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let outcome = command_run(&dir, "sleep 5", &|| true, 1);
        assert!(outcome.timed_out);
        assert_eq!(outcome.kind, CommandOutcomeKind::TimedOut);
        assert!(outcome.output.contains("timed out") || outcome.output.contains("cancelled"));
        assert_ne!(outcome.kind, CommandOutcomeKind::Failed);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_cancel_kills_process_tree() {
        let dir = std::env::temp_dir().join(format!("kodo_cancel_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let marker = dir.join("grandchild.marker");
        // Shell starts a grandchild that would touch the marker after 3s.
        let script = format!("(sleep 2; echo done > '{}') & sleep 5", marker.display());
        let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let flag = alive.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            flag.store(false, std::sync::atomic::Ordering::SeqCst);
        });
        let outcome = command_run(
            &dir,
            &script,
            &move || alive.load(std::sync::atomic::Ordering::SeqCst),
            10,
        );
        assert!(outcome.cancelled);
        assert_eq!(outcome.kind, CommandOutcomeKind::Cancelled);
        std::thread::sleep(std::time::Duration::from_millis(2500));
        // Grandchild must not have completed if process-group kill worked.
        assert!(
            !marker.exists(),
            "grandchild survived cancel — process-group kill failed"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn successful_command_is_success() {
        let dir = std::env::temp_dir().join(format!("kodo_ok_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let outcome = command_run(&dir, "echo hello", &|| true, 5);
        assert_eq!(outcome.kind, CommandOutcomeKind::Success);
        assert_eq!(outcome.exit_code, Some(0));
        assert!(outcome.output.contains("hello"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nonzero_exit_is_failed_not_timeout() {
        let dir = std::env::temp_dir().join(format!("kodo_fail_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let outcome = command_run(&dir, "exit 3", &|| true, 5);
        assert_eq!(outcome.kind, CommandOutcomeKind::Failed);
        assert_eq!(outcome.exit_code, Some(3));
        assert!(!outcome.timed_out && !outcome.cancelled);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn redact_secrets_masks_tokens_and_env() {
        let raw = "Authorization: Bearer sk-abcdef1234567890\nMY_API_TOKEN=supersecretval\nGITHUB_TOKEN=ghp_abcdefghij\npassword=hunter2\nGREETING=hello";
        let clean = redact_secrets(raw);
        assert!(!clean.contains("supersecretval"), "{clean}");
        assert!(!clean.contains("ghp_abcdefghij"), "{clean}");
        assert!(!clean.contains("hunter2"), "{clean}");
        assert!(clean.contains("GREETING=hello"), "{clean}");
        assert!(clean.contains("•••"), "{clean}");
    }

    #[test]
    fn output_truncation_notices_the_model() {
        let dir = std::env::temp_dir().join(format!("kodo_trunc_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let outcome = command_run(&dir, "seq 1 5000", &|| true, 10);
        assert!(
            outcome.output.contains("output truncated"),
            "missing truncation notice: len={}",
            outcome.output.len()
        );
        assert!(
            outcome.output.len() < 6000,
            "cap failed: {}",
            outcome.output.len()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn destructive_git_always_needs_approval_even_in_full() {
        let risk = classify_command_risk("git reset --hard HEAD~1");
        assert!(risk.destructive_git);
        assert!(risk.needs_approval_in_full);
        let risk = classify_command_risk("rm -rf /tmp/x");
        assert!(risk.dangerous);
        let risk = classify_command_risk("git status --short");
        assert!(!risk.needs_approval_in_full);
        assert!(!risk.dangerous);
        assert!(!risk.destructive_git);
    }

    #[test]
    fn read_range_respects_bounds() {
        let dir = std::env::temp_dir().join(format!("kodo_readr_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let file = dir.join("x.rs");
        fs::write(&file, "a\nb\nc\nd\ne\n").unwrap();
        let text = read_text_range(&file, 2, 3, 1000).unwrap();
        assert!(text.contains("b") && text.contains("c"));
        assert!(!text.contains("\nd"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_command_run_maps_to_shared_runner() {
        let dir = std::env::temp_dir().join(format!("kodo_map_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let outcome = command_run(&dir, "echo via-runner", &|| true, 5);
        assert_eq!(outcome.kind, CommandOutcomeKind::Success);
        assert!(outcome.output.contains("via-runner"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_cancelled_task_is_not_success() {
        let dir = std::env::temp_dir().join(format!("kodo_nc_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let outcome = command_run(&dir, "sleep 30", &|| false, 10);
        assert_eq!(outcome.kind, CommandOutcomeKind::Cancelled);
        assert!(outcome.cancelled);
        assert_ne!(outcome.kind, CommandOutcomeKind::Success);
        let _ = fs::remove_dir_all(&dir);
    }
}
