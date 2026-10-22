//! Synchronous terminal approval prompt — the CLI twin of the desktop
//! shell's mpsc approval rendezvous. Same fingerprints, same graduated allow.
//!
//! `Approve` in `kodo-agent` is `Fn`, not `FnMut`, so the session-wide allow
//! set lives behind interior mutability.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;

use kodo_agent::tools;
use kodo_agent::StepKind;
use kodo_shell::{approval_fingerprint, step_kind_label};

use crate::interrupt;

/// What one line of user input at the prompt means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Deny,
    Once,
    Session,
    /// Session-wide allow **and** escalate the permission to Full (next turn).
    All,
}

pub fn parse_reply(line: &str) -> Reply {
    match line.trim() {
        "y" | "Y" => Reply::Once,
        "s" | "S" => Reply::Session,
        "a" | "A" => Reply::All,
        _ => Reply::Deny,
    }
}

fn kind_label_zh(kind: StepKind) -> &'static str {
    match kind {
        StepKind::Command => "运行命令",
        StepKind::FileChange => "编辑文件",
        StepKind::ExternalTool => "调用外部工具",
        // The prompt only ever fires for Command/FileChange/ExternalTool
        // (gated by `Permission::needs_approval`); the rest are unreachable.
        other => step_kind_label(other),
    }
}

/// The prompt block, as it should appear on stderr.
pub fn prompt_block(kind: StepKind, command: &str, cwd: &str) -> String {
    let risk = tools::classify_command_risk(command);
    let reason =
        tools::redact_secrets(tools::dangerous_reason(command).unwrap_or_else(|| risk.reason()));
    let safe_command = tools::redact_secrets(command);
    let prefix = match kind {
        StepKind::Command => "$ ",
        _ => "",
    };
    format!(
        "────────────────────────────────────────\n\
         需要批准 · {}\n\
         \x20 {prefix}{safe_command}\n\
         \x20 原因：{reason}\n\
         \x20 目录：{cwd}\n\
         [y] 允许一次   [s] 本会话允许   [a] 本会话全部允许   [n] 拒绝\n\
         > ",
        kind_label_zh(kind)
    )
}

/// The prompt block for a request to run **outside** the sandbox.
///
/// The block states the risk and the scope, because this decision lifts the
/// boundary the product promised. It offers one option only: a sandbox
/// escalation is one-shot by construction.
pub fn escalation_prompt_block(command: &str, cwd: &str) -> String {
    let safe_command = tools::redact_secrets(command);
    format!(
        "────────────────────────────────────────\n\
         需要批准 · 沙箱越权\n\
         \x20 {safe_command}\n\
         \x20 原因：沙箱拒绝了该命令，本次将**不带沙箱限制**执行\n\
         \x20 风险：命令将以当前用户权限运行，可访问工作区之外的文件与网络\n\
         \x20 范围：仅本次生效，不改变本会话的沙箱设置\n\
         \x20 目录：{cwd}\n\
         [y] 仅本次允许   [n] 拒绝\n\
         > "
    )
}

pub struct Approver {
    allows: RefCell<HashSet<String>>,
    escalate: Cell<bool>,
    cwd: String,
}

impl Approver {
    pub fn new(project: PathBuf) -> Self {
        Self {
            allows: RefCell::new(HashSet::new()),
            escalate: Cell::new(false),
            cwd: project.to_string_lossy().into_owned(),
        }
    }

    /// The `Approve` callback for `agent::run()`.
    pub fn approve(&self, kind: StepKind, command: &str) -> bool {
        let interactive = io::stdin().is_terminal();
        let mut input = io::stdin().lock();
        let mut output = io::stderr();
        self.decide(kind, command, &mut input, &mut output, interactive)
    }

    /// True when the user picked `[a]` — the caller escalates permission to
    /// Full for subsequent turns (permission is baked into each RunRequest).
    pub fn take_escalate(&self) -> bool {
        self.escalate.replace(false)
    }

