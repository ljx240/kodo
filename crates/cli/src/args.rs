//! Hand-rolled argument parsing — same precedent as `evals`, no clap.

use std::path::PathBuf;

use kodo_agent::Permission;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// Project root; resolved against cwd by the caller.
    pub project: PathBuf,
    /// `--permission` flag; `None` → the shared settings store decides.
    pub permission: Option<Permission>,
    /// `--session <id>`: resume this session.
    pub session: Option<String>,
    /// `-c/--continue`: resume the most recent session for this project.
    pub resume_last: bool,
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
}

pub const USAGE: &str = "\
用法: kodo [选项] [消息...]

在终端里与 Kodo 对话：不带消息进入交互式 REPL，带消息则执行一回合后退出。
设置与会话和桌面端 Kodo 完全共用。

选项:
  --project <目录>        项目根目录（默认当前目录）
  --permission <模式>     ask | auto | full（默认读取与桌面端共享的设置）
  --session <id>          继续指定会话
  -c, --continue          继续本项目最近一次会话
  --model <模型>          覆盖默认模型（仅本次进程）
  -h, --help              显示本帮助
  -V, --version           显示版本

REPL 命令:
  /help                   显示帮助
  /quit                   退出（/exit、/q 同义）
  /permission [模式]      查看或设置本会话权限（ask | auto | full）
  /session list           列出会话
  /session new            下一条消息开启新会话
  /session <id>           切换到指定会话
";

pub fn parse(argv: &[String]) -> Parsed {
    let mut project: Option<PathBuf> = None;
    let mut permission: Option<Permission> = None;
    let mut session: Option<String> = None;
    let mut resume_last = false;
    let mut model: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].as_str();
        let mut take_value = |flag: &str| -> Result<String, Parsed> {
            i += 1;
            argv.get(i)
                .cloned()
                .ok_or_else(|| Parsed::Error(format!("{flag} 需要一个值")))
        };
        match arg {
            "-h" | "--help" => return Parsed::Help,
            "-V" | "--version" => return Parsed::Version,
            "-c" | "--continue" => resume_last = true,
            "--project" => match take_value("--project") {
                Ok(value) => project = Some(PathBuf::from(value)),
                Err(err) => return err,
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
                Err(err) => return err,
            },
            "--session" => match take_value("--session") {
                Ok(value) => session = Some(value),
                Err(err) => return err,
            },
            "--model" => match take_value("--model") {
                Ok(value) => model = Some(value),
                Err(err) => return err,
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
        session,
        resume_last,
        model,
        message: if positional.is_empty() {
            None
        } else {
            Some(positional.join(" "))
        },
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
        }
    }

    fn error(argv: &[&str]) -> String {
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        match parse(&argv) {
            Parsed::Error(error) => error,
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn empty_argv_is_a_repl_with_defaults() {
        let args = run(&[]);
        assert_eq!(args.project, PathBuf::from("."));
        assert_eq!(args.permission, None);
        assert_eq!(args.session, None);
        assert!(!args.resume_last);
        assert_eq!(args.model, None);
        assert_eq!(args.message, None);
    }

    #[test]
    fn positional_arguments_join_into_one_message() {
        let args = run(&["解释", "这个", "仓库"]);
        assert_eq!(args.message.as_deref(), Some("解释 这个 仓库"));
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
}
