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
    match classify_command_risk(command) {
        CommandRisk::ReadOnly => None,
        risk => Some(risk.reason()),
    }
}

/// Shell-safety severity for one command. Highest matching category wins.
///
/// Ordered from least to most severe so `max()` / comparisons work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandRisk {
    /// Inspection only (ls, cat inside project, git status, rg, cargo check…).
    ReadOnly,
    /// Mutates files under the workspace (write, patch, non-system rm, chmod file).
    FilesystemWrite,
    /// Network egress (curl, wget, git push/fetch/pull, http clients).
    Network,
    /// Package installers (npm/pip/cargo/brew/apt install…).
    PackageInstall,
    /// Process control (kill, pkill, taskkill, nested shell, stop services).
    ProcessControl,
    /// Handles credentials or dumps env/secrets (printenv-style, secret files).
    SensitiveData,
    /// History-rewriting / force git that can lose work.
    DestructiveGit,
    /// Disk wipe, privilege escalation, system shutdown, recursive delete of
    /// system roots — always requires approval, even in Full mode.
    Catastrophic,
}

impl CommandRisk {
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "ReadOnly",
            Self::FilesystemWrite => "FilesystemWrite",
            Self::Network => "Network",
            Self::PackageInstall => "PackageInstall",
            Self::ProcessControl => "ProcessControl",
            Self::SensitiveData => "SensitiveData",
            Self::DestructiveGit => "DestructiveGit",
            Self::Catastrophic => "Catastrophic",
        }
    }

    pub fn reason(self) -> &'static str {
        match self {
            Self::ReadOnly => "普通命令",
            Self::FilesystemWrite => "修改工作区文件",
            Self::Network => "网络敏感命令",
            Self::PackageInstall => "安装依赖包",
            Self::ProcessControl => "控制系统进程",
            Self::SensitiveData => "可能触碰敏感数据",
            Self::DestructiveGit => "破坏性 git 操作",
            Self::Catastrophic => "灾难性操作",
        }
    }

    pub fn is_catastrophic(self) -> bool {
        matches!(self, Self::Catastrophic)
    }

    /// Full mode still blocks catastrophic and history-destroying git.
    pub fn needs_approval_in_full(self) -> bool {
        matches!(self, Self::Catastrophic | Self::DestructiveGit)
    }

    /// Auto mode auto-runs only read-only commands; every filesystem mutation
    /// still requires an explicit approval, even when it stays in the project.
    pub fn needs_approval_in_auto(self) -> bool {
        !matches!(self, Self::ReadOnly)
    }
}

/// Minimal environment keys inherited for agent shell commands.
/// Everything else from the parent process is dropped (explicit allowlist).
pub const AGENT_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "TMPDIR",
    "TMP",
    "TEMP",
    "PWD",
    "CI",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "GOPATH",
    "GOROOT",
    "GOPROXY",
    "JAVA_HOME",
    "PYTHONPATH",
    "VIRTUAL_ENV",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
];

/// Build the child env for agent commands: clear parent, keep allowlist only.
pub fn agent_env_policy() -> EnvPolicy {
    let mut vars = Vec::new();
    for key in AGENT_ENV_ALLOWLIST {
        if let Ok(val) = std::env::var(key) {
            vars.push((key.to_string(), val));
        }
    }
    // Ensure a usable PATH even if the parent had none.
    if !vars.iter().any(|(k, _)| k == "PATH") {
        vars.push((
            "PATH".into(),
            "/usr/local/bin:/usr/bin:/bin".into(),
        ));
    }
    EnvPolicy::Scrubbed { vars }
}

/// Split a shell line into rough tokens (quotes preserved as content).
/// Not a full parser — enough to stop naive substring bypasses like
/// `VAR=x rm -rf /` or `foo; rm`.
fn tokenize_shell(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut chars = command.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(ch) = chars.next() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => cur.push(ch),
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    cur.push(ch);
                }
                '\\' => {
                    if let Some(next) = chars.next() {
                        cur.push(next);
                    }
                }
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        tokens.push(std::mem::take(&mut cur));
                    }
                }
                '|' | ';' | '&' | '>' | '<' => {
                    if !cur.is_empty() {
                        tokens.push(std::mem::take(&mut cur));
                    }
                    // collapse && || >>
                    if (ch == '|' || ch == '&' || ch == '>') && chars.peek() == Some(&ch) {
                        chars.next();
                        tokens.push(format!("{ch}{ch}"));
                    } else {
                        tokens.push(ch.to_string());
                    }
                }
                c => cur.push(c),
            },
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

