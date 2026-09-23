//! Codex-style natural-language rendering of the agent event stream.
//!
//! stdout carries the assistant text, step notices and the final answer;
//! stderr carries progress notes, approval blocks and errors. `line_open`
//! guarantees a stderr write never lands mid-line on stdout.

use std::io::{IsTerminal, Write};

use kodo_agent::{SinkEvent, Step};

pub struct Renderer<O: Write, E: Write> {
    out: O,
    err: E,
    color_out: bool,
    color_err: bool,
    line_open: bool,
    streamed: bool,
}

impl Renderer<std::io::Stdout, std::io::Stderr> {
    pub fn stdio() -> Self {
        let out = std::io::stdout();
        let err = std::io::stderr();
        let color_out = out.is_terminal();
        let color_err = err.is_terminal();
        Self::new(out, err, color_out, color_err)
    }
}

impl<O: Write, E: Write> Renderer<O, E> {
    pub fn new(out: O, err: E, color_out: bool, color_err: bool) -> Self {
        Self {
            out,
            err,
            color_out,
            color_err,
            line_open: false,
            streamed: false,
        }
    }

    /// Streaming state is per turn; call before each `run()`. Closes any
    /// half-written stdout line so the next output starts clean.
    pub fn reset_turn(&mut self) {
        self.close_line();
        self.streamed = false;
    }

    /// Dim note on stderr; closes an open stdout line first.
    pub fn note(&mut self, message: &str) {
        self.close_line();
        let text = if self.color_err {
            format!("\x1b[2m{message}\x1b[0m")
        } else {
            message.to_owned()
        };
        let _ = writeln!(self.err, "{text}");
        let _ = self.err.flush();
    }

    /// Plain (non-dim) stderr line — approval blocks, warnings, errors.
    pub fn warn(&mut self, message: &str) {
        self.close_line();
        let _ = writeln!(self.err, "{message}");
        let _ = self.err.flush();
    }

    pub fn handle(&mut self, event: &SinkEvent) {
        match event {
            SinkEvent::TextDelta { text } => {
                let _ = self.out.write_all(text.as_bytes());
                let _ = self.out.flush();
                self.line_open = true;
                self.streamed = true;
            }
            SinkEvent::Started { step } => {
                self.notice(&started_notice(step));
            }
            SinkEvent::Finished {
                step,
                duration_ms,
                denied,
            } => self.finished(step, *duration_ms, *denied),
            SinkEvent::Progress { phase, detail } => {
                self.note(&format!("· {phase} — {detail}"));
            }
            SinkEvent::Failover {
                from_provider,
                from_model,
                error_class,
                to_provider,
                to_model,
                ..
            } => {
                self.note(&format!(
                    "⚠ 已切换模型 {from_provider}/{from_model} → {to_provider}/{to_model}（{error_class}）"
                ));
            }
        }
    }

    fn finished(&mut self, step: &Step, duration_ms: u64, denied: bool) {
        match step {
            Step::Command { command, .. } => {
                let line = if denied {
                    format!("✗ 已拒绝 {command}")
                } else {
                    match step_exit(step) {
                        Some(0) => format!("→ 已跑 {command}（退出 0，{duration_ms}ms）"),
                        Some(code) => format!("✗ {command}（退出 {code}）"),
                        None => format!("✗ {command}（未完成，{duration_ms}ms）"),
                    }
                };
                self.notice(&line);
            }
            Step::FileChange { changes } => {
                for change in changes {
                    self.notice(&format!(
                        "✎ 已编辑 {}（+{} -{}）",
                        change.path, change.added, change.removed
                    ));
                }
            }
            Step::Reasoning { summary, .. } => {
                self.note(&format!("· 思考：{summary}"));
            }
            Step::AgentMessage {
                text,
                checks,
                plain,
                ..
            } => {
                if self.streamed {
                    self.close_line();
                } else if !text.trim().is_empty() {
                    let _ = writeln!(self.out, "{text}");
                    let _ = self.out.flush();
                }
                if !*plain && !checks.is_empty() {
                    let joined = checks.join("；");
                    self.note(&format!("· 检查：{joined}"));
                }
                self.streamed = false;
            }
            // The Started notice plus turn flow already cover these.
            Step::Search { .. } | Step::FileRead { .. } | Step::ModelCall { .. } => {
                self.close_line();
            }
        }
    }

    /// One complete notice line on stdout (dim when the stream is a TTY).
    fn notice(&mut self, line: &str) {
        self.close_line();
        let text = if self.color_out {
            format!("\x1b[2m{line}\x1b[0m")
        } else {
            line.to_owned()
        };
        let _ = writeln!(self.out, "{text}");
        let _ = self.out.flush();
    }

    /// Ends a half-written stdout stream line, if one is open. Call before
    /// printing the REPL prompt or exiting.
    pub fn close_line(&mut self) {
        if self.line_open {
            let _ = writeln!(self.out);
            let _ = self.out.flush();
            self.line_open = false;
        }
    }
}

fn step_exit(step: &Step) -> Option<i32> {
    match step {
        Step::Command { exit_code, .. } => *exit_code,
        _ => None,
    }
}

