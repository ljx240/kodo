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
    /// P1 post-check: did the protected anchors (leading system block +
    /// trailing tool-group-aligned tail) survive the rewrite? Defaults to
    /// `true` when the caller did not run the post-check.
    #[serde(default = "default_true")]
    pub anchors_ok: bool,
}

fn default_true() -> bool {
    true
}

/// Decide whether compaction is warranted for this turn.
///
/// The figure is the one the *next request* will carry: the caller sizes the
/// history before asking (`Budget::note_history_size`), so a provider that
/// never reports usage still gets a trigger, and a stale reading from the
/// previous call cannot keep a small history from compacting (FR-D2).
pub fn should_compact(budget: &Budget, cfg: &CompactConfig, history_len: usize) -> bool {
    if budget.window_tokens == 0 {
        return false;
    }
    // Don't bother on tiny turns — there's nothing meaningful to compress.
    if history_len <= cfg.keep_tail + 1 {
        return false;
    }
    budget.planned_percent_used() >= cfg.trigger_percent
}

/// Move a cut before every assistant call whose results remain in the tail.
pub(crate) fn tool_group_boundary(
    history: &[ProviderMessage],
    desired: usize,
    floor: usize,
) -> usize {
    let mut cut = desired.max(floor).min(history.len());
    loop {
        let mut earliest = cut;
        for message in &history[cut..] {
            for block in &message.content {
                if let provider::ContentBlock::ToolResult { call_id, .. } = block {
                    for (index, candidate) in history[floor..cut].iter().enumerate() {
                        if candidate.content.iter().any(|b| matches!(b, provider::ContentBlock::ToolCall { id, .. } if id == call_id)) { earliest = earliest.min(floor + index); }
                    }
                }
            }
        }
        if earliest == cut {
            return cut;
        }
        cut = earliest;
    }
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
    let keep_tail_start = tool_group_boundary(history, total - cfg.keep_tail, system_end);
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
        summary
            .chars()
            .take(cfg.max_summary_chars)
            .collect::<String>()
    } else {
        summary
    };

    // Rebuild history: [system...] + summary + [*m_tail].
    // Step 1: drain the leading system block — what's left in `history` is
    // the [middle..., tail...] slice.
    let mut new_history: Vec<ProviderMessage> = history.drain(..system_end).collect();
    // Step 2: drop the middle; `history` ends up holding only the trailing
    // `keep_tail` messages.
    let mid_to_drop = middle_count;
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
        anchors_ok: true,
    })
}

/// P1 invariant check: after a successful compaction, the leading system
/// block and the trailing tool-group-aligned tail must still be present
/// in the rebuilt history. The system block is preserved verbatim by
/// `compact_history`; the tail is preserved verbatim because the cut
/// only consumed `system_end..keep_tail_start` and the tail starts at
/// `keep_tail_start` (the tool-group boundary). This function is the
/// runtime check that those invariants actually held — useful when
/// `compact_history` is later refactored or partial compaction lands.
///
/// Returns `false` if the rebuilt history has the wrong shape, missing
/// the leading system block, or has a tail length that does not match
/// `keep_tail`. `pre_kept_tail` is the number of tail messages the
/// caller preserved going in (same `cfg.keep_tail`).
pub fn verify_anchors(
    history: &[provider::ProviderMessage],
    system_end: usize,
    pre_kept_tail: usize,
) -> bool {
    if history.len() < system_end + pre_kept_tail {
        return false;
    }
    if system_end > 0
        && history
            .iter()
            .take(system_end)
            .any(|m| m.role != provider::MessageRole::System)
    {
        return false;
    }
    // The tail is the last `pre_kept_tail` messages; their position in
    // the new history is `history.len() - pre_kept_tail`. We don't
    // compare content (the summary sat between system and tail and
    // did not touch either), we just confirm the count is right.
    history.len() >= system_end + 1 + pre_kept_tail
}

/// Outcome of a pre-send fit pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FitReport {
    /// How many older observations were dropped to make the request fit.
    pub dropped: usize,
    /// How many trailing messages were kept verbatim.
    pub kept_tail: usize,
    /// Share of the request's char volume that was reclaimed (0..=100).
    pub freed_pct: u8,
    /// Even the smallest legal history does not fit. The caller must not
    /// send it and must not retry: the provider would reject it again.
    pub still_over: bool,
}

