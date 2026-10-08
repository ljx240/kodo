//! `kodo` — the terminal shell for Kodo.
//!
//! Shares the desktop's local state: settings/credentials and session logs
//! under the same Application Support directory.

mod approve;
mod args;
mod interrupt;
mod render;
mod repl;
mod turn;

use std::io::{self, Write};

use kodo_core::session;
use kodo_shell::{Overrides, ShellConfig};

use crate::args::Parsed;
use crate::turn::{permission_label, TurnContext, TurnStatus};

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = match args::parse(&argv) {
        Parsed::Help => {
            print!("{}", args::USAGE);
            0
        }
        Parsed::Version => {
            println!("kodo {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Parsed::Error(message) => {
            eprintln!("{message}");
            eprintln!();
            eprint!("{}", args::USAGE);
            2
        }
        Parsed::Run(parsed) => run(parsed),
    };
    // `process::exit` skips destructors — flush explicitly.
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
    std::process::exit(code);
}

fn run(parsed: args::Args) -> i32 {
    let project = match std::fs::canonicalize(&parsed.project) {
        Ok(path) if path.is_dir() => path,
        _ => {
            eprintln!("项目目录无效：{}", parsed.project.display());
            return 2;
        }
    };

    interrupt::install();

    let Some(session_dir) = session::dir() else {
        eprintln!("未设置 HOME，无法定位会话目录");
        return 1;
    };

    let config = ShellConfig::load(Overrides {
        permission: parsed.permission,
        model: parsed.model,
    });
    let mut ctx = TurnContext::new(project, session_dir, config);

    if let Some(id) = parsed.session.as_deref() {
        if let Err(error) = ctx.resolve_session(id) {
            eprintln!("{error}");
            return 2;
        }
    } else if parsed.resume_last {
        match ctx.continue_last() {
            Ok(Some(_)) => {}
            Ok(None) => ctx.renderer.warn("未找到可继续的会话，将开启新会话"),
            Err(error) => ctx.renderer.warn(&format!("读取会话列表失败：{error}")),
        }
    }

    // Startup banner (stderr): version, project, permission · session.
    let session_desc = match &ctx.session_id {
        Some(id) => format!("会话 {id}"),
        None => "新会话（首条消息时创建）".to_owned(),
    };
    ctx.renderer.note(&format!(
        "kodo {}\n项目 {}\n权限 {} · {}",
        env!("CARGO_PKG_VERSION"),
        ctx.project.display(),
        permission_label(ctx.permission),
        session_desc,
    ));
    if ctx.config.provider.is_none() {
        ctx.renderer
            .warn("⚠ 未配置模型或密钥，离线模式：仅本地笔记（设置请在桌面端 Kodo 中配置）");
    }

    match parsed.message {
        Some(message) => match ctx.run_turn(&message) {
            Ok(TurnStatus::Completed) => 0,
            Ok(TurnStatus::Stopped) => 130,
            Ok(TurnStatus::Failed) => 1,
            Err(error) => {
                ctx.renderer.warn(&error);
                1
            }
        },
        None => repl::run(&mut ctx),
    }
}
