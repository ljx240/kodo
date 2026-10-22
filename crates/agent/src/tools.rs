//! Local tools the agent can run. Commands always go through an explicit
//! approval path when the permission mode requires it; there is no ambient
//! "run anything" path from the model.
//!
//! Every spawned process goes through [`crate::process::ProcessRunner`]
//! (timeout + cancel + process-tree kill + output caps).

use crate::process::{
    EnvPolicy, ProcessOutcome, ProcessRunner, ProcessSpec, ProcessStatus, RunFailure, SandboxReport,
};
use kodo_sandbox::SandboxContext;
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
    /// Recognized project build/test command (cargo test, npm run build, …).
    /// Build scripts and proc-macros run arbitrary code, so this is not
    /// read-only; only Full mode auto-runs it.
    ProjectBuild,
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
            Self::ProjectBuild => "ProjectBuild",
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
            Self::ProjectBuild => "项目构建/测试命令；构建脚本可执行任意代码",
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

    /// Full mode still requires consent for opaque, sensitive, and external
    /// effects. Recognized project build/test commands are the one extra
    /// class Full accepts, because they are the normal verification loop.
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

/// One search answer stays inside a bounded block: the point is to answer
/// "where is it and what does the line say", not to dump the repository.
/// Content spans pulled from the retrieval index. We ask for more than we
/// print, so the "另有 N 处未显示" hint is truthful — and well past the context
/// packer's own 12-file ceiling, which is far too shallow for a code search.
const MAX_SEARCH_HITS: usize = 48;
const MAX_SEARCH_SNIPPETS: usize = 24;
const MAX_SEARCH_LINE_CHARS: usize = 200;
const MAX_SEARCH_CHARS: usize = 6_000;

