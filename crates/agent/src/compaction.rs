//! History-shape compaction: summarize the middle of an agent turn into one
//! message when the context window is filling up.
//!
//! The legacy `trim_history` (see `lib.rs`) drops older messages verbatim and
//! leaves a one-line breadcrumb. That works for "I have too many system
//! messages" cases but loses all evidence when the model needs it most — late
//! in a turn, when context pressure peaks. This module replaces the **middle**
//! of history with a single summarized message and keeps:
//!
//! - the leading system block (unmodified),
//! - the most recent `keep_tail` messages verbatim (so the model still sees
//!   the latest tool results without confusion),
//! - one synthetic user message holding the summary in between.
//!
//! Triggering lives at the call site (see `lib.rs::compact_if_needed`); this
//! module is purely the mechanical transform plus the one model call that
//! produces the summary.
//!
//! No IO outside the model call. The function is sync — same shape as the
//! rest of the agent loop, no `async`/`.await`.

use crate::provider::{self, Provider, ProviderMessage};
use crate::state::Budget;
use serde::{Deserialize, Serialize};

/// Tunables for one compaction pass.
#[derive(Debug, Clone)]
pub struct CompactConfig {
    /// Percent of the model window that triggers compaction (0..=100).
    /// `80` matches Codex's "warning" band.
    pub trigger_percent: u8,
    /// How many trailing messages to keep verbatim. Six (3 user/tool pairs)
    /// is enough for the model to see the most recent action and its result.
    pub keep_tail: usize,
    /// Output cap for the summary call (chars). Past this the summary gets
    /// truncated; the agent relies on the model to write concise summaries.
    pub max_summary_chars: usize,
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            trigger_percent: 80,
            keep_tail: 6,
            max_summary_chars: 1500,
        }
    }
}

/// Result of a single compaction pass. `None` means the caller should fall
/// back to plain `trim_history` — see `compact_history`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactReport {
    /// How many older messages got replaced by the summary.
    pub dropped: usize,
    /// Always `1` today (the middle becomes one summary message).
    pub summarized_into: usize,
    /// Approximate percentage of the window reclaimed (0..=100). Round-number
    /// estimate, not a precise measurement — the UI surfaces it as a hint.
    pub freed_pct: u8,
    /// Plain-text rendering of the middle block that fed into the summary —
    /// kept so the trace UI can show what got compressed away, not just that
    /// compression happened. Empty when the block was already empty.
    #[serde(default)]
    pub source_summary: String,
    /// Number of messages covered by `source_summary` (same as `dropped`
    /// today, but kept explicit so a future partial compaction can diverge).
    #[serde(default)]
    pub covered_messages: usize,
}

/// Decide whether compaction is warranted for this turn. The caller feeds
/// the budget's most recent token reading and the planned window.
pub fn should_compact(budget: &Budget, cfg: &CompactConfig, history_len: usize) -> bool {
    if budget.window_tokens == 0 || budget.last_input_tokens == 0 {
        return false;
    }
    // Don't bother on tiny turns — there's nothing meaningful to compress.
    if history_len <= cfg.keep_tail + 1 {
        return false;
    }
    budget.percent_used() >= cfg.trigger_percent
}