/// Shrink `history` deterministically so the request about to be built fits
/// `usable_window` (the model window less the output reserve).
///
/// Unlike [`compact_history`] this makes no model call — a summarizer request
/// would have to carry the oversized history it is fixing, so it fails the
/// same way the real request would. What it keeps, in order:
///
/// - the leading system block, which holds both the operating rules and the
///   task-checkpoint message the run loop writes there, so current task
///   constraints survive the cut;
/// - a trailing run of messages cut on a tool-group boundary, so no
///   `tool_use` is ever separated from its `tool_result` in either direction;
/// - a breadcrumb naming what was dropped, so the model knows to re-read.
///
/// `None` means there is nothing to do: the history already fits, or there is
/// no middle block to drop.
pub fn fit_history_to_window(
    history: &mut Vec<ProviderMessage>,
    usable_window: u32,
    cfg: &CompactConfig,
) -> Option<FitReport> {
    let system_end = history
        .iter()
        .take_while(|m| m.role == provider::MessageRole::System)
        .count();
    // The tail is at least one message (the current turn's last request) and
    // at most the whole non-system history.
    let max_keep = history.len().saturating_sub(system_end);
    if max_keep < 2 {
        return None;
    }
    let chars_before = crate::history_estimate_chars(history);
    if crate::state::estimate_input_tokens(chars_before, history.len()) <= usable_window {
        return None;
    }
    let mut keep = cfg.keep_tail.min(max_keep - 1).max(1);
    loop {
        let desired = history.len() - keep;
        let cut = tool_group_boundary(history, desired, system_end);
        if cut > system_end {
            let marker = ProviderMessage::user(format!(
                "[context budget: dropped {} older observations before sending to fit the \
                 window; re-read files if needed]",
                cut - system_end
            ));
            let kept_chars = crate::history_estimate_chars(&history[..system_end])
                .saturating_add(crate::history_estimate_chars(&history[cut..]))
                .saturating_add(crate::history_estimate_chars(&[marker.clone()]));
            let planned = crate::state::estimate_input_tokens(kept_chars, system_end + keep + 1);
            if planned <= usable_window {
                let dropped = cut - system_end;
                let mut rebuilt: Vec<ProviderMessage> = history.drain(..system_end).collect();
                let _: Vec<ProviderMessage> = history.drain(..dropped).collect();
                rebuilt.push(marker);
                rebuilt.append(history);
                *history = rebuilt;
                let chars_after = crate::history_estimate_chars(history);
                let freed_pct = if chars_before == 0 {
                    0
                } else {
                    let saved = chars_before.saturating_sub(chars_after);
                    ((saved * 100) / chars_before).min(100) as u8
                };
                return Some(FitReport {
                    dropped,
                    kept_tail: keep,
                    freed_pct,
                    still_over: false,
                });
            }
        }
        if keep == 1 {
            return Some(FitReport {
                dropped: 0,
                kept_tail: keep,
                freed_pct: 0,
                still_over: true,
            });
        }
        keep -= 1;
    }
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
                provider::ContentBlock::ToolCall {
                    name, arguments, ..
                } => {
                    out.push_str(&format!("→ tool_call {name}({arguments})\n"));
                }
                provider::ContentBlock::ToolResult {
                    content, is_error, ..
                } => {
                    let tag = if *is_error { "[error]" } else { "[ok]" };
                    out.push_str(&format!("{tag} {content}\n"));
                }
                // Compaction summarizes history to a future-session
                // observer; image bytes are noise in that summary. Note
                // the media type so the recap still shows *which* images
                // rode the prior turn.
                provider::ContentBlock::Image { media_type, .. } => {
                    out.push_str(&format!("[image omitted {media_type}]\n"));
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
    fn boundary_preserves_seven_call_group_and_intervening_annotations() {
        let calls = (0..7)
            .map(|i| provider::NativeToolCall {
                id: format!("c{i}"),
                name: "read_file".into(),
                arguments: serde_json::json!({"path":"a.rs"}),
            })
            .collect();
        let mut history = vec![
            ProviderMessage::system("rules + original task"),
            ProviderMessage::user("older"),
            ProviderMessage::assistant("", calls),
        ];
        for i in 0..7 {
            history.push(ProviderMessage::tool_result(format!("c{i}"), "body", false));
            if i == 2 {
                history.push(ProviderMessage::user("stale annotation"));
            }
        }
        assert_eq!(tool_group_boundary(&history, history.len() - 6, 1), 2);
    }

    #[test]
    fn should_compact_skips_small_history() {
        let mut b = Budget::default();
        b.set_window(200_000);
        b.charge_tokens(190_000); // 95%
        let cfg = CompactConfig::default();
        // 1 message (system) → too few to bother.
        let tiny = [msg(provider::MessageRole::System, "s")];
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

    /// FR-D2: a provider that never reports usage still gets a trigger. The
    /// old rule demanded `last_input_tokens > 0`, so these turns compacted
    /// never and died on the provider's own rejection.
    #[test]
    fn should_compact_triggers_from_an_estimate_without_any_usage() {
        let mut b = Budget::default();
        b.set_window(8_000);
        let cfg = CompactConfig::default();
        let history = history_with_middle(12, 3_000);
        assert_eq!(b.last_input_tokens, 0, "no usage has arrived");
        b.note_history_size(crate::history_chars(&history), history.len());
        assert!(
            should_compact(&b, &cfg, history.len()),
            "planned {} tokens in an {} window",
            b.planned_input_tokens(),
            b.window_tokens
        );
    }

    /// The estimate describes the request about to be sent, so a reading from
    /// an earlier, larger call cannot keep triggering compaction on a history
    /// that has already been rewritten.
    #[test]
    fn should_compact_follows_the_history_not_a_stale_measurement() {
        let mut b = Budget::default();
        b.set_window(200_000);
        b.charge_tokens(170_000); // the previous call really was 85%
        let cfg = CompactConfig::default();
        let history = vec![
            msg(provider::MessageRole::System, "s"),
            msg(provider::MessageRole::User, "tiny"),
            msg(provider::MessageRole::User, "tiny"),
            msg(provider::MessageRole::User, "tiny"),
            msg(provider::MessageRole::User, "tiny"),
            msg(provider::MessageRole::User, "tiny"),
            msg(provider::MessageRole::User, "tiny"),
            msg(provider::MessageRole::User, "tiny"),
        ];
        b.note_history_size(crate::history_chars(&history), history.len());
        assert_eq!(b.percent_used(), 85, "the measurement is still on record");
        assert!(
            !should_compact(&b, &cfg, history.len()),
            "planned {} of {}",
            b.planned_input_tokens(),
            b.window_tokens
        );
    }

    /// History used by the fit tests: two system messages (rules plus the
    /// task-checkpoint block the run loop rewrites each round), a filler
    /// middle, one tool call/result pair, then the current request.
    fn history_with_middle(middle: usize, filler_chars: usize) -> Vec<ProviderMessage> {
        let filler = "x".repeat(filler_chars);
        let mut history = vec![
            msg(provider::MessageRole::System, "operating rules"),
            msg(
                provider::MessageRole::System,
                "## Task checkpoint\n- 不得改动公开 API",
            ),
        ];
        for i in 0..middle {
            history.push(msg(provider::MessageRole::User, &format!("{i}: {filler}")));
        }
        history.push(ProviderMessage::assistant(
            "",
            vec![provider::NativeToolCall {
                id: "c-last".into(),
                name: "read_file".into(),
                arguments: serde_json::json!({"path": "src/lib.rs"}),
            }],
        ));
        history.push(ProviderMessage::tool_result(
            "c-last",
            "pub fn run() {}",
            false,
        ));
        history.push(msg(provider::MessageRole::User, "现在把导出补上"));
        history
    }

    fn count_blocks(history: &[ProviderMessage], wants_call: bool) -> usize {
        history
            .iter()
            .flat_map(|m| m.content.iter())
            .filter(|b| {
                matches!(
                    (wants_call, b),
                    (true, provider::ContentBlock::ToolCall { .. })
                        | (false, provider::ContentBlock::ToolResult { .. })
                )
            })
            .count()
    }

    fn assert_pairs_intact(history: &[ProviderMessage]) {
        for (index, message) in history.iter().enumerate() {
            for block in &message.content {
                if let provider::ContentBlock::ToolResult { call_id, .. } = block {
                    assert!(
                        history[..index]
                            .iter()
                            .any(|prior| prior.content.iter().any(|b| matches!(
                                b,
                                provider::ContentBlock::ToolCall { id, .. } if id == call_id
                            ))),
                        "kept tool_result {call_id} without its tool_use"
                    );
                }
            }
        }
    }

    /// AC-D1 / AC-D4: the trim is mechanical, but the anchors — system rules,
    /// the task constraints carried with them, the current request, and every
    /// kept tool pairing — must survive it.
    #[test]
    fn fit_keeps_system_block_task_constraints_and_tool_pairs() {
        let cfg = CompactConfig::default();
        let history = history_with_middle(12, 400);
        // Only room for the system block, the marker and the recent group.
        let usable = crate::state::estimate_input_tokens(
            crate::history_chars(&history[..2]) + crate::history_chars(&history[14..]) + 160,
            7,
        );
        let mut history = history.clone();
        let report = fit_history_to_window(&mut history, usable, &cfg).expect("a fit happens");
        assert!(!report.still_over);
        assert!(report.dropped > 0, "{report:?}");
        assert_eq!(history[0].text_of(), "operating rules");
        assert!(history[1].text_of().contains("不得改动公开 API"));
        assert!(history[2].text_of().contains("dropped"));
        assert!(
            history.last().unwrap().text_of().contains("把导出补上"),
            "the current request must survive"
        );
        assert_eq!(count_blocks(&history, true), 1);
        assert_eq!(count_blocks(&history, false), 1);
        assert_pairs_intact(&history);
        assert!(
            crate::state::estimate_input_tokens(crate::history_chars(&history), history.len())
                <= usable,
            "the trimmed request must actually fit"
        );
    }

    #[test]
    fn fit_drops_nothing_when_the_request_already_fits() {
        let cfg = CompactConfig::default();
        let mut history = history_with_middle(4, 400);
        let before = history.len();
        assert!(
            fit_history_to_window(&mut history, 200_000, &cfg).is_none(),
            "a request with room left must not lose observations"
        );
        assert_eq!(history.len(), before);
    }

    /// FR-D4: when no legal shape fits, say so instead of returning a history
    /// the provider will reject again — the caller needs the "stop" signal.
    #[test]
    fn fit_reports_still_over_without_touching_the_history() {
        let cfg = CompactConfig::default();
        let mut history = history_with_middle(6, 4_000);
        let before = history.len();
        let report = fit_history_to_window(&mut history, 1, &cfg).expect("a verdict");
        assert!(report.still_over, "{report:?}");
        assert_eq!(report.dropped, 0);
        assert_eq!(
            history.len(),
            before,
            "an unusable plan must not rewrite history"
        );
    }

    /// AC-D5: output space is held back before the input is judged, so a
    /// request that fits the raw window can still be trimmed.
    #[test]
    fn output_reserve_trims_a_request_the_bare_window_would_accept() {
        let cfg = CompactConfig::default();
        let history = history_with_middle(10, 1_200);
        let chars = crate::history_chars(&history);
        let planned = crate::state::estimate_input_tokens(chars, history.len());

        let mut plain = Budget::default();
        plain.set_window(4_000);
        plain.note_history_size(chars, history.len());
        assert!(
            planned as u32 <= plain.window_tokens,
            "precondition: {} fits {}",
            planned,
            plain.window_tokens
        );
        assert!(!plain.overfits_window());
        assert!(fit_history_to_window(&mut history.clone(), plain.usable_window(), &cfg).is_none());

        plain.set_output_reserve(1_000);
        assert!(
            plain.overfits_window(),
            "the same history plus 1k of output does not fit"
        );
        let mut trimmed = history.clone();
        let report =
            fit_history_to_window(&mut trimmed, plain.usable_window(), &cfg).expect("a fit");
        assert!(!report.still_over);
        assert!(report.dropped > 0, "{report:?}");
        assert_pairs_intact(&trimmed);
    }

    /// A screenshot is not a context overflow. The token estimate charges an
    /// image a fixed allowance instead of its base64 length, so a media-heavy
    /// request is not trimmed (or refused) for bytes the model never counts.
    #[test]
    fn fit_charges_an_image_by_allowance_not_by_payload() {
        let base64 = "A".repeat(400_000);
        let history = vec![
            msg(provider::MessageRole::System, "rules"),
            msg(provider::MessageRole::User, "看一下这张图"),
            ProviderMessage {
                role: provider::MessageRole::User,
                content: vec![provider::ContentBlock::Image {
                    media_type: "image/png".into(),
                    data_base64: base64,
                }],
            },
        ];
        let wire = crate::history_chars(&history);
        let planned = crate::state::estimate_input_tokens(
            crate::history_estimate_chars(&history),
            history.len(),
        );
        assert!(wire > 400_000, "the bar shows the wire volume: {wire}");
        assert!(planned < 3_000, "one image, one allowance: {planned}");

        let cfg = CompactConfig::default();
        let mut history = history.clone();
        assert!(
            fit_history_to_window(&mut history, 4_000, &cfg).is_none(),
            "a pasted screenshot must not trigger a trim"
        );
        assert_eq!(history.len(), 3);
    }

    /// A whole tool group that does not fit is never split to make it fit.
    #[test]
    fn fit_never_separates_a_call_from_the_result_it_keeps() {
        let cfg = CompactConfig::default();
        let mut history = vec![msg(provider::MessageRole::System, "rules")];
        for i in 0..4 {
            history.push(ProviderMessage::assistant(
                "",
                vec![provider::NativeToolCall {
                    id: format!("c{i}"),
                    name: "read_file".into(),
                    arguments: serde_json::json!({"path": format!("{i}.rs")}),
                }],
            ));
            history.push(ProviderMessage::tool_result(
                &format!("c{i}"),
                &"y".repeat(2_000),
                false,
            ));
        }
        history.push(msg(provider::MessageRole::User, "接着改"));
        // Keep-tail of 6 lands inside the last three groups; the boundary must
        // pull the cut back so no kept result loses its call.
        let target = crate::state::estimate_input_tokens(
            crate::history_chars(&history[..1]) + 2_100 * 3 + 200,
            8,
        );
        let before = history.len();
        let report = fit_history_to_window(&mut history, target, &cfg);
        assert!(report.is_some(), "the request is larger than the target");
        assert!(before > history.len());
        assert_pairs_intact(&history);
        assert_eq!(count_blocks(&history, true), count_blocks(&history, false));
    }

    #[test]
    fn render_middle_skips_system_block() {
        let messages = [
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
            anchors_ok: true,
        };
        let serialized = serde_json::to_string(&report).unwrap();
        let parsed: CompactReport = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed, report);
    }

    /// P1 invariant: the leading system block and the trailing tool-group
    /// tail must survive a rewrite. The post-check is a positive signal
    /// today (the build path preserves both by construction) and a guard
    /// against any future refactor that introduces a regression.
    #[test]
    fn verify_anchors_accepts_intact_system_and_tail() {
        use provider::ProviderMessage;
        let history = vec![
            ProviderMessage::system("sys-1"),
            ProviderMessage::system("sys-2"),
            ProviderMessage::user("compacted summary"),
            ProviderMessage::user("tail-1"),
            ProviderMessage::assistant("tail-2", Vec::new()),
        ];
        assert!(verify_anchors(&history, 2, 2));
    }

    #[test]
    fn verify_anchors_rejects_missing_system() {
        use provider::ProviderMessage;
        // System block was supposed to be preserved but the leading slot
        // is now a user message — invariant violated.
        let history = vec![
            ProviderMessage::user("oops, system lost"),
            ProviderMessage::user("tail"),
        ];
        assert!(!verify_anchors(&history, 1, 1));
    }

    #[test]
    fn verify_anchors_rejects_short_tail() {
        use provider::ProviderMessage;
        let history = vec![
            ProviderMessage::system("sys"),
            ProviderMessage::user("summary"),
        ];
        // pre_kept_tail was 3 but only summary remains.
        assert!(!verify_anchors(&history, 1, 3));
    }
}