/// Case-insensitive glob match. A pattern without `/` is tested against the
/// basename, so `*.rs` also finds `crates/agent/src/tools.rs`. `*` spans path
/// separators and `?` matches one character.
fn glob_matches(pattern: &str, path: &str) -> bool {
    let pat: Vec<char> = pattern.to_lowercase().chars().collect();
    let target = if pattern.contains('/') {
        path.to_owned()
    } else {
        basename(path)
    };
    let text: Vec<char> = target.chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star_p, mut star_t) = (None, 0usize);
    while t < text.len() {
        if p < pat.len() && (pat[p] == '?' || pat[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pat.len() && pat[p] == '*' {
            star_p = Some(p);
            star_t = t;
            p += 1;
        } else if let Some(anchor) = star_p {
            // Let the last `*` absorb one more text character and retry the
            // tail. `star_t` must advance here or the loop never moves.
            p = anchor + 1;
            star_t += 1;
            t = star_t;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == '*' {
        p += 1;
    }
    p == pat.len()
}

/// Search project files for `query`, optionally filtered by a filename glob.
/// Each content hit prints as `path:line` plus the matched line, because a
/// bare path list made the model spend a `read_file` round trip per hit.
pub fn search_files(project: &Path, query: &str, glob: Option<&str>) -> String {
    if query.trim().is_empty() {
        return "空查询".to_owned();
    }

    let mut context = crate::context::ContextManager::new(project, Default::default());
    if context.scan(&|| true).is_err() {
        return "搜索已取消".into();
    }
    let keep = |path: &str| glob.is_none_or(|pattern| glob_matches(pattern, path));
    let needle = query.to_lowercase();
    let tokens: Vec<&str> = needle.split_whitespace().collect();
    let is_hit = |line: &str| {
        let lowered = line.to_lowercase();
        lowered.contains(&needle) || tokens.iter().any(|token| lowered.contains(token))
    };

    // One span per file, centred on its densest run of matches. The span
    // window starts at `start_line`, so the matched line is that plus the
    // offset of the first hit inside the window.
    let mut found: Vec<(String, usize, usize, String)> = Vec::new();
    for span in context
        .grep(query, MAX_SEARCH_HITS, &|| true)
        .unwrap_or_default()
    {
        if !keep(&span.path) {
            continue;
        }
        let lines: Vec<&str> = span.snippet.lines().collect();
        let Some(offset) = lines.iter().copied().position(is_hit) else {
            continue;
        };
        let line = lines[offset]
            .trim()
            .chars()
            .take(MAX_SEARCH_LINE_CHARS)
            .collect::<String>();
        let others = lines
            .iter()
            .filter(|line| is_hit(line))
            .count()
            .saturating_sub(1);
        found.push((span.path, span.start_line + offset, others, line));
    }
    let hit_paths: std::collections::BTreeSet<&str> =
        found.iter().map(|(path, ..)| path.as_str()).collect();
    let mut names: Vec<String> = context
        .file_map()
        .iter()
        .map(|entry| entry.path.clone())
        .filter(|rel| keep(rel) && rel.to_lowercase().contains(&needle))
        .filter(|rel| !hit_paths.contains(rel.as_str()))
        .collect();
    names.sort();
    names.dedup();
    if found.is_empty() && names.is_empty() {
        return match glob {
            Some(pattern) => format!("0 处匹配（glob `{pattern}` 过滤后）"),
            None => "0 处匹配".into(),
        };
    }
    found.sort();
    let mut out = format!("{} 个文件匹配：\n", found.len().saturating_add(names.len()));
    let mut shown = 0usize;
    let mut skipped = 0usize;
    for (path, line, others, text) in found {
        if shown >= MAX_SEARCH_SNIPPETS || out.len() >= MAX_SEARCH_CHARS {
            skipped += 1;
            continue;
        }
        shown += 1;
        let head = if others > 0 {
            format!("{path}:{line}（同文件另有 {others} 处）")
        } else {
            format!("{path}:{line}")
        };
        if text.is_empty() {
            out.push_str(&format!("{head}\n"));
        } else {
            out.push_str(&format!("{head}\n    {text}\n"));
        }
    }
    // Filename matches print last: without a line number they still cost the
    // model a read_file round trip, so a content hit is worth more of the cap.
    for path in names {
        if shown >= MAX_SEARCH_SNIPPETS || out.len() >= MAX_SEARCH_CHARS {
            skipped += 1;
            continue;
        }
        shown += 1;
        out.push_str(&format!("{path} (文件名匹配)\n"));
    }
    if skipped > 0 {
        out.push_str(&format!(
            "…（另有 {skipped} 处未显示，可用 glob 参数或更具体的查询缩小范围）"
        ));
    }
    out.trim_end().to_owned()
}

pub fn read_text(path: &Path, max_lines: usize) -> String {
    read_text_result(path, max_lines).unwrap_or_else(|error| format!("(无法读取: {error})"))
}

/// Character budget for DOCX body / spreadsheet cell extraction.
///
/// Extraction is done once at a generous budget and then sliced by line, so
/// "continue reading from line N" means the same thing for text, DOCX and
/// sheets, and never re-reads the head to fake a continuation.
const EXTRACT_BUDGET_CHARS: usize = 4 * 1024 * 1024;

/// Structured result of one read: what the model received, which range, of
/// which representation, at which source version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOutcome {
    pub text: String,
    pub representation: crate::read_receipt::Representation,
    pub version: String,
    pub total_units: Option<usize>,
    /// 1-based inclusive range actually delivered.
    pub shown: (usize, usize),
    /// The *delivery* was clipped: extraction budget, character cap, or an
    /// incomplete observation. A window that simply ends before the last line
    /// is not "clipped" — the range itself is accurate, it is just partial.
    pub truncated: bool,
}

impl ReadOutcome {
    /// Build the receipt this read earns. `complete` is derived from the
    /// delivered range, never asserted by the caller.
    pub fn receipt(&self, path: &str) -> crate::read_receipt::ReadReceipt {
        let mut receipt =
            crate::read_receipt::ReadReceipt::new(path, self.representation, self.version.clone())
                .with_total(self.total_units);
        if self.total_units == Some(0) {
            receipt.complete = true;
        } else {
            receipt.add_range(self.shown.0, self.shown.1);
        }
        if self.truncated {
            receipt.note_truncated();
        }
        receipt
    }
}

/// Read a file for the model, with an optional 1-based start line.
///
/// The returned range is exactly what the caller may put in context: a
/// truncated window never claims the whole file.
pub fn read_outcome(
    path: &Path,
    start_line: Option<usize>,
    max_lines: usize,
    max_chars: Option<usize>,
) -> Result<ReadOutcome, String> {
    use crate::read_receipt::Representation;
    let (body, representation, mut clipped) = if crate::office::is_docx(&path.to_string_lossy()) {
        let doc = crate::office::read_docx(path, EXTRACT_BUDGET_CHARS)?;
        (doc.text, Representation::DocxText, doc.truncated)
    } else if crate::office::is_spreadsheet(&path.to_string_lossy()) {
        let sheet = crate::office::read_spreadsheet(path, EXTRACT_BUDGET_CHARS)?;
        (sheet.text, Representation::SheetCells, sheet.truncated)
    } else {
        let meta = fs::metadata(path).map_err(|e| e.to_string())?;
        if meta.len() > 8 * 1024 * 1024 {
            return Err("text file exceeds 8 MiB".into());
        }
        let raw = fs::read(path).map_err(|e| e.to_string())?;
        if raw.contains(&0) {
            return Err("binary file refused".into());
        }
        let text = String::from_utf8(raw).map_err(|e| e.to_string())?;
        (text, Representation::RawText, false)
    };
    // Digest the source bytes, not the extracted text: a DOCX that changed
    // only in styles/images still counts as a new version for overwrite
    // purposes.
    let version = crate::read_receipt::version_of(&fs::read(path).map_err(|e| e.to_string())?);
    let lines: Vec<&str> = body.lines().collect();
    let total = lines.len();
    if total == 0 {
        return Ok(ReadOutcome {
            text: String::new(),
            representation,
            version,
            total_units: Some(0),
            shown: (1, 0),
            truncated: false,
        });
    }
    let start = start_line.unwrap_or(1).clamp(1, total);
    let end = start
        .saturating_add(max_lines.max(1).saturating_sub(1))
        .min(total);
    let mut text = lines[start - 1..end].join("\n");
    // A character cap is a second truncation surface: if it fires, the model
    // did not receive the requested lines and the read cannot be complete.
    if let Some(cap) = max_chars {
        if text.chars().count() > cap {
            text = text.chars().take(cap).collect();
            clipped = true;
        }
    }
    // One sentence for the model *and* the UI (T017): the window, the total,
    // and whether continuing can still reach the rest. The desktop shows the
    // same string from `attachment::view_of`, so a partial read cannot look
    // complete on one side.
    if end < total || clipped {
        let (_, note) =
            crate::attachment::describe(representation, Some(total), (start, end), clipped);
        text.push_str("\n… ");
        text.push_str(&note);
    }
    Ok(ReadOutcome {
        text,
        representation,
        version,
        total_units: Some(total),
        shown: (start, end),
        truncated: clipped,
    })
}

pub(crate) fn read_text_result(path: &Path, max_lines: usize) -> Result<String, String> {
    read_outcome(path, None, max_lines, None).map(|outcome| outcome.text)
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
    /// The sandbox decision for this run. `None` for a trusted control-plane
    /// command that was never meant to be confined.
    pub sandbox: Option<SandboxReport>,
    /// Why the run did not end normally. `None` for an ordinary exit —
    /// including a non-zero exit such as `126`, which is a normal result
    /// and never a permission problem.
    pub failure: Option<RunFailure>,
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

/// Run a *trusted control-plane* command.
///
/// Only commands Kodo itself issues belong here (its own `git status` query,
/// a user-started terminal). Model-driven commands must go through
/// [`command_run_confined`], which requires a sandbox context.
pub fn command_run(
    cwd: &Path,
    command: &str,
    alive: &dyn Fn() -> bool,
    timeout_secs: u64,
) -> CommandOutcome {
    command_run_with(cwd, command, alive, timeout_secs, None)
}

/// Run a *model-driven* command under OS confinement.
///
/// The sandbox context is mandatory: a command the model chose is never
/// executed without confinement. When the sandbox cannot be established the
/// command is refused — it is not started unconfined.
pub fn command_run_confined(
    cwd: &Path,
    command: &str,
    alive: &dyn Fn() -> bool,
    timeout_secs: u64,
    sandbox: &SandboxContext,
) -> CommandOutcome {
    command_run_with(cwd, command, alive, timeout_secs, Some(sandbox))
}

/// Shared implementation for both command paths.
fn command_run_with(
    cwd: &Path,
    command: &str,
    alive: &dyn Fn() -> bool,
    timeout_secs: u64,
    sandbox: Option<&SandboxContext>,
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
            let mut spec = ProcessSpec::shell(cwd,
                r#"git --no-pager --no-optional-locks -c core.fsmonitor=false config --null --name-only --get-regexp "^filter\..*\.(clean|smudge|process|required)$""#)
                .env(agent_env_policy())
                .timeout(timeout)
                .stdout_limit(48 * 1024)
                .stderr_limit(16 * 1024);
            if let Some(sandbox) = sandbox {
                spec = spec.sandbox(sandbox.clone());
            }
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
    let mut spec = ProcessSpec::shell(cwd, command)
        .timeout(
            timeout
                .saturating_sub(began.elapsed())
                .max(std::time::Duration::from_millis(1)),
        )
        .env(env)
        .stdout_limit(48 * 1024)
        .stderr_limit(16 * 1024);
    if let Some(sandbox) = sandbox {
        spec = spec.sandbox(sandbox.clone());
    }
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
        sandbox: outcome.sandbox,
        failure: outcome.failure,
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
        // Backgrounding `&` (alone, not part of `&&` or a `2>&1`
        // redirect) can fork and the classifier can't see what the
        // right side does.
        || tokens
            .iter()
            .enumerate()
            .any(|(index, token)| token == "&" && !is_redirect_ampersand(&tokens, index))
        // Subshell `(...)` is a fresh scope — refuse to reason about it.
        || tokens.iter().any(|t| matches!(t.as_str(), "(" | ")"))
        // Bare `;` (without `&&` / `||`) is a footgun for read-only
        // review; keep it on the conservative path. The chain logic
        // below handles `&&` / `||` / `|` explicitly.
        || command.contains(';')
    {
        return risk.max(CommandRisk::Unknown);
    }

    // Compound-but-transparently-readable: a chain of `&&` / `||` / `|` is
    // allowed when every segment is read-only, or when it scopes a
    // recognized project build (`cd apps/desktop && cargo test | tail -20`).
    // The chain fix keeps read-only review commands out of the approval path;
    // the build segments are what Full mode accepts as its verification loop.
    // Note we do NOT take this path for `;` (kept conservative) or commands
    // containing `>` / `<` (the single-command path handles those via the
    // absolute-path heuristic and bumps to Catastrophic / FilesystemWrite
    // correctly).
    if command.contains("&&") || command.contains("||") || command.contains('|') {
        return classify_chain_risk(command);
    }

    risk
}

/// `2>&1` / `>&2` tokenize as `2` `>` `&` `1`; that `&` is a dup redirect,
/// not a background job.
fn is_redirect_ampersand(tokens: &[String], index: usize) -> bool {
    index
        .checked_sub(1)
        .and_then(|previous| tokens.get(previous))
        .is_some_and(|previous| previous == ">" || previous == ">>")
}

/// Split on `&&` / `||` / `|` tokens, keeping redirect `&` inside its
/// segment. Returns `None` for empty segments (`cmd1 && && cmd2`), which
/// stay on the conservative path.
fn split_chain_segments(tokens: &[String]) -> Option<Vec<Vec<String>>> {
    let mut segments = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token.as_str() {
            "&&" | "||" | "|" => {
                if current.is_empty() {
                    return None;
                }
                segments.push(std::mem::take(&mut current));
            }
            "&" if is_redirect_ampersand(tokens, index) => current.push(token.clone()),
            "&" => return None,
            _ => current.push(token.clone()),
        }
    }
    if current.is_empty() {
        return None;
    }
    segments.push(current);
    Some(segments)
}

/// Split on `&&` / `||` / `|` and classify every segment. Returns the
/// worst risk across segments. Empty segments force Unknown (the model
/// ran `cmd1 &&` and forgot the right side).
fn classify_chain_risk(command: &str) -> CommandRisk {
    let Some(segments) = split_chain_segments(&tokenize_shell(command)) else {
        return CommandRisk::Unknown;
    };
    let mut saw_project_build = false;
    let mut saw_cd = false;
    for segment in &segments {
        // The top-level assignment check only sees the first token; a chain
        // can hide `cd x && PATH=… npm test`. Assignments can swap the
        // executable or its environment, so no segment may start with one.
        if segment.first().is_some_and(|token| token.contains('=')) {
            return CommandRisk::Unknown;
        }
        // A project-relative `cd` only scopes the build that follows it; it
        // is never treated as a command with side effects of its own.
        if is_project_relative_cd(segment) {
            saw_cd = true;
            continue;
        }
        match classify_single_command_risk(&segment.join(" ")) {
            CommandRisk::ReadOnly => {}
            CommandRisk::ProjectBuild => saw_project_build = true,
            other => return other.max(CommandRisk::Unknown),
        }
    }
    if saw_project_build {
        CommandRisk::ProjectBuild
    } else if saw_cd {
        // `cd x && <read-only>` still asks: directory changes are only
        // transparent when they scope a recognized build.
        CommandRisk::Unknown
    } else {
        CommandRisk::ReadOnly
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

/// Lexically inside the project: no absolute, home, drive-qualified, or
/// parent-relative targets. Symlink escapes are not detectable here; build
/// commands are trusted code execution either way.
fn inside_project_path(value: &str) -> bool {
    let path = value.trim_matches(|c| c == '\'' || c == '"');
    if path.is_empty()
        || path == "-"
        || path.starts_with('/')
        || path.starts_with('~')
        || path.starts_with('$')
        || path.contains(['\\', '`'])
        || path.as_bytes().get(1) == Some(&b':')
    {
        return false;
    }
    !path.split('/').any(|part| part == "..")
}

/// `cd <project-relative dir>` used to scope a build command.
fn is_project_relative_cd(tokens: &[String]) -> bool {
    let head = command_head_tokens(tokens);
    head.len() == 2
        && basename(&head[0]) == "cd"
        && !head[0].contains(['/', '\\'])
        && inside_project_path(&head[1])
}

/// Arguments that would aim a build at something outside the project or
/// hand it another program to execute.
fn escaping_build_arg(arg: &str) -> bool {
    let bare = arg.trim_matches(|c| c == '\'' || c == '"');
    if matches!(
        bare,
        "-exec" | "--exec" | "-execdir" | "--execdir" | "--open"
    ) {
        return true;
    }
    let value = bare.split_once('=').map(|(_, value)| value).unwrap_or(bare);
    if !inside_project_path(value) {
        return true;
    }
    // `-o/tmp/out` attaches the output path to the flag.
    bare.strip_prefix("-o").is_some_and(|rest| {
        !rest.is_empty() && !rest.starts_with('-') && !inside_project_path(rest)
    })
}

/// Common project build/test entry points. They write build outputs and run
/// project code, so they are neither read-only nor arbitrary: Full mode may
/// run them without a prompt while Ask and Auto still ask.
fn is_project_build_command(head: &[String]) -> bool {
    let Some(first) = head.first() else {
        return false;
    };
    if first.contains('/') || first.contains('\\') {
        return false;
    }
    let args = &head[1..];
    if args.iter().any(|arg| escaping_build_arg(arg)) {
        return false;
    }
    let script_is_buildish = |name: &str| {
        matches!(
            name.split(':').next().unwrap_or(name),
            "build" | "test" | "lint" | "typecheck" | "check" | "fmt" | "format"
        )
    };
    match basename(first).as_str() {
        "cargo" => matches!(
            args.first().map(String::as_str),
            Some("check" | "test" | "clippy" | "fmt" | "build" | "bench" | "doc")
        ),
        "npm" => match args.first().map(String::as_str) {
            Some("test") => true,
            Some("run") => args.get(1).is_some_and(|script| script_is_buildish(script)),
            _ => false,
        },
        "pnpm" | "yarn" => match args.first().map(String::as_str) {
            Some("run") => args.get(1).is_some_and(|script| script_is_buildish(script)),
            Some(script) => script_is_buildish(script),
            None => false,
        },
        "pytest" => true,
        "go" => matches!(
            args.first().map(String::as_str),
            Some("test" | "build" | "vet")
        ),
        "python" | "python3" => args.len() >= 2 && args[0] == "-m" && args[1] == "pytest",
        _ => false,
    }
}

/// `head -20` / `tail -n 20` as a pipe sink; file arguments and follow
/// mode stay on the conservative path.
fn is_head_tail_count_only(args: &[String]) -> bool {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].trim_matches(|c| c == '\'' || c == '"');
        if matches!(arg, "-n" | "-c") {
            let Some(next) = args.get(index + 1) else {
                return false;
            };
            if next.is_empty() || !next.chars().all(|c| c.is_ascii_digit()) {
                return false;
            }
            index += 2;
        } else if let Some(digits) = arg.strip_prefix('-') {
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return false;
            }
            index += 1;
        } else {
            return false;
        }
    }
    true
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
        "head" | "tail" => is_head_tail_count_only(args),
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

    if risk == CommandRisk::ReadOnly {
        if is_project_build_command(&head) {
            return CommandRisk::ProjectBuild;
        }
        if !clearly_read_only(command, &head) {
            return CommandRisk::Unknown;
        }
    }
    risk
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
    write_project_file_checked(project, relative, content, None)
        .map(|(path, added, removed, _version)| (path, added, removed))
}

/// Current content version of a project file (`None` when it does not exist).
pub fn file_version(project: &Path, relative: &str) -> Result<Option<String>, String> {
    let full = resolve_in_project(project, relative)?;
    match fs::read(&full) {
        Ok(bytes) => Ok(Some(crate::read_receipt::version_of(&bytes))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Atomic, version-checked write.
///
/// `expect_version` is the digest the caller observed when it read the file.
/// When it is `Some`, the write only lands if the file still has those bytes:
/// the check runs before staging and again immediately before the rename, so
/// a concurrent external edit is refused instead of silently overwritten.
/// A failed write leaves the original file untouched and removes only the
/// temporary file this call created.
pub fn write_project_file_checked(
    project: &Path,
    relative: &str,
    content: &str,
    expect_version: Option<&str>,
) -> Result<(String, u32, u32, String), String> {
    let full = resolve_in_project(project, relative)?;
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let docx = crate::office::is_docx(relative);
    let current = match fs::read(&full) {
        Ok(bytes) => Some(crate::read_receipt::version_of(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    if let Some(expected) = expect_version {
        match &current {
            Some(now) if now == expected => {}
            Some(now) => {
                return Err(format!(
                    "stale_source: `{relative}` 在读取后已被修改（读取版本 {expected}，当前版本 {now}）；未写入，请重新读取后再修改"
                ))
            }
            None => {
                return Err(format!(
                    "stale_source: `{relative}` 在读取后已被删除或移动；未写入，请重新确认目标文件"
                ))
            }
        }
    }
    let old = if docx {
        crate::office::read_docx(&full, 2 * 1024 * 1024)
            .map(|doc| doc.text)
            .unwrap_or_default()
    } else {
        fs::read_to_string(&full).unwrap_or_default()
    };
    let (added, removed) = crate::patch::line_diff_counts(&old, content);
    let staged = stage_file(&full, relative, content, docx)?;
    // Second check: narrows (does not close) the external-write window between
    // the precondition and the rename. A standard cross-platform rename cannot
    // offer a true compare-and-swap to other editors.
    if let Some(expected) = expect_version {
        let now = fs::read(&full)
            .ok()
            .map(|bytes| crate::read_receipt::version_of(&bytes));
        if now.as_deref() != Some(expected) {
            let _ = fs::remove_file(&staged);
            return Err(format!(
                "stale_source: `{relative}` 在写入前发生变化；未覆盖，请重新读取后再修改"
            ));
        }
    }
    if let Err(error) = fs::rename(&staged, &full) {
        let _ = fs::remove_file(&staged);
        return Err(format!("写入失败，原文件保持不变：{error}"));
    }
    let version = crate::read_receipt::version_of(&fs::read(&full).map_err(|e| e.to_string())?);
    Ok((relative.trim().to_owned(), added, removed, version))
}

/// Write `content` to a sibling temp file and return its path.
fn stage_file(full: &Path, relative: &str, content: &str, docx: bool) -> Result<PathBuf, String> {
    let parent = full.parent().unwrap_or_else(|| Path::new("."));
    let name = full
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_owned());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let staged = parent.join(format!(".{name}.kodo-tmp-{}-{nanos}", std::process::id()));
    let result = if docx {
        crate::office::write_docx(&staged, content)
    } else {
        fs::write(&staged, content).map_err(|e| e.to_string())
    };
    if let Err(error) = result {
        let _ = fs::remove_file(&staged);
        return Err(format!("写入 `{relative}` 失败，原文件保持不变：{error}"));
    }
    Ok(staged)
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

    /// AC-004: a write that fails while staging leaves the original file intact
    /// and does not leave a temp file behind.
    #[test]
    fn failed_write_keeps_the_original_and_cleans_its_temp_file() {
        let dir = std::env::temp_dir().join(format!("kodo-fail-write-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let original = "# 原标题\n正文\n";
        fs::write(dir.join("doc.md"), original).unwrap();
        // Empty DOCX content is refused by the office writer before staging.
        let error = write_project_file_checked(&dir, "doc.docx", "   ", None)
            .expect_err("an empty DOCX must be refused");
        assert!(error.contains("失败") || error.contains("empty"), "{error}");
        assert_eq!(
            fs::read_to_string(dir.join("doc.md")).unwrap(),
            original,
            "an unrelated file must be untouched"
        );
        let leftovers: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains("kodo-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files leaked: {leftovers:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// The version precondition refuses a stale overwrite at the file layer too
    /// (not only in the agent's ledger).
    #[test]
    fn version_precondition_refuses_a_stale_overwrite() {
        let dir = std::env::temp_dir().join(format!("kodo-version-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.md"), "one\n").unwrap();
        let stale = file_version(&dir, "a.md").unwrap().unwrap();
        fs::write(dir.join("a.md"), "one\ntwo\n").unwrap();
        let error = write_project_file_checked(&dir, "a.md", "replaced\n", Some(&stale))
            .expect_err("stale version must be refused");
        assert!(error.contains("stale_source"), "{error}");
        assert_eq!(
            fs::read_to_string(dir.join("a.md")).unwrap(),
            "one\ntwo\n",
            "the external edit must survive"
        );
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
            "npm run deploy",
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
        // Recognized build/test commands are the Full-mode exception.
        for command in ["npm test", "cargo check"] {
            assert_eq!(
                classify_command_risk(command),
                CommandRisk::ProjectBuild,
                "{command}"
            );
            assert!(
                !classify_command_risk(command).needs_approval_in_full(),
                "{command}"
            );
            assert!(
                classify_command_risk(command).needs_approval_in_auto(),
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

    #[test]
    fn recognized_project_builds_auto_run_only_in_full() {
        for command in [
            "cargo check",
            "cargo check --workspace",
            "cargo test -p kodo-agent",
            "cargo clippy --all-targets -- -D warnings",
            "cargo fmt --all -- --check",
            "cargo build --release",
            "npm test",
            "npm test -- --watchAll=false",
            "npm run build",
            "npm run typecheck",
            "npm run test:visual",
            "pnpm test",
            "pnpm run build",
            "yarn build",
            "yarn run lint",
            "pytest -q",
            "python3 -m pytest -q",
            "go test ./...",
            "go build ./...",
            "go vet ./...",
        ] {
            assert_eq!(
                classify_command_risk(command),
                CommandRisk::ProjectBuild,
                "{command}"
            );
            assert!(
                crate::Permission::Ask.needs_approval(crate::StepKind::Command, Some(command)),
                "ask {command}"
            );
            assert!(
                crate::Permission::Auto.needs_approval(crate::StepKind::Command, Some(command)),
                "auto {command}"
            );
            assert!(
                !crate::Permission::Full.needs_approval(crate::StepKind::Command, Some(command)),
                "full {command}"
            );
        }
    }

    #[test]
    fn transparent_build_wrappers_stay_in_full_mode() {
        for command in [
            "cd apps/desktop && npm test",
            "cd crates/agent && cargo test -p kodo-agent 2>&1 | tail -40",
            "npm run build 2>&1 | head -20",
            "cargo check 2>&1 | tail -n 50",
            "pytest -q 2>&1 | tail -5",
            "cd apps/desktop && npm run build && git status --short",
            // Exact shapes from the failed k2k-rust evaluation run.
            "cargo check --locked 2>&1 | tail -20",
            "cargo check --manifest-path k2k-rust/Cargo.toml --locked 2>&1 | tail -15",
            "cd k2k-rust && cargo check --locked 2>&1 | tail -20",
        ] {
            assert_eq!(
                classify_command_risk(command),
                CommandRisk::ProjectBuild,
                "{command}"
            );
            assert!(
                !crate::Permission::Full.needs_approval(crate::StepKind::Command, Some(command)),
                "full {command}"
            );
            assert!(
                crate::Permission::Auto.needs_approval(crate::StepKind::Command, Some(command)),
                "auto {command}"
            );
        }
        // `head`/`tail` as pipe sinks are read-only on their own.
        assert_eq!(
            classify_command_risk("git status | tail -20"),
            CommandRisk::ReadOnly
        );
    }

    #[test]
    fn build_wrappers_cannot_escape_the_project_or_hide_other_risks() {
        for command in [
            "cd .. && npm test",
            "cd /tmp && npm test",
            "cd ~ && cargo test",
            "cargo test --target-dir ../outside",
            "cargo test --target-dir=/tmp/out",
            "cargo test --manifest-path ../outside/Cargo.toml",
            "npm run build --prefix=../outside",
            "cd apps/desktop && PATH=/tmp/evil npm test",
            "cd apps/desktop && cargo doc --open",
            "go build -o /tmp/out ./...",
            "go test -exec /bin/sh -c true",
            "npm test; rm -rf /",
            "npm test &",
            "npm test > /tmp/out",
            "npm test > build.log",
            "make",
            "make test",
            "tail -f build.log",
            "head ../outside.txt",
        ] {
            let risk = classify_command_risk(command);
            if command == "npm test > /tmp/out" {
                assert_eq!(risk, CommandRisk::Catastrophic, "{command}");
            } else {
                assert!(risk.needs_approval_in_full(), "{command} actual={risk:?}");
            }
        }
        // A chain that appends a destructive segment collapses to the worst
        // class, not to a build allowance.
        assert_eq!(
            classify_command_risk("npm test && rm -rf /"),
            CommandRisk::Catastrophic
        );
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

    fn search_fixture(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "kodo-search-{}-{}-{tag}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&root);
        for (path, body) in files {
            let full = root.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(&full, body).unwrap();
        }
        root
    }

    #[test]
    fn search_reports_the_matched_line_and_counts_hits_in_the_same_file() {
        let root = search_fixture(
            "lines",
            &[
                (
                    "src/tax.rs",
                    "// one\n// two\npub fn kodo_tax_v2(amount: u32) -> u32 {\n    amount * 7 / 100\n}\n",
                ),
                (
                    "src/loop.rs",
                    "// head\nkodo_tax_v2 first\n// mid\nkodo_tax_v2 second\n",
                ),
                ("notes.md", "# notes\n\nkodo_tax_v2 is documented here\n"),
            ],
        );
        let out = search_files(&root, "kodo_tax_v2", None);
        assert!(out.contains("src/tax.rs:3"), "{out}");
        assert!(
            out.contains("pub fn kodo_tax_v2(amount: u32) -> u32"),
            "matched text must ship with the hit: {out}"
        );
        assert!(out.contains("notes.md:3"), "{out}");
        assert!(out.contains("src/loop.rs:2"), "{out}");
        assert!(out.contains("同文件另有 1 处"), "{out}");
        assert!(out.starts_with("3 个文件匹配："), "{out}");
        // Every hit was a real content match, so no filename-only fallback.
        assert!(!out.contains("(文件名匹配)"), "{out}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn glob_matches_absorbs_variable_prefixes() {
        // A `*` that has to swallow several characters is also the case that
        // must terminate: the backtrack has to advance the star anchor.
        assert!(glob_matches("*.rs", "crates/agent/src/tools.rs"));
        assert!(glob_matches("*.rs", "tools.rs"));
        assert!(glob_matches("src/**/*.ts", "src/a/b/c.ts"));
        assert!(glob_matches("?.rs", "a.rs"));
        assert!(glob_matches("*", ""));
        assert!(!glob_matches("*.rs", "notes.md"));
        assert!(!glob_matches("src/**/*.ts", "tests/a/b/c.ts"));
        assert!(!glob_matches("a.rs", "ab.rs"));
    }

    #[test]
    fn search_glob_narrows_by_basename_or_full_path() {
        let root = search_fixture(
            "glob",
            &[
                ("src/a/deep.rs", "kodo_glob_marker inside rust\n"),
                ("docs/notes.md", "kodo_glob_marker inside markdown\n"),
            ],
        );
        // A pattern without `/` matches the basename, so `*.rs` reaches nested files.
        let only_rust = search_files(&root, "kodo_glob_marker", Some("*.rs"));
        assert!(only_rust.contains("src/a/deep.rs"), "{only_rust}");
        assert!(!only_rust.contains("notes.md"), "{only_rust}");
        let only_md = search_files(&root, "kodo_glob_marker", Some("*.md"));
        assert!(only_md.contains("docs/notes.md"), "{only_md}");
        assert!(!only_md.contains("deep.rs"), "{only_md}");
        let path_glob = search_files(&root, "kodo_glob_marker", Some("src/**/*.rs"));
        assert!(path_glob.contains("src/a/deep.rs"), "{path_glob}");
        assert!(!path_glob.contains("notes.md"), "{path_glob}");
        let filtered_out = search_files(&root, "kodo_glob_marker", Some("*.py"));
        assert_eq!(filtered_out, "0 处匹配（glob `*.py` 过滤后）");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn search_falls_back_to_filenames_and_flags_overflow() {
        let mut files: Vec<(String, String)> = (0..MAX_SEARCH_SNIPPETS + 6)
            .map(|i| {
                (
                    format!("src/mod{i:02}.rs"),
                    format!("kodo_flood_marker number {i}\n"),
                )
            })
            .collect();
        files.push((
            "kodo_flood_helper.rs".to_owned(),
            "// this file only matches by name\n".to_owned(),
        ));
        let owned: Vec<(&str, &str)> = files
            .iter()
            .map(|(path, body)| (path.as_str(), body.as_str()))
            .collect();
        let root = search_fixture("flood", &owned);
        let out = search_files(&root, "kodo_flood", None);
        assert!(out.starts_with("31 个文件匹配："), "{out}");
        assert!(out.contains("src/mod00.rs:1"), "{out}");
        assert!(
            !out.contains("src/mod29.rs"),
            "capped hits must not print: {out}"
        );
        assert!(!out.contains("(文件名匹配)"), "{out}");
        // 30 content hits minus the 24 shown, plus the filename-only file.
        assert!(out.contains("另有 7 处未显示"), "{out}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn filename_only_hits_are_listed_when_they_fit() {
        let root = search_fixture("names", &[("kodo_marker_helper.rs", "// unrelated\n")]);
        let out = search_files(&root, "kodo_marker", None);
        assert!(out.starts_with("1 个文件匹配："), "{out}");
        assert!(out.contains("kodo_marker_helper.rs (文件名匹配)"), "{out}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn search_refuses_blank_queries_and_reports_zero_hits() {
        let root = search_fixture("blank", &[("src/a.rs", "kodo_marker\n")]);
        assert_eq!(search_files(&root, "   ", None), "空查询");
        assert_eq!(search_files(&root, "kodo_absent_marker", None), "0 处匹配");
        fs::remove_dir_all(root).unwrap();
    }
}