    pub fn decide<R: BufRead, W: Write>(
        &self,
        kind: StepKind,
        command: &str,
        input: &mut R,
        output: &mut W,
        interactive: bool,
    ) -> bool {
        // Ctrl-C already pressed → deny at once so the turn cancels.
        if interrupt::pending() {
            return false;
        }
        // A sandbox escalation is a one-shot decision by construction. It
        // never consults the session-allow cache — a command that was
        // allowed earlier does not authorise running outside the sandbox —
        // and it is never remembered either.
        let escalation = matches!(kind, StepKind::SandboxEscalation);
        let fingerprint = approval_fingerprint(kind, command);
        if !escalation && self.allows.borrow().contains(&fingerprint) {
            let _ = writeln!(output, "· 本会话已允许 {command}");
            let _ = output.flush();
            return true;
        }
        if !interactive {
            if escalation {
                let _ = writeln!(output, "非交互执行，已拒绝沙箱越权：{command}");
            } else {
                let _ = writeln!(output, "非交互输入，已拒绝：{command}");
            }
            let _ = output.flush();
            return false;
        }

        let block = if escalation {
            escalation_prompt_block(command, &self.cwd)
        } else {
            prompt_block(kind, command, &self.cwd)
        };
        let _ = write!(output, "{block}");
        let _ = output.flush();

        let mut line = String::new();
        match input.read_line(&mut line) {
            Ok(0) => {
                let _ = writeln!(output, "输入结束，已拒绝");
                let _ = output.flush();
                false
            }
            Ok(_) => match parse_reply(&line) {
                Reply::Deny => false,
                Reply::Once => true,
                // A sandbox escalation is never remembered and never widens
                // the session: "本会话" is downgraded to "仅本次".
                Reply::Session | Reply::All if escalation => {
                    let _ = writeln!(output, "· 沙箱越权仅对本次生效，不会记住");
                    let _ = output.flush();
                    true
                }
                Reply::Session => {
                    self.allows.borrow_mut().insert(fingerprint);
                    let _ = writeln!(output, "· 已允许（本会话不再询问）");
                    let _ = output.flush();
                    true
                }
                Reply::All => {
                    self.allows.borrow_mut().insert(fingerprint);
                    self.escalate.set(true);
                    let _ = writeln!(output, "· 已将本会话权限提升为 full，下一问生效");
                    let _ = output.flush();
                    true
                }
            },
            // EOF, Ctrl-C (EINTR) or a read error — deny, never guess.
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reply_matrix() {
        assert_eq!(parse_reply("y"), Reply::Once);
        assert_eq!(parse_reply(" Y \n"), Reply::Once);
        assert_eq!(parse_reply("s"), Reply::Session);
        assert_eq!(parse_reply("a"), Reply::All);
        assert_eq!(parse_reply(""), Reply::Deny);
        assert_eq!(parse_reply("n"), Reply::Deny);
        assert_eq!(parse_reply("anything"), Reply::Deny);
    }

    fn approver() -> Approver {
        Approver::new(PathBuf::from("/tmp/proj"))
    }

    #[test]
    fn y_allows_once_without_remembering() {
        let a = approver();
        let mut input = Cursor::new("y\n");
        let mut out = Vec::new();
        assert!(a.decide(StepKind::Command, "ls", &mut input, &mut out, true));
        assert!(!a
            .allows
            .borrow()
            .contains(&approval_fingerprint(StepKind::Command, "ls")));
    }

    #[test]
    fn s_remembers_the_fingerprint_for_the_session() {
        let a = approver();
        let mut input = Cursor::new("s\n");
        let mut out = Vec::new();
        assert!(a.decide(StepKind::Command, "ls", &mut input, &mut out, true));
        // Second time: no input needed (cursor exhausted anyway).
        let mut input = Cursor::new("");
        let mut out = Vec::new();
        assert!(a.decide(StepKind::Command, "ls", &mut input, &mut out, true));
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("本会话已允许"), "{text}");
    }

    #[test]
    fn a_escalates_to_full() {
        let a = approver();
        let mut input = Cursor::new("a\n");
        let mut out = Vec::new();
        assert!(a.decide(StepKind::Command, "cargo build", &mut input, &mut out, true));
        assert!(a.take_escalate());
        assert!(!a.take_escalate());
    }

    #[test]
    fn empty_line_denies() {
        let a = approver();
        let mut input = Cursor::new("\n");
        let mut out = Vec::new();
        assert!(!a.decide(StepKind::Command, "ls", &mut input, &mut out, true));
    }

    #[test]
    fn eof_denies() {
        let a = approver();
        let mut input = Cursor::new("");
        let mut out = Vec::new();
        assert!(!a.decide(StepKind::Command, "ls", &mut input, &mut out, true));
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("已拒绝"), "{text}");
    }

