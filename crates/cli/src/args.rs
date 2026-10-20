//! Hand-rolled argument parsing — same precedent as `evals`, no clap.

use std::path::PathBuf;

use kodo_agent::{AgentMode, Permission};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// Project root; resolved against cwd by the caller.
    pub project: PathBuf,
    /// `--permission` flag; `None` → the shared settings store decides.
    pub permission: Option<Permission>,
    /// `--mode` flag; `None` → the shared settings store decides.
    pub mode: Option<AgentMode>,
    /// `--session <id>`: resume this session.
    pub session: Option<String>,
    /// `-c/--continue`: resume the most recent session for this project.
    pub resume_last: bool,
    /// `--allow-verify`: run the planner's own verification commands without
    /// prompting. Needed for headless turns, where a prompt can never be
    /// answered and verification would otherwise be skipped entirely.
    pub allow_verify: bool,
    /// `--model <id>`: override the stored default model for this process.
    pub model: Option<String>,
    /// Joined positional arguments; `None` → interactive REPL.
    pub message: Option<String>,
}

pub enum Parsed {
    Run(Args),
    Help,
    Version,
    Error(String),
    Memory(MemoryCmd),
    /// `kodo memory --help` / `kodo memory` with no verb — print subcommand usage.
    MemoryHelp,
    Schedule(ScheduleCmd),
    /// `kodo schedule --help` / `kodo schedule` with no verb — print subcommand usage.
    ScheduleHelp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCmd {
    /// Sub-command; `None` when the user just typed `kodo memory`.
    pub verb: Option<MemoryVerb>,
    /// `--project` override; `None` → cwd.
    pub project: Option<PathBuf>,
    /// `--limit <n>` for `list` / `search`.
    pub limit: Option<usize>,
    /// `--kind <kind>` for `list` (working|episodic|semantic|procedural|rule|evidence_ref).
    pub kind: Option<String>,
    /// `--status <status>` for `list` (candidate|active|stale|superseded|quarantined|rejected).
    pub status: Option<String>,
    /// Positional query / id argument, depending on the verb.
    pub value: Option<String>,
    /// `--apply` for `reindex`; default is dry-run.
    pub apply: bool,
    /// `embed <status|warmup|off>` — present when verb == `Embed`.
    pub embed_verb: Option<EmbedVerb>,
    /// `--backend <name>` for `embed` (default = `local_bge_small_en`).
    pub embed_backend: Option<String>,
    /// `--from <dir>` for `embed warmup` — offline warmup: copy
    /// `model.onnx` and `tokenizer.json` from this local directory
    /// instead of fetching over the network. Used when
    /// `huggingface.co` is unreachable from the user's network.
    pub embed_from: Option<PathBuf>,
    /// `promote <id> --confirm --force` — write the entry as a skill file.
    /// Default (no `--confirm`) is dry-run.
    pub promote_confirm: bool,
    pub promote_force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryVerb {
    Status,
    List,
    Search,
    Inspect,
    Disable,
    Doctor,
    Reindex,
    /// `kodo memory embed <status|warmup|off>` — manage the optional
    /// local embedding backend (no-op when the `onnx` feature is off).
    Embed,
    /// Reserved for the eventual `promote` / `benchmark` verbs; not parsed
    /// in this revision — see P2 acceptance gate.
    Promote,
    Benchmark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedVerb {
    /// Print the current backend + cache state.
    Status,
    /// Download model weights (no-op without `--features onnx`).
    Warmup,
    /// Remove cached weights for a backend.
    Off,
}

/// `kodo schedule <verb>` — terminal surface for the scheduler. The
/// first version is read + lifecycle only; create / edit / archive
/// land when the GUI-side modal needs the same shapes (the desktop
/// drives writes, this CLI is the diagnostic companion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleCmd {
    pub verb: Option<ScheduleVerb>,
    /// Positional task id for `show` / `history` / `run-now` /
    /// `cancel` / `enable` / `disable` / `archive`.
    pub task_id: Option<String>,
    /// Positional run id for `cancel <run_id>`.
    pub run_id: Option<String>,
    /// `--limit N` for `list` and `history`.
    pub limit: Option<usize>,
    /// `--include-archived` for `list`.
    pub include_archived: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleVerb {
    List,
    Show,
    History,
    RunNow,
    Cancel,
    Enable,
    Disable,
    Archive,
    /// Print the registered job types the scheduler can dispatch.
    JobTypes,
}

pub const USAGE: &str = "\
用法: kodo [选项] [消息...]

在终端里与 Kodo 对话：不带消息进入交互式 REPL，带消息则执行一回合后退出。
设置与会话和桌面端 Kodo 完全共用。

选项:
  --project <目录>        项目根目录（默认当前目录）
  --permission <模式>     ask | auto | full（默认读取与桌面端共享的设置）
  --mode <模式>           work | code（默认读取与桌面端共享的设置）
  --session <id>          继续指定会话
  -c, --continue          继续本项目最近一次会话
  --allow-verify          放行验证计划自身的命令（无人值守时用；危险类别仍会询问）
  --model <模型>          覆盖默认模型（仅本次进程）
  -h, --help              显示本帮助
  -V, --version           显示版本

子命令:
  kodo memory <verb>      管理 Memory Engine（详见 `kodo memory --help`）
  kodo schedule <verb>    定时任务：查看、立即执行、取消、暂停、归档（详见 `kodo schedule --help`）

REPL 命令:
  /help                   显示帮助
  /quit                   退出（/exit、/q 同义）
  /permission [模式]      查看或设置本会话权限（ask | auto | full）
  /session list           列出会话
  /session new            下一条消息开启新会话
  /session <id>           切换到指定会话
";

pub fn parse(argv: &[String]) -> Parsed {
    // `kodo memory ...` is a sub-tree with its own flag set. Pull it out
    // before the main parser so a typo there can't take down `kodo` itself.
    if argv.first().map(String::as_str) == Some("memory") {
        return parse_memory(&argv[1..]);
    }
    // Same precedent for `kodo schedule ...`.
    if argv.first().map(String::as_str) == Some("schedule") {
        return parse_schedule(&argv[1..]);
    }

    let mut project: Option<PathBuf> = None;
    let mut permission: Option<Permission> = None;
    let mut mode: Option<AgentMode> = None;
    let mut session: Option<String> = None;
    let mut resume_last = false;
    let mut allow_verify = false;
    let mut model: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].as_str();
        let mut take_value = |flag: &str| -> Result<String, String> {
            i += 1;
            argv.get(i)
                .cloned()
                .ok_or_else(|| format!("{flag} 需要一个值"))
        };
        match arg {
            "-h" | "--help" => return Parsed::Help,
            "-V" | "--version" => return Parsed::Version,
            "-c" | "--continue" => resume_last = true,
            "--allow-verify" => allow_verify = true,
            "--project" => match take_value("--project") {
                Ok(value) => project = Some(PathBuf::from(value)),
                Err(err) => return Parsed::Error(err),
            },
            "--permission" => match take_value("--permission") {
                Ok(value) => match value.as_str() {
                    "ask" | "auto" | "full" => permission = Some(Permission::parse(&value)),
                    other => {
                        return Parsed::Error(format!(
                            "--permission 必须是 ask、auto 或 full，收到：{other}"
                        ))
                    }
                },
                Err(err) => return Parsed::Error(err),
            },
            "--mode" => match take_value("--mode") {
                Ok(value) => match value.as_str() {
                    "work" | "code" => mode = Some(AgentMode::parse(&value)),
                    other => {
                        return Parsed::Error(format!("--mode 必须是 work 或 code，收到：{other}"))
                    }
                },
                Err(err) => return Parsed::Error(err),
            },
            "--session" => match take_value("--session") {
                Ok(value) => session = Some(value),
                Err(err) => return Parsed::Error(err),
            },
            "--model" => match take_value("--model") {
                Ok(value) => model = Some(value),
                Err(err) => return Parsed::Error(err),
            },
            other if other.starts_with('-') => {
                return Parsed::Error(format!("未知参数：{other}"));
            }
            _ => positional.push(arg.to_owned()),
        }
        i += 1;
    }

    if resume_last && session.is_some() {
        return Parsed::Error("--continue 与 --session 不能同时使用".to_owned());
    }

    Parsed::Run(Args {
        project: project.unwrap_or_else(|| PathBuf::from(".")),
        permission,
        mode,
        session,
        resume_last,
        allow_verify,
        model,
        message: if positional.is_empty() {
            None
        } else {
            Some(positional.join(" "))
        },
    })
}

fn parse_memory(argv: &[String]) -> Parsed {
    let mut verb: Option<MemoryVerb> = None;
    let mut project: Option<PathBuf> = None;
    let mut limit: Option<usize> = None;
    let mut kind: Option<String> = None;
    let mut status: Option<String> = None;
    let mut value: Option<String> = None;
    let mut apply = false;
    let mut embed_verb: Option<EmbedVerb> = None;
    let mut embed_backend: Option<String> = None;
    let mut embed_from: Option<PathBuf> = None;
    let mut promote_confirm = false;
    let mut promote_force = false;

    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].as_str();
        let mut take_value = |flag: &str| -> Result<String, String> {
            i += 1;
            argv.get(i)
                .cloned()
                .ok_or_else(|| format!("{flag} 需要一个值"))
        };
        match arg {
            "-h" | "--help" => return Parsed::MemoryHelp,
            "--project" => match take_value("--project") {
                Ok(v) => project = Some(PathBuf::from(v)),
                Err(e) => return Parsed::Error(e),
            },
            "--limit" => match take_value("--limit") {
                Ok(v) => match v.parse::<usize>() {
                    Ok(n) => limit = Some(n),
                    Err(_) => return Parsed::Error(format!("--limit 需要正整数：{v}")),
                },
                Err(e) => return Parsed::Error(e),
            },
            "--kind" => match take_value("--kind") {
                Ok(v) => kind = Some(v),
                Err(e) => return Parsed::Error(e),
            },
            "--status" => match take_value("--status") {
                Ok(v) => status = Some(v),
                Err(e) => return Parsed::Error(e),
            },
            "--apply" => apply = true,
            "--backend" => match take_value("--backend") {
                Ok(v) => embed_backend = Some(v),
                Err(e) => return Parsed::Error(e),
            },
            "--from" => match take_value("--from") {
                Ok(v) => embed_from = Some(PathBuf::from(v)),
                Err(e) => return Parsed::Error(e),
            },
            "--confirm" => promote_confirm = true,
            "--force" => promote_force = true,
            other if other.starts_with('-') => {
                return Parsed::Error(format!("未知参数：{other}"));
            }
            _ => {
                if verb.is_none() {
                    verb = match arg {
                        "status" => Some(MemoryVerb::Status),
                        "list" => Some(MemoryVerb::List),
                        "search" => Some(MemoryVerb::Search),
                        "inspect" => Some(MemoryVerb::Inspect),
                        "disable" => Some(MemoryVerb::Disable),
                        "doctor" => Some(MemoryVerb::Doctor),
                        "reindex" => Some(MemoryVerb::Reindex),
                        "promote" => Some(MemoryVerb::Promote),
                        "benchmark" => Some(MemoryVerb::Benchmark),
                        "embed" => Some(MemoryVerb::Embed),
                        other => {
                            return Parsed::Error(format!("未知 memory 子命令：{other}"));
                        }
                    };
                } else if verb == Some(MemoryVerb::Embed) && embed_verb.is_none() {
                    embed_verb = match arg {
                        "status" => Some(EmbedVerb::Status),
                        "warmup" => Some(EmbedVerb::Warmup),
                        "off" => Some(EmbedVerb::Off),
                        other => {
                            return Parsed::Error(format!("未知 memory embed 子命令：{other}"));
                        }
                    };
                } else {
                    value = Some(arg.to_owned());
                }
            }
        }
        i += 1;
    }

    Parsed::Memory(MemoryCmd {
        verb,
        project,
        limit,
        kind,
        status,
        value,
        apply,
        embed_verb,
        embed_backend,
        embed_from,
        promote_confirm,
        promote_force,
    })
}