/// Strip leading `VAR=value` assignments so the real command head is visible.
fn command_head_tokens(tokens: &[String]) -> Vec<String> {
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i].trim_start_matches(|c| c == '\'' || c == '"');
        if t.contains('=') && !t.starts_with('=') {
            let name_end = t.find('=').unwrap_or(0);
            let name = &t[..name_end];
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                i += 1;
                continue;
            }
        }
        break;
    }
    tokens[i.min(tokens.len())..].to_vec()
}

fn basename(tok: &str) -> String {
    let t = tok.trim_matches(|c| c == '\'' || c == '"');
    t.rsplit('/').next().unwrap_or(t).to_ascii_lowercase()
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

/// Structured truncation record for one command's combined output.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TruncationInfo {
    /// Pre-truncation size in bytes when known (None if cut off mid-stream
    /// before the true total was observed).
    pub original_size: Option<u64>,
    pub truncated: bool,
    pub kept_head: usize,
    pub kept_tail: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    pub kind: CommandOutcomeKind,
    pub output: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub cancelled: bool,
    pub truncation: TruncationInfo,
}

/// Default wall-clock timeout for agent shell commands (seconds).
pub const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 60;
/// Hard cap so a mis-set timeout cannot run forever.
pub const MAX_COMMAND_TIMEOUT_SECS: u64 = 600;

/// Redact secret-looking material from command output, errors, commands, and
/// anything else that reaches the model, session log, or UI. Never prints raw
/// env dumps. Applied at every boundary (tool result, trace, session, events).
pub fn redact_secrets(text: &str) -> String {
    if text.is_empty() {
        return text.to_owned();
    }
    let mut out = text.to_owned();
    out = redact_private_key_blocks(&out);
    out = redact_token_prefixes(&out);
    out = redact_bearer_and_basic(&out);
    out = redact_json_secrets(&out);
    out = redact_url_userinfo(&out);
    redact_env_assignments(&out)
}

fn redact_private_key_blocks(text: &str) -> String {
    // Collapse PEM private key bodies between headers.
    let mut out = text.to_owned();
    for header in ["-----BEGIN PRIVATE KEY-----", "-----BEGIN RSA PRIVATE KEY-----"] {
        while let Some(start) = out.find(header) {
            let end_marker = header.replace("BEGIN", "END");
            let rest_start = start + header.len();
            match out[rest_start..].find(&end_marker) {
                Some(rel) => {
                    let end = rest_start + rel + end_marker.len();
                    out.replace_range(start..end, "-----REDACTED PRIVATE KEY-----");
                }
                None => {
                    out.replace_range(start.., "-----REDACTED PRIVATE KEY-----");
                    break;
                }
            }
        }
    }
    out
}

fn redact_token_prefixes(text: &str) -> String {
    let mut out = text.to_owned();
    for pattern in [
        "sk-ant-",
        "sk-proj-",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "xoxa-",
        "xoxs-",
        "glpat-",
        "AIza",
        "AKIA",
        "eyJ",
        "sk-",
    ] {
        let mut search_from = 0;
        while let Some(rel) = out[search_from..].find(pattern) {
            let pos = search_from + rel;
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
            // JWT / AWS need a minimum body so we don't shred ordinary words.
            let min_body = if pattern == "eyJ" || pattern == "AKIA" {
                pattern.len() + 8
            } else {
                after
            };
            if end > min_body || (pattern != "eyJ" && pattern != "AKIA" && end > after) {
                out.replace_range(after..end, "•••");
                search_from = after + "•••".len();
            } else {
                search_from = after;
            }
        }
    }
    out
}

