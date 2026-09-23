//! Interactive REPL: prompt, slash commands, turn dispatch, Ctrl-C at the prompt.

use std::io::{self, BufRead, Write};

use kodo_agent::Permission;
use kodo_core::session;

use crate::args::USAGE;
use crate::interrupt;
use crate::turn::{permission_label, same_project, TurnContext};

/// Runs the REPL until EOF, `/quit`, or a second Ctrl-C. Returns the exit code.
pub fn run(ctx: &mut TurnContext) -> i32 {
    let mut line = String::new();
    let mut interrupt_hints = 0;

    loop {
        let name = ctx
            .project
            .file_name()
            .map(|part| part.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_owned());
        print!("kodo {name}> ");
        let _ = io::stdout().flush();
        ctx.renderer.close_line();

        line.clear();
        // Complete the read *before* the match: a lock held into an arm would
        // deadlock against Approver's own stdin lock during `run_turn`.
        let read = io::stdin().lock().read_line(&mut line);

        match read {
            Ok(0) => {
                // EOF — end the prompt line and exit cleanly.
                println!();
                return 0;
            }
            Ok(_) => {
                interrupt_hints = 0;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(command) = trimmed.strip_prefix('/') {
                    if handle_slash(ctx, command) {
                        return 0;
                    }
                    continue;
                }
                // Statuses are already rendered by run_turn (✗/⏹).
                if let Err(error) = ctx.run_turn(trimmed) {
                    ctx.renderer.warn(&error);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                interrupt::set(false);
                interrupt_hints += 1;
                if interrupt_hints >= 2 {
                    println!();
                    return 0;
                }
                println!();
                ctx.renderer.note("（再按一次 Ctrl-C 退出，或输入 /quit）");
                continue;
            }
            Err(error) => {
                ctx.renderer.warn(&format!("读取输入失败：{error}"));
                return 1;
            }
        }
    }
}

/// Dispatches one slash command (without the leading `/`).
/// Returns true when the REPL should exit.
fn handle_slash(ctx: &mut TurnContext, command: &str) -> bool {
    let mut parts = command.split_whitespace();
    let name = parts.next().unwrap_or("");
    match name {
        "help" => {
            print!("{USAGE}");
            let _ = io::stdout().flush();
        }
        "quit" | "exit" | "q" => return true,
        "permission" => match parts.next() {
            None => ctx
                .renderer
                .note(&format!("当前权限：{}", permission_label(ctx.permission))),
            Some(value) => match value {
                "ask" | "auto" | "full" => {
                    ctx.permission = Permission::parse(value);
                    ctx.renderer.note(&format!(
                        "权限已设为 {}（下一问生效）",
                        permission_label(ctx.permission)
                    ));
                }
                other => ctx
                    .renderer
                    .warn(&format!("未知权限模式：{other}（可用：ask、auto、full）")),
            },
        },
        "session" => match parts.next() {
            None => ctx.renderer.note("用法：/session list | new | <id>"),
            Some("list") => list_sessions(ctx),
            Some("new") => {
                ctx.session_id = None;
                ctx.renderer.note("下一条消息将开启新会话");
            }
            Some(id) => match ctx.resolve_session(id) {
                Ok(()) => ctx.renderer.note(&format!("已切换到会话 {id}")),
                Err(error) => ctx.renderer.warn(&error),
            },
        },
        other => ctx
            .renderer
            .warn(&format!("未知命令：/{other}（输入 /help 查看帮助）")),
    }
    false
}

fn list_sessions(ctx: &mut TurnContext) {
    let sessions = match session::list(&ctx.session_dir) {
        Ok(sessions) => sessions,
        Err(error) => {
            ctx.renderer.warn(&format!("读取会话列表失败：{error}"));
            return;
        }
    };
    if sessions.is_empty() {
        ctx.renderer.note("暂无会话");
        return;
    }
    for item in sessions {
        let mut marks = Vec::new();
        if same_project(&item.project, &ctx.project) {
            marks.push("[当前项目]");
        }
        if ctx.session_id.as_deref() == Some(item.id.as_str()) {
            marks.push("[当前]");
        }
        if item.archived {
            marks.push("[已归档]");
        }
        let mark = if marks.is_empty() {
            String::new()
        } else {
            format!("  {}", marks.join(""))
        };
        println!(
            "{}  {}  {}{mark}",
            item.id,
            format_time(item.at),
            item.title
        );
    }
}

/// `YYYY-MM-DD HH:MM` in local time (UTC for the non-unix fallback).
#[cfg(unix)]
fn format_time(at: u64) -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let timestamp = at as libc::time_t;
    if unsafe { libc::localtime_r(&timestamp, &mut tm) }.is_null() {
        return at.to_string();
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

#[cfg(not(unix))]
fn format_time(at: u64) -> String {
    // Howard Hinnant's civil_from_days — UTC, no chrono dependency.
    let days = (at / 86_400) as i64;
    let secs = at % 86_400;
    let (hour, minute) = ((secs / 3600) as i32, ((secs % 3600) / 60) as i32);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}