fn parse_schedule(argv: &[String]) -> Parsed {
    let mut verb: Option<ScheduleVerb> = None;
    let mut task_id: Option<String> = None;
    let mut run_id: Option<String> = None;
    let mut limit: Option<usize> = None;
    let mut include_archived = false;

    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].as_str();
        let mut take_value = |flag: &str| -> Result<String, String> {
            i += 1;
            argv.get(i)
                .cloned()
                .ok_or_else(|| format!("{flag} 需要一个值"))
        };
        match arg {
            "-h" | "--help" => return Parsed::ScheduleHelp,
            "--limit" => match take_value("--limit") {
                Ok(v) => match v.parse::<usize>() {
                    Ok(n) => limit = Some(n),
                    Err(_) => return Parsed::Error(format!("--limit 需要正整数：{v}")),
                },
                Err(e) => return Parsed::Error(e),
            },
            "--include-archived" => include_archived = true,
            other if other.starts_with('-') => {
                return Parsed::Error(format!("未知参数：{other}"));
            }
            _ => {
                if verb.is_none() {
                    verb = match arg {
                        "list" => Some(ScheduleVerb::List),
                        "show" => Some(ScheduleVerb::Show),
                        "history" => Some(ScheduleVerb::History),
                        "run-now" => Some(ScheduleVerb::RunNow),
                        "cancel" => Some(ScheduleVerb::Cancel),
                        "enable" => Some(ScheduleVerb::Enable),
                        "disable" => Some(ScheduleVerb::Disable),
                        "archive" => Some(ScheduleVerb::Archive),
                        "job-types" => Some(ScheduleVerb::JobTypes),
                        other => {
                            return Parsed::Error(format!("未知 schedule 子命令：{other}"));
                        }
                    };
                } else if task_id.is_none() {
                    task_id = Some(arg.to_owned());
                } else if run_id.is_none() {
                    run_id = Some(arg.to_owned());
                } else {
                    return Parsed::Error(format!("多余的位置参数：{arg}"));
                }
            }
        }
        i += 1;
    }

    Parsed::Schedule(ScheduleCmd {
        verb,
        task_id,
        run_id,
        limit,
        include_archived,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(argv: &[&str]) -> Args {
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        match parse(&argv) {
            Parsed::Run(args) => args,
            Parsed::Help => panic!("expected Run, got Help"),
            Parsed::Version => panic!("expected Run, got Version"),
            Parsed::Error(error) => panic!("expected Run, got Error: {error}"),
            Parsed::Memory(_) | Parsed::MemoryHelp => panic!("expected Run, got Memory"),
            Parsed::Schedule(_) | Parsed::ScheduleHelp => panic!("expected Run, got Schedule"),
        }
    }

    fn error(argv: &[&str]) -> String {
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        match parse(&argv) {
            Parsed::Error(error) => error,
            _ => panic!("expected Error"),
        }
    }

    /// Helper for parsing `kodo memory ...` argv slices. Panics if the
    /// result is not a `Memory(MemoryCmd)` so tests can `.field`
    /// directly.
    fn memory(argv: &[&str]) -> MemoryCmd {
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        match parse(&argv) {
            Parsed::Memory(cmd) => cmd,
            Parsed::Error(e) => panic!("expected Memory, got Error: {e}"),
            Parsed::Run(_) => panic!("expected Memory, got Run"),
            Parsed::Help => panic!("expected Memory, got Help"),
            Parsed::Version => panic!("expected Memory, got Version"),
            Parsed::MemoryHelp => panic!("expected Memory, got MemoryHelp"),
            Parsed::Schedule(_) | Parsed::ScheduleHelp => panic!("expected Memory, got Schedule"),
        }
    }

    /// Helper for parsing `kodo schedule ...` argv slices. Panics if
    /// the result is not a `Schedule(ScheduleCmd)`.
    fn schedule(argv: &[&str]) -> ScheduleCmd {
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        match parse(&argv) {
            Parsed::Schedule(cmd) => cmd,
            Parsed::Error(e) => panic!("expected Schedule, got Error: {e}"),
            Parsed::Run(_) => panic!("expected Schedule, got Run"),
            Parsed::Help => panic!("expected Schedule, got Help"),
            Parsed::Version => panic!("expected Schedule, got Version"),
            Parsed::Memory(_) | Parsed::MemoryHelp => panic!("expected Schedule, got Memory"),
            Parsed::ScheduleHelp => panic!("expected Schedule, got ScheduleHelp"),
        }
    }

    #[test]
    fn empty_argv_is_a_repl_with_defaults() {
        let args = run(&[]);
        assert_eq!(args.project, PathBuf::from("."));
        assert_eq!(args.permission, None);
        assert_eq!(args.mode, None);
        assert_eq!(args.session, None);
        assert!(!args.resume_last);
        assert_eq!(args.model, None);
        assert_eq!(args.message, None);
    }

    #[test]
    fn mode_flag_parses_work_and_code() {
        assert_eq!(run(&["--mode", "work"]).mode, Some(AgentMode::Work));
        assert_eq!(run(&["--mode", "code"]).mode, Some(AgentMode::Code));
        let err = error(&["--mode", "bogus"]);
        assert!(err.contains("--mode"), "{err}");
    }

    #[test]
    fn positional_arguments_join_into_one_message() {
        let args = run(&["解释", "这个", "仓库"]);
        assert_eq!(args.message.as_deref(), Some("解释 这个 仓库"));
    }

    #[test]
    fn allow_verify_is_off_unless_asked() {
        assert!(!run(&["hi"]).allow_verify);
        assert!(run(&["--allow-verify", "hi"]).allow_verify);
    }

    #[test]
    fn flags_parse_with_values() {
        let args = run(&[
            "--project",
            "/tmp/p",
            "--permission",
            "auto",
            "--session",
            "abc",
            "--model",
            "m1",
            "hi",
        ]);
        assert_eq!(args.project, PathBuf::from("/tmp/p"));
        assert_eq!(args.permission, Some(Permission::Auto));
        assert_eq!(args.session.as_deref(), Some("abc"));
        assert_eq!(args.model.as_deref(), Some("m1"));
        assert_eq!(args.message.as_deref(), Some("hi"));
    }

    #[test]
    fn continue_short_and_long_forms() {
        assert!(run(&["-c"]).resume_last);
        assert!(run(&["--continue"]).resume_last);
    }

    #[test]
    fn help_and_version_short_circuit() {
        let argv = vec!["--help".to_string()];
        assert!(matches!(parse(&argv), Parsed::Help));
        let argv = vec!["-V".to_string()];
        assert!(matches!(parse(&argv), Parsed::Version));
    }

    #[test]
    fn unknown_flag_is_an_error() {
        let message = error(&["--nope"]);
        assert!(message.contains("未知参数"), "{message}");
    }

    #[test]
    fn bad_permission_value_is_an_error() {
        let message = error(&["--permission", "yolo"]);
        assert!(message.contains("ask"), "{message}");
    }

    #[test]
    fn missing_flag_value_is_an_error() {
        let message = error(&["--project"]);
        assert!(message.contains("需要一个值"), "{message}");
    }

    #[test]
    fn continue_and_session_conflict() {
        let message = error(&["-c", "--session", "abc"]);
        assert!(message.contains("不能同时"), "{message}");
    }

    /// `kodo memory embed warmup --from <dir>` parses: `--from` lands
    /// in `embed_from` as a `PathBuf`. The downstream `run_embed` is
    /// what actually copies the files.
    #[test]
    fn memory_embed_warmup_from_parses() {
        let cmd = memory(&["memory", "embed", "warmup", "--from", "/tmp/my-bge"]);
        assert_eq!(cmd.verb, Some(MemoryVerb::Embed));
        assert_eq!(cmd.embed_verb, Some(EmbedVerb::Warmup));
        assert_eq!(cmd.embed_from, Some(PathBuf::from("/tmp/my-bge")));
    }

    /// `--from` combined with `--backend` parses both into the right
    /// fields without interfering.
    #[test]
    fn memory_embed_warmup_from_and_backend_combine() {
        let cmd = memory(&[
            "memory",
            "embed",
            "warmup",
            "--backend",
            "bge-small-en",
            "--from",
            "/tmp/bge-fixture",
        ]);
        assert_eq!(cmd.embed_backend.as_deref(), Some("bge-small-en"));
        assert_eq!(cmd.embed_from, Some(PathBuf::from("/tmp/bge-fixture")));
    }

    /// `--from` with no value is an error (matches the existing
    /// `take_value` pattern used for `--project`, `--backend`, etc.).
    #[test]
    fn memory_embed_warmup_from_missing_value_is_error() {
        let argv: Vec<String> = vec![
            "memory".into(),
            "embed".into(),
            "warmup".into(),
            "--from".into(),
        ];
        match parse(&argv) {
            Parsed::Error(msg) => assert!(msg.contains("--from"), "{msg}"),
            Parsed::Memory(_) => panic!("expected Error, got Memory"),
            Parsed::Run(_) => panic!("expected Error, got Run"),
            Parsed::Help => panic!("expected Error, got Help"),
            Parsed::Version => panic!("expected Error, got Version"),
            Parsed::MemoryHelp => panic!("expected Error, got MemoryHelp"),
            Parsed::Schedule(_) | Parsed::ScheduleHelp => {
                panic!("expected Error, got Schedule")
            }
        }
    }

    #[test]
    fn schedule_list_default_args() {
        let cmd = schedule(&["schedule", "list"]);
        assert_eq!(cmd.verb, Some(ScheduleVerb::List));
        assert_eq!(cmd.task_id, None);
        assert!(!cmd.include_archived);
        assert_eq!(cmd.limit, None);
    }

    #[test]
    fn schedule_list_with_include_archived_and_limit() {
        let cmd = schedule(&["schedule", "list", "--include-archived", "--limit", "20"]);
        assert_eq!(cmd.verb, Some(ScheduleVerb::List));
        assert!(cmd.include_archived);
        assert_eq!(cmd.limit, Some(20));
    }

    #[test]
    fn schedule_show_takes_task_id() {
        let cmd = schedule(&["schedule", "show", "task-1"]);
        assert_eq!(cmd.verb, Some(ScheduleVerb::Show));
        assert_eq!(cmd.task_id.as_deref(), Some("task-1"));
    }

    #[test]
    fn schedule_cancel_takes_task_and_run_id() {
        let cmd = schedule(&["schedule", "cancel", "task-1", "run-1"]);
        assert_eq!(cmd.verb, Some(ScheduleVerb::Cancel));
        assert_eq!(cmd.task_id.as_deref(), Some("task-1"));
        assert_eq!(cmd.run_id.as_deref(), Some("run-1"));
    }

    #[test]
    fn schedule_history_with_limit() {
        let cmd = schedule(&["schedule", "history", "task-1", "--limit", "5"]);
        assert_eq!(cmd.verb, Some(ScheduleVerb::History));
        assert_eq!(cmd.task_id.as_deref(), Some("task-1"));
        assert_eq!(cmd.limit, Some(5));
    }

    #[test]
    fn schedule_unknown_verb_is_error() {
        let argv: Vec<String> = vec!["schedule".into(), "nope".into()];
        match parse(&argv) {
            Parsed::Error(msg) => assert!(msg.contains("未知 schedule"), "{msg}"),
            Parsed::Schedule(_) => panic!("expected Error, got Schedule"),
            Parsed::Run(_) => panic!("expected Error, got Run"),
            Parsed::Help => panic!("expected Error, got Help"),
            Parsed::Version => panic!("expected Error, got Version"),
            Parsed::Memory(_) | Parsed::MemoryHelp => panic!("expected Error, got Memory"),
            Parsed::ScheduleHelp => panic!("expected Error, got ScheduleHelp"),
        }
    }

    #[test]
    fn schedule_help_short_circuits() {
        let argv: Vec<String> = vec!["schedule".into(), "--help".into()];
        assert!(matches!(parse(&argv), Parsed::ScheduleHelp));
    }
}