fn redact_bearer_and_basic(text: &str) -> String {
    let mut out = text.to_owned();
    for prefix in ["Bearer ", "Basic "] {
        let mut search_from = 0;
        while let Some(rel) = out[search_from..].find(prefix) {
            let pos = search_from + rel + prefix.len();
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
    }
    out
}

fn redact_json_secrets(text: &str) -> String {
    // "api_key": "secret" / "password": "secret" / "token": "secret"
    let keys = [
        "api_key",
        "apikey",
        "accessToken",
        "access_token",
        "refresh_token",
        "client_secret",
        "password",
        "secret",
        "token",
        "authorization",
        "private_key",
    ];
    let mut out = text.to_owned();
    for key in keys {
        for quote in ['"', '\''] {
            let needle = format!("{quote}{key}{quote}");
            let mut from = 0;
            while let Some(rel) = out[from..].find(&needle) {
                let key_end = from + rel + needle.len();
                let rest = &out[key_end..];
                let Some(colon_rel) = rest.find(':') else {
                    break;
                };
                let after_colon = key_end + colon_rel + 1;
                let value_part = &out[after_colon..];
                let trimmed = value_part.trim_start();
                if trimmed.is_empty() {
                    break;
                }
                let q = trimmed.chars().next().unwrap();
                if q != '"' && q != '\'' {
                    // numeric / bare — leave
                    from = after_colon;
                    continue;
                }
                let value_start_offset = value_part.len() - trimmed[q.len_utf8()..].len().saturating_sub(0);
                // absolute index of first quote
                let abs_q = after_colon + value_part.find(q).unwrap_or(0);
                let body_start = abs_q + q.len_utf8();
                if body_start >= out.len() {
                    break;
                }
                let Some(end_rel) = out[body_start..].find(q) else {
                    break;
                };
                let body_end = body_start + end_rel;
                if body_end > body_start {
                    out.replace_range(body_start..body_end, "•••");
                }
                from = body_start + 3;
                let _ = value_start_offset;
            }
        }
    }
    out
}

fn redact_url_userinfo(text: &str) -> String {
    // scheme://user:pass@host → scheme://user:•••@host
    let mut out = text.to_owned();
    let mut search_from = 0;
    while let Some(rel) = out[search_from..].find("://") {
        let scheme_end = search_from + rel + 3;
        let rest = &out[scheme_end..];
        let Some(at_rel) = rest.find('@') else {
            search_from = scheme_end;
            continue;
        };
        let authority = &rest[..at_rel];
        if let Some(colon) = authority.find(':') {
            if colon > 0 && authority[colon + 1..].len() > 2 {
                let pass_start = scheme_end + colon + 1;
                let pass_end = scheme_end + at_rel;
                if pass_end > pass_start {
                    out.replace_range(pass_start..pass_end, "•••");
                    search_from = pass_start + 3;
                    continue;
                }
            }
        }
        search_from = scheme_end;
    }
    out
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
                    || upper == "TOKEN"
                    || upper == "KEY"
                    || upper == "SECRET";
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
/// Env is a minimal allowlist (not full parent inherit). cwd is the project.
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
        .env(agent_env_policy())
        .stdout_limit(48 * 1024)
        .stderr_limit(16 * 1024);
    let outcome = ProcessRunner::run(&spec, alive);
    map_process_outcome(outcome, timeout_secs)
}

fn map_process_outcome(outcome: ProcessOutcome, timeout_secs: u64) -> CommandOutcome {
    let raw_combined = outcome.combined_output();
    let raw_len = if raw_combined == "(无输出)" {
        0
    } else {
        raw_combined.len() as u64
    };
    // ProcessRunner already byte-capped streams; note that when either hit.
    let stream_capped = outcome.stdout_hit_limit || outcome.stderr_hit_limit;

    let mut text = raw_combined;
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

    let mut truncation = TruncationInfo {
        original_size: if stream_capped {
            // True total unknown — stream was cut at the byte cap.
            None
        } else {
            Some(raw_len)
        },
        truncated: stream_capped,
        kept_head: 0,
        kept_tail: 0,
    };

    let soft_cap = 4000usize;
    if text.len() > soft_cap {
        let head_cap = 2800;
        let mut head_end = head_cap.min(text.len());
        while head_end > 0 && !text.is_char_boundary(head_end) {
            head_end -= 1;
        }
        let tail_start_candidates = text.len().saturating_sub(1000);
        let mut tail_start = tail_start_candidates;
        while tail_start < text.len() && !text.is_char_boundary(tail_start) {
            tail_start += 1;
        }
        if truncation.original_size.is_none() {
            // soft-cap observation of already-capped stream still truncates.
            truncation.original_size = None;
        } else {
            truncation.original_size = Some(text.len() as u64);
        }
        truncation.truncated = true;
        truncation.kept_head = head_end;
        truncation.kept_tail = text.len() - tail_start;
        let head = &text[..head_end];
        let tail = &text[tail_start..];
        let original = truncation
            .original_size
            .map(|n| n.to_string())
            .unwrap_or_else(|| "unknown".into());
        text = format!(
            "{head}\n… [output truncated: original_size={original} kept_head={} kept_tail={} truncated=true] …\n{tail}",
            truncation.kept_head, truncation.kept_tail
        );
    } else if stream_capped {
        truncation.truncated = true;
        // Record cap observation in the notice so the model knows.
        text.push_str(&format!(
            "\n… [output truncated: original_size=unknown truncated=true; stream byte cap hit] …"
        ));
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
        truncation,
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

/// Classify a shell command by tokenizing first (not naive substring-only).
/// Highest-severity match wins.
pub fn classify_command_risk(command: &str) -> CommandRisk {
    let tokens = tokenize_shell(command);
    let head = command_head_tokens(&tokens);
    let lower_all = command.to_ascii_lowercase();
    let mut risk = CommandRisk::ReadOnly;

    let mut bump = |r: CommandRisk| {
        if r > risk {
            risk = r;
        }
    };

    // --- Catastrophic ---
    if head.first().map(|s| basename(s)) == Some("sudo".into())
        || head.iter().any(|t| basename(t) == "sudo")
    {
        bump(CommandRisk::Catastrophic);
    }
    if head.first().map(|s| basename(s)) == Some("mkfs".into())
        || head.iter().any(|t| basename(t).starts_with("mkfs"))
        || head.iter().any(|t| t == "dd" || t.starts_with("dd "))
        || lower_all.contains(":(){")
        || lower_all.contains("fork bomb")
    {
        bump(CommandRisk::Catastrophic);
    }
    if head.first().map(|s| basename(s)).is_some_and(|b| {
        matches!(b.as_str(), "shutdown" | "reboot" | "halt" | "poweroff")
    }) {
        bump(CommandRisk::Catastrophic);
    }
    // rm -rf of system / home roots (not a normal workspace file)
    if head.first().map(|s| basename(s)) == Some("rm".into()) {
        let recursive = tokens.iter().any(|t| {
            let t = t.trim_start_matches(|c| c == '\'' || c == '"');
            t.starts_with("-") && t.contains('r') && t.contains('f')
        }) || lower_all.contains("rm -rf") || lower_all.contains("rm -fr");
        let targets_system = tokens.iter().any(|t| {
            let bare = t.trim_matches(|c| c == '\'' || c == '"');
            bare == "/"
                || bare == "/*"
                || bare.starts_with("/etc")
                || bare.starts_with("/usr")
                || bare.starts_with("/bin")
                || bare.starts_with("/sbin")
                || bare.starts_with("/boot")
                || bare.starts_with("/lib")
                || bare == "~"
                || bare == "~/"
                || bare.starts_with("~/")
                || bare.starts_with("$HOME")
        });
        if recursive && targets_system {
            bump(CommandRisk::Catastrophic);
        } else {
            bump(CommandRisk::FilesystemWrite);
        }
    }

    // --- DestructiveGit ---
    if head.first().map(|s| basename(s)) == Some("git".into()) {
        let git_args: Vec<String> = head.iter().skip(1).cloned().collect();
        let sub = git_args.first().map(|s| s.to_ascii_lowercase());
        match sub.as_deref() {
            Some("push") => {
                if git_args.iter().any(|a| a == "--force" || a == "-f" || a == "--force-with-lease")
                    || git_args.iter().any(|a| a.starts_with("--force"))
                {
                    bump(CommandRisk::DestructiveGit);
                } else {
                    bump(CommandRisk::Network);
                }
            }
            Some("fetch") | Some("pull") | Some("clone") | Some("ls-remote") => {
                bump(CommandRisk::Network)
            }
            Some("reset") if git_args.iter().any(|a| a == "--hard") => {
                bump(CommandRisk::DestructiveGit)
            }
            Some("clean") if git_args.iter().any(|a| a.starts_with('-')) => {
                bump(CommandRisk::DestructiveGit)
            }
            Some("checkout") | Some("restore")
                if git_args.iter().any(|a| a == "." || a == "--" || a.starts_with("--source")) =>
            {
                // `git checkout .` / `git restore .` can wipe local edits
                if git_args.iter().any(|a| a == ".") {
                    bump(CommandRisk::DestructiveGit);
                } else {
                    bump(CommandRisk::FilesystemWrite);
                }
            }
            Some("add") | Some("commit") | Some("status") | Some("diff") | Some("log")
            | Some("branch") | Some("switch") | Some("stash") | Some("show") | Some("blame") => {
                if matches!(
                    sub.as_deref(),
                    Some("add") | Some("commit") | Some("stash") | Some("switch") | Some("branch")
                ) {
                    bump(CommandRisk::FilesystemWrite);
                }
            }
            _ => {}
        }
    }

    // --- PackageInstall ---
    let installer_heads = [
        "npm",
        "pnpm",
        "yarn",
        "pip",
        "pip3",
        "cargo",
        "brew",
        "apt",
        "apt-get",
        "yum",
        "dnf",
        "apk",
        "gem",
        "go",
        "uv",
    ];
    if let Some(h) = head.first().map(|s| basename(s)) {
        let installish = head.iter().any(|t| {
            let b = basename(t);
            b == "install" || b == "i" && (h == "npm" || h == "yarn" || h == "pnpm")
        });
        if installish && installer_heads.contains(&h.as_str()) {
            bump(CommandRisk::PackageInstall);
        }
        // cargo install / go get / go install
        if (h == "cargo" && head.iter().any(|t| basename(t) == "install"))
            || (h == "go" && head.iter().any(|t| matches!(basename(t).as_str(), "get" | "install")))
        {
            bump(CommandRisk::PackageInstall);
        }
    }

    // --- Network ---
    if head.first().map(|s| basename(s)).is_some_and(|b| {
        matches!(b.as_str(), "curl" | "wget" | "nc" | "ncat" | "ssh" | "scp" | "sftp" | "ftp")
    }) {
        bump(CommandRisk::Network);
    }

    // --- ProcessControl ---
    if head.first().map(|s| basename(s)).is_some_and(|b| {
        matches!(
            b.as_str(),
            "kill" | "killall" | "pkill" | "taskkill" | "kill -9" | "shutdown"
        )
    }) {
        bump(CommandRisk::ProcessControl);
    }
    if head.first().map(|s| basename(s)).is_some_and(|b| {
        matches!(b.as_str(), "sh" | "bash" | "zsh" | "dash")
    }) && head.iter().any(|t| t == "-c")
    {
        bump(CommandRisk::ProcessControl);
    }

    // --- SensitiveData ---
    if head.first().map(|s| basename(s)).is_some_and(|b| {
        matches!(b.as_str(), "printenv" | "env" | "set")
    }) {
        bump(CommandRisk::SensitiveData);
    }
    // Reading credential-shaped paths via shell (when not already higher)
    if lower_all.contains(".ssh/")
        || lower_all.contains("id_rsa")
        || lower_all.contains("id_ed25519")
        || lower_all.contains(".aws/credentials")
        || lower_all.contains(".env")
        || lower_all.contains(" secrets")
    {
        bump(CommandRisk::SensitiveData);
    }

    // --- FilesystemWrite: chmod/chown/write redirection ---
    if head.first().map(|s| basename(s)).is_some_and(|b| {
        matches!(b.as_str(), "chmod" | "chown" | "chgrp" | "mv" | "cp" | "tee" | "truncate")
    }) {
        bump(CommandRisk::FilesystemWrite);
    }
    if tokens.iter().any(|t| t == ">" || t == ">>")
        && tokens.iter().any(|t| t.starts_with('/') || t.starts_with("~"))
    {
        bump(CommandRisk::Catastrophic);
    }

    // Pipe-to-shell is catastrophic (download + execute).
    if tokens.iter().any(|t| t == "|") {
        let has_fetch = head
            .first()
            .map(|s| basename(s))
            .is_some_and(|b| matches!(b.as_str(), "curl" | "wget"));
        if has_fetch && lower_all.contains("sh") {
            bump(CommandRisk::Catastrophic);
        }
    }

    risk
}

/// Resolve `relative` under `project`, refusing escapes outside the root
/// and sensitive credential files even inside the workspace.
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
    if is_sensitive_relative_path(relative) {
        return Err("refusing to read or write a sensitive credential path".to_owned());
    }
    let full = project.join(relative);
    // Canonicalize parent when possible to catch symlink escapes.
    if let Ok(canon) = full.canonicalize() {
        if let Ok(root) = project.canonicalize() {
            if !canon.starts_with(&root) {
                return Err("path escapes the project root".to_owned());
            }
            // Sensitive files under the workspace (e.g. project/.env)
            if let Ok(rel) = canon.strip_prefix(&root) {
                if is_sensitive_relative_path(&rel.to_string_lossy()) {
                    return Err(
                        "refusing to read or write a sensitive credential path".to_owned()
                    );
                }
            }
        }
    }
    Ok(full)
}

/// Credential-shaped paths the agent must never open — even inside the repo.
pub fn is_sensitive_relative_path(relative: &str) -> bool {
    let norm = relative.replace('\\', "/").to_ascii_lowercase();
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    if name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
        || name.ends_with(".jks")
        || name.ends_with(".keystore")
        || name.ends_with(".crt")
        || name.ends_with(".cer")
        || name == "id_rsa"
        || name == "id_ed25519"
        || name == "id_ecdsa"
        || name == "id_dsa"
        || name == "credentials"
        || name == "credentials.json"
        || name == "service-account.json"
        || name == "secrets.toml"
        || name == "secrets.yaml"
        || name == "secrets.yml"
        || name == ".npmrc"
        || name == ".pypirc"
        || name == ".netrc"
    {
        return true;
    }
    norm.contains("/.ssh/")
        || norm.contains("/.aws/")
        || norm.contains("/.gnupg/")
        || norm.starts_with(".ssh/")
        || norm.starts_with(".aws/")
        || norm.starts_with(".gnupg/")
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
    fn redact_secrets_covers_credential_formats() {
        let cases = [
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sigpart",
            "AKIAIOSFODNN7EXAMPLE",
            "Basic dXNlcjpwYXNzd29yZA==",
            r#"{"api_key":"sk-live-abcdef123456"}"#,
            "https://user:hunter2@example.com/api",
        ];
        for case in cases {
            let clean = redact_secrets(case);
            assert!(!clean.contains(case), "not redacted: {case} => {clean}");
        }
        let pem = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBg\n-----END PRIVATE KEY-----";
        let clean = redact_secrets(pem);
        assert!(!clean.contains("MIIEvQIBADANBg"), "{clean}");
    }

    #[test]
    fn output_truncation_notices_the_model() {
        let dir = std::env::temp_dir().join(format!("kodo_trunc_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let outcome = command_run(&dir, "seq 1 5000", &|| true, 10);
        assert!(
            outcome.output.contains("output truncated") || outcome.truncation.truncated,
            "missing truncation notice: len={}",
            outcome.output.len()
        );
        if outcome.output.contains("original_size") {
            assert!(outcome.output.contains("truncated=true"));
            assert!(outcome.output.contains("kept_head"));
        }
        assert!(
            outcome.output.len() < 8000,
            "cap failed: {}",
            outcome.output.len()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn destructive_git_always_needs_approval_even_in_full() {
        let risk = classify_command_risk("git reset --hard HEAD~1");
        assert_eq!(risk, CommandRisk::DestructiveGit);
        assert!(risk.needs_approval_in_full());
        let risk = classify_command_risk("git push --force origin main");
        assert_eq!(risk, CommandRisk::DestructiveGit);
        assert!(risk.needs_approval_in_full());
        let risk = classify_command_risk("rm -rf /");
        assert_eq!(risk, CommandRisk::Catastrophic);
        assert!(risk.needs_approval_in_full());
        let risk = classify_command_risk("git status --short");
        assert_eq!(risk, CommandRisk::ReadOnly);
        assert!(!risk.needs_approval_in_full());
    }

    #[test]
    fn package_install_and_network_classify_for_ask_approval() {
        assert_eq!(classify_command_risk("npm install left-pad"), CommandRisk::PackageInstall);
        assert_eq!(classify_command_risk("pip install requests"), CommandRisk::PackageInstall);
        assert_eq!(classify_command_risk("cargo install ripgrep"), CommandRisk::PackageInstall);
        assert_eq!(classify_command_risk("curl https://example.com"), CommandRisk::Network);
        assert_eq!(
            classify_command_risk("VAR=1 curl https://api.example.com"),
            CommandRisk::Network
        );
        // Ask always needs approval for any command (including these).
        assert!(crate::Permission::Ask.needs_approval(
            crate::StepKind::Command,
            Some("npm install left-pad")
        ));
    }

    #[test]
    fn tokenized_classification_not_naive_substring() {
        // Harmless word containing "rm" as substring must stay ReadOnly.
        assert_eq!(classify_command_risk("echo confirm"), CommandRisk::ReadOnly);
        // Prefix assignment before real command is detected as write.
        assert_eq!(
            classify_command_risk("FOO=bar rm -rf build"),
            CommandRisk::FilesystemWrite
        );
        // chmod is FilesystemWrite, not catastrophic unless targeting system roots.
        assert_eq!(classify_command_risk("chmod +x script.sh"), CommandRisk::FilesystemWrite);
        assert_eq!(classify_command_risk("sudo rm -rf /"), CommandRisk::Catastrophic);
    }

    #[test]
    fn secret_stdout_and_stderr_are_redacted() {
        let dir = std::env::temp_dir().join(format!("kodo_secret_io_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let secret = "fake_secret_value_abc123";
        let outcome = command_run(
            &dir,
            &format!("echo MY_API_TOKEN={secret}; echo ERR_TOKEN={secret} >&2"),
            &|| true,
            5,
        );
        assert!(
            !outcome.output.contains(secret),
            "secret leaked in combined output: {}",
            outcome.output
        );
        assert!(outcome.output.contains("•••"), "{}", outcome.output);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_token_not_exposed_to_child_process() {
        let dir = std::env::temp_dir().join(format!("kodo_env_tok_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        std::env::set_var("KODO_LEAK_TEST_TOKEN", "parent-secret-tok");
        let outcome = command_run(
            &dir,
            "echo tok=${KODO_LEAK_TEST_TOKEN:-none}",
            &|| true,
            5,
        );
        assert!(
            outcome.output.contains("tok=none"),
            "allowlist env must not inherit arbitrary parent secrets: {}",
            outcome.output
        );
        assert!(!outcome.output.contains("parent-secret-tok"));
        std::env::remove_var("KODO_LEAK_TEST_TOKEN");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sensitive_paths_are_refused_even_inside_workspace() {
        let dir = std::env::temp_dir().join(format!("kodo_sens_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        assert!(resolve_in_project(&dir, ".env").is_err());
        assert!(resolve_in_project(&dir, "config/.env.local").is_err());
        assert!(resolve_in_project(&dir, "id_rsa").is_err());
        assert!(resolve_in_project(&dir, ".ssh/id_ed25519").is_err());
        assert!(resolve_in_project(&dir, "secrets.pem").is_err());
        assert!(resolve_in_project(&dir, "src/main.rs").is_ok());
        let _ = fs::remove_dir_all(&dir);
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