fn started_notice(step: &Step) -> String {
    match step {
        Step::Reasoning { .. } => "→ 思考…".to_owned(),
        Step::Search { query, .. } => format!("→ 搜索 “{query}”"),
        Step::FileRead { path, .. } => format!("→ 读取 {path}"),
        Step::Command { command, .. } => format!("→ 跑 {command}"),
        Step::ModelCall { model, .. } => format!("→ 调用模型 {model}"),
        Step::FileChange { changes } => {
            let paths = changes
                .iter()
                .map(|c| c.path.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("→ 写文件 {paths}")
        }
        Step::AgentMessage { .. } => "→ 起草回答…".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kodo_agent::FileDelta;

    fn renderer() -> Renderer<Vec<u8>, Vec<u8>> {
        Renderer::new(Vec::new(), Vec::new(), false, false)
    }

    #[test]
    fn text_delta_streams_raw_to_stdout() {
        let mut r = renderer();
        r.handle(&SinkEvent::TextDelta {
            text: "你好".into(),
        });
        assert_eq!(r.out.as_slice(), b"\xe4\xbd\xa0\xe5\xa5\xbd");
        assert!(r.streamed);
    }

    #[test]
    fn started_renders_a_notice_per_kind() {
        let mut r = renderer();
        r.handle(&SinkEvent::Started {
            step: Step::Search {
                query: "fn main".into(),
                detail: String::new(),
            },
        });
        r.handle(&SinkEvent::Started {
            step: Step::Command {
                command: "ls".into(),
                cwd: ".".into(),
                output: String::new(),
                exit_code: None,
            },
        });
        let out = String::from_utf8_lossy(&r.out).into_owned();
        assert!(out.contains("→ 搜索 “fn main”"), "{out}");
        assert!(out.contains("→ 跑 ls"), "{out}");
        assert!(r.err.is_empty());
    }

    #[test]
    fn command_finished_covers_success_failure_and_denied() {
        let mut r = renderer();
        r.handle(&SinkEvent::Finished {
            step: Step::Command {
                command: "ok".into(),
                cwd: ".".into(),
                output: String::new(),
                exit_code: Some(0),
            },
            duration_ms: 7,
            denied: false,
        });
        r.handle(&SinkEvent::Finished {
            step: Step::Command {
                command: "bad".into(),
                cwd: ".".into(),
                output: String::new(),
                exit_code: Some(2),
            },
            duration_ms: 1,
            denied: false,
        });
        r.handle(&SinkEvent::Finished {
            step: Step::Command {
                command: "rm".into(),
                cwd: ".".into(),
                output: String::new(),
                exit_code: None,
            },
            duration_ms: 0,
            denied: true,
        });
        let out = String::from_utf8_lossy(&r.out).into_owned();
        assert!(out.contains("→ 已跑 ok（退出 0，7ms）"), "{out}");
        assert!(out.contains("✗ bad（退出 2）"), "{out}");
        assert!(out.contains("✗ 已拒绝 rm"), "{out}");
    }

    #[test]
    fn agent_message_prints_text_only_when_not_streamed() {
        let mut r = renderer();
        r.handle(&SinkEvent::Finished {
            step: Step::AgentMessage {
                text: "离线回答".into(),
                checks: vec!["cargo check".into()],
                delivery: "ready".into(),
                verification: "passed".into(),
                plain: false,
            },
            duration_ms: 0,
            denied: false,
        });
        let out = String::from_utf8_lossy(&r.out).into_owned();
        assert!(out.contains("离线回答"), "{out}");
        let err = String::from_utf8_lossy(&r.err).into_owned();
        assert!(err.contains("· 检查：cargo check"), "{err}");

        // Streamed this turn → text already on stdout, do not repeat it.
        let mut r = renderer();
        r.handle(&SinkEvent::TextDelta {
            text: "流式答案".into(),
        });
        r.handle(&SinkEvent::Finished {
            step: Step::AgentMessage {
                text: "流式答案".into(),
                checks: Vec::new(),
                delivery: "ready".into(),
                verification: "not_run".into(),
                plain: false,
            },
            duration_ms: 0,
            denied: false,
        });
        let out = String::from_utf8_lossy(&r.out).into_owned();
        assert_eq!(out, "流式答案\n");
    }

    #[test]
    fn stderr_note_closes_an_open_stdout_line() {
        let mut r = renderer();
        r.handle(&SinkEvent::TextDelta {
            text: "半截".into(),
        });
        r.note("· 进度");
        let out = String::from_utf8_lossy(&r.out).into_owned();
        assert_eq!(out, "半截\n");
        let err = String::from_utf8_lossy(&r.err).into_owned();
        assert!(err.contains("· 进度"), "{err}");
    }

    #[test]
    fn file_change_finished_lists_each_path() {
        let mut r = renderer();
        r.handle(&SinkEvent::Finished {
            step: Step::FileChange {
                changes: vec![
                    FileDelta {
                        path: "a.rs".into(),
                        added: 3,
                        removed: 1,
                    },
                    FileDelta {
                        path: "b.rs".into(),
                        added: 0,
                        removed: 2,
                    },
                ],
            },
            duration_ms: 0,
            denied: false,
        });
        let out = String::from_utf8_lossy(&r.out).into_owned();
        assert!(out.contains("✎ 已编辑 a.rs（+3 -1）"), "{out}");
        assert!(out.contains("✎ 已编辑 b.rs（+0 -2）"), "{out}");
    }
}