/// Compact the middle of `history` into one summary message.
///
/// `None` return means compaction could not happen (provider error, empty
/// output, or `alive()` returned false). The caller is responsible for
/// deciding how to react — today the run loop simply skips the pass and the
/// next call will fall back to `trim_history` naturally.
pub fn compact_history(
    history: &mut Vec<ProviderMessage>,
    cfg: &CompactConfig,
    provider: &Provider,
    alive: &dyn Fn() -> bool,
) -> Option<CompactReport> {
    if !alive() {
        return None;
    }

    // Find the leading system block (kept verbatim). Operate on the slice
    // counts first so we never hold overlapping borrows on `history`.
    let system_end = history
        .iter()
        .take_while(|m| m.role == provider::MessageRole::System)
        .count();
    let total = history.len();
    if total <= system_end + cfg.keep_tail + 1 {
        return None;
    }
    let keep_tail_start = total - cfg.keep_tail;
    if keep_tail_start <= system_end {
        return None;
    }
    let middle_count = keep_tail_start - system_end;

    // Render the middle block while we still have an immutable borrow, then
    // move on to mutating the vector.
    let middle_text = {
        let middle = &history[system_end..keep_tail_start];
        render_middle(middle)
    };
    let sys_prompt = "You are a compaction summarizer for a coding agent. \
         Produce a concise summary that preserves any facts the agent still \
         needs: file paths, command outputs, error messages, key decisions. \
         Strip pleasantries, retries, and empty tool outputs. \
         Output plain text only — no tool calls, no JSON.";
    let user_prompt = format!(
        "Summarize the following earlier turns into a single short paragraph \
         (≤ {max_chars} chars):\n\n{middle}",
        max_chars = cfg.max_summary_chars,
        middle = middle_text,
    );

    let max_output_tokens = provider::effective_max_tokens(cfg.max_summary_chars as u32);
    let request_messages = vec![
        ProviderMessage::system(sys_prompt),
        ProviderMessage::user(user_prompt),
    ];
    let response = match provider::chat_stream(
        provider,
        &request_messages,
        &[],
        max_output_tokens,
        alive,
        &mut |_event| true,
    ) {
        Ok(r) => r,
        Err(_) => return None,
    };
    if !alive() {
        return None;
    }
    let summary = response.text.trim().to_owned();
    if summary.is_empty() {
        return None;
    }
    // Cap to the configured ceiling regardless of what the model produced.
    let summary = if summary.chars().count() > cfg.max_summary_chars {
        summary.chars().take(cfg.max_summary_chars).collect::<String>()
    } else {
        summary
    };

    // Rebuild history: [system...] + summary + [*m_tail].
    // Step 1: drain the leading system block — what's left in `history` is
    // the [middle..., tail...] slice.
    let mut new_history: Vec<ProviderMessage> = history.drain(..system_end).collect();
    // Step 2: drop the middle; `history` ends up holding only the trailing
    // `keep_tail` messages.
    let mid_to_drop = history.len().saturating_sub(cfg.keep_tail);
    let _: Vec<ProviderMessage> = history.drain(..mid_to_drop).collect();
    // Step 3: assemble [system...] + summary + tail.
    new_history.push(ProviderMessage::user(format!(
        "[compacted summary from earlier rounds]\n\n{summary}"
    )));
    new_history.append(history);
    *history = new_history;

    // Rough estimate: 1 - (new_size / old_size), clamped to 0..=100.
    let new_size = history.len();
    let freed_pct = if total == 0 {
        0
    } else {
        let ratio = (total.saturating_sub(new_size) as u64 * 100) / total as u64;
        ratio.min(100) as u8
    };
    Some(CompactReport {
        dropped: middle_count,
        summarized_into: 1,
        freed_pct,
        source_summary: middle_text,
        covered_messages: middle_count,
    })
}