    #[test]
    fn non_interactive_denies_without_reading() {
        let a = approver();
        let mut input = Cursor::new("y\n");
        let mut out = Vec::new();
        assert!(!a.decide(StepKind::Command, "ls", &mut input, &mut out, false));
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("非交互输入"), "{text}");
        // Cursor untouched.
        assert_eq!(input.position(), 0);
    }

    #[test]
    fn prompt_block_shows_the_command_reason_and_options() {
        let block = prompt_block(StepKind::Command, "rm -rf target", "/tmp/p");
        assert!(block.contains("需要批准 · 运行命令"), "{block}");
        assert!(block.contains("$ rm -rf target"), "{block}");
        assert!(block.contains("原因："), "{block}");
        assert!(block.contains("目录：/tmp/p"), "{block}");
        assert!(block.contains("[y] 允许一次"), "{block}");
        assert!(block.contains("[s] 本会话允许"), "{block}");
        assert!(block.contains("[a] 本会话全部允许"), "{block}");
        assert!(block.contains("[n] 拒绝"), "{block}");

        let block = prompt_block(StepKind::FileChange, "src/lib.rs", "/tmp/p");
        assert!(block.contains("需要批准 · 编辑文件"), "{block}");
        assert!(!block.contains("$ "), "{block}");
    }

    #[test]
    fn an_escalation_is_never_remembered_for_the_session() {
        let a = approver();
        // "s" on an escalation grants this one operation only.
        let mut input = Cursor::new("s\n");
        let mut out = Vec::new();
        assert!(a.decide(
            StepKind::SandboxEscalation,
            "cargo build",
            &mut input,
            &mut out,
            true
        ));
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("仅对本次生效"), "{text}");
        assert!(
            !a.allows.borrow().contains(&approval_fingerprint(
                StepKind::SandboxEscalation,
                "cargo build"
            )),
            "an escalation must never enter the session-allow cache"
        );
    }

    #[test]
    fn a_session_allow_does_not_authorise_an_escalation() {
        let a = approver();
        // The command itself was allowed for the session …
        let mut input = Cursor::new("s\n");
        let mut out = Vec::new();
        assert!(a.decide(StepKind::Command, "cargo build", &mut input, &mut out, true));
        // … but the escalation still asks, and the cached allow does not
        // answer it.
        let mut input = Cursor::new("");
        let mut out = Vec::new();
        assert!(!a.decide(
            StepKind::SandboxEscalation,
            "cargo build",
            &mut input,
            &mut out,
            true
        ));
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("需要批准 · 沙箱越权"), "{text}");
    }

    #[test]
    fn a_non_interactive_escalation_is_refused() {
        let a = approver();
        let mut input = Cursor::new("y\n");
        let mut out = Vec::new();
        assert!(!a.decide(
            StepKind::SandboxEscalation,
            "cargo build",
            &mut input,
            &mut out,
            false
        ));
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("已拒绝沙箱越权"), "{text}");
        assert_eq!(input.position(), 0);
    }

    #[test]
    fn the_escalation_prompt_states_the_risk_and_the_scope() {
        let block = escalation_prompt_block("curl https://example.com", "/tmp/p");
        assert!(block.contains("需要批准 · 沙箱越权"), "{block}");
        assert!(block.contains("风险："), "{block}");
        assert!(block.contains("范围：仅本次生效"), "{block}");
        assert!(block.contains("[y] 仅本次允许"), "{block}");
        assert!(!block.contains("[s] 本会话允许"), "{block}");
    }
}