/// Render the middle history block as plain text for the summary prompt.
fn render_middle(messages: &[ProviderMessage]) -> String {
    let mut out = String::new();
    for m in messages {
        match m.role {
            provider::MessageRole::System => continue,
            provider::MessageRole::User => out.push_str("[user]\n"),
            provider::MessageRole::Assistant => out.push_str("[assistant]\n"),
            provider::MessageRole::Tool => out.push_str("[tool]\n"),
        }
        for block in &m.content {
            match block {
                provider::ContentBlock::Text { text } => {
                    out.push_str(text);
                    out.push('\n');
                }
                provider::ContentBlock::ToolCall { name, arguments, .. } => {
                    out.push_str(&format!("→ tool_call {name}({arguments})\n"));
                }
                provider::ContentBlock::ToolResult { content, is_error, .. } => {
                    let tag = if *is_error { "[error]" } else { "[ok]" };
                    out.push_str(&format!("{tag} {content}\n"));
                }
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: provider::MessageRole, text: &str) -> ProviderMessage {
        ProviderMessage {
            role,
            content: vec![provider::ContentBlock::Text { text: text.into() }],
        }
    }

    #[test]
    fn should_compact_skips_small_history() {
        let mut b = Budget::default();
        b.set_window(200_000);
        b.charge_tokens(190_000); // 95%
        let cfg = CompactConfig::default();
        // 1 message (system) → too few to bother.
        let tiny = vec![msg(provider::MessageRole::System, "s")];
        assert!(!should_compact(&b, &cfg, tiny.len()));
    }

    #[test]
    fn should_compact_triggers_on_high_percent() {
        let mut b = Budget::default();
        b.set_window(200_000);
        b.charge_tokens(170_000); // 85%
        let cfg = CompactConfig::default();
        let mut history = vec![msg(provider::MessageRole::System, "s")];
        for i in 0..20 {
            history.push(msg(provider::MessageRole::User, &format!("u{i}")));
        }
        assert!(should_compact(&b, &cfg, history.len()));
    }

    #[test]
    fn should_compact_skips_when_window_unknown() {
        let mut b = Budget::default();
        // window stays 0
        b.charge_tokens(10_000);
        let cfg = CompactConfig::default();
        let history = vec![msg(provider::MessageRole::System, "s"); 50];
        assert!(!should_compact(&b, &cfg, history.len()));
    }

    #[test]
    fn should_compact_skips_below_threshold() {
        let mut b = Budget::default();
        b.set_window(200_000);
        b.charge_tokens(50_000); // 25%
        let cfg = CompactConfig::default();
        let mut history = vec![msg(provider::MessageRole::System, "s")];
        for i in 0..20 {
            history.push(msg(provider::MessageRole::User, &format!("u{i}")));
        }
        assert!(!should_compact(&b, &cfg, history.len()));
    }

    #[test]
    fn render_middle_skips_system_block() {
        let messages = vec![
            msg(provider::MessageRole::System, "you are Kodo"),
            msg(provider::MessageRole::User, "find the bug"),
            msg(provider::MessageRole::Assistant, "I'll search"),
        ];
        let rendered = render_middle(&messages);
        assert!(!rendered.contains("you are Kodo"));
        assert!(rendered.contains("[user]"));
        assert!(rendered.contains("[assistant]"));
    }

    #[test]
    fn render_middle_includes_tool_calls_and_results() {
        let messages = vec![
            msg(provider::MessageRole::System, "sys"),
            ProviderMessage {
                role: provider::MessageRole::Assistant,
                content: vec![provider::ContentBlock::ToolCall {
                    id: "c1".into(),
                    name: "run_command".into(),
                    arguments: serde_json::json!({"cmd": "ls"}),
                }],
            },
            ProviderMessage {
                role: provider::MessageRole::Tool,
                content: vec![provider::ContentBlock::ToolResult {
                    call_id: "c1".into(),
                    content: "a.rs\nb.rs".into(),
                    is_error: false,
                }],
            },
        ];
        let rendered = render_middle(&messages);
        assert!(rendered.contains("[assistant]"));
        assert!(rendered.contains("→ tool_call run_command"));
        assert!(rendered.contains("[tool]"));
        assert!(rendered.contains("[ok] a.rs"));
    }

    // The full chat_stream path requires a real Provider; covered by
    // integration tests in lib.rs. The unit tests here only exercise the
    // pure decision logic.

    #[test]
    fn compact_report_serde_roundtrip() {
        let report = CompactReport {
            dropped: 8,
            summarized_into: 1,
            freed_pct: 28,
            source_summary: "earlier reads + tool result".into(),
            covered_messages: 8,
        };
        let serialized = serde_json::to_string(&report).unwrap();
        let parsed: CompactReport = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed, report);
    }
}