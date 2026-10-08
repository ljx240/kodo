//! Per-turn tool-call dedup. Pure state, no IO — same lifetime as a turn.
//!
//! The failure mode this prevents: a confused model re-calls `list_files`
//! with the same prefix every turn, eating the tool-call budget (32/turn)
//! without any progress. Two surfaces:
//!
//! - **Same-prefix cache**: identical `(prefix)` within a turn returns the
//!   cached body (cheap "you already listed this" suffix on the result).
//! - **Repeat warning**: three consecutive same-prefix calls with no
//!   intervening `read_file` get a hint appended to the result, nudging the
//!   model to read a specific file instead of recursing on the directory.
//!
//! Both surfaces are off by default until `note_list_prefix` records at
//! least one call; cleared explicitly at the start of each turn.

use std::collections::VecDeque;

/// How many recent `list_files` calls to remember for the same-prefix check.
const LIST_DEDUP_WINDOW: usize = 16;
/// Three same-prefix calls in a row with no read in between — append a hint.
const LIST_REPEAT_WARN_THRESHOLD: usize = 3;
/// Five same-prefix calls in a row with no read in between — refuse to
/// execute (Phase 0 second-pass). Three was just a hint; the screenshot
/// showed the model happily went to ten calls anyway. Five stops the
/// spiral while leaving room for legitimate "browse then probe" patterns.
const LIST_HARD_BLOCK_THRESHOLD: usize = 5;

/// Per-turn state for tool-call dedup. Reset between turns.
#[derive(Debug, Default, Clone)]
pub struct TurnDedup {
    /// `(prefix, ts_ms)` for recent `list_files` calls in this turn. The
    /// ts is wall-clock millis (informational — only ordering matters).
    recent_list_calls: VecDeque<(String, u64)>,
    /// Number of `read_file` calls since the most recent same-prefix run.
    /// Resets to zero each time a new prefix is recorded, so the threshold
    /// only fires on uninterrupted repeats.
    reads_since_last_list: usize,
}

impl TurnDedup {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a `list_files(prefix)` call. Returns:
    /// - `Some(repeated_prefix)` when this exact prefix was already recorded
    ///   in this turn (caller appends a `(cached · same prefix)` suffix).
    /// - `None` for the first occurrence.
    pub fn note_list_prefix(&mut self, prefix: Option<&str>, ts_ms: u64) -> Option<String> {
        let key = prefix.map(|s| s.to_owned()).unwrap_or_default();
        let is_repeat = self
            .recent_list_calls
            .iter()
            .any(|(prev, _)| prev == &key);
        // Always record the call (including repeats) so the warning window
        // tracks how many trailing entries share the latest prefix.
        if self.recent_list_calls.len() >= LIST_DEDUP_WINDOW {
            self.recent_list_calls.pop_front();
        }
        self.recent_list_calls.push_back((key.clone(), ts_ms));
        if is_repeat {
            Some(key)
        } else {
            // A new prefix resets the consecutive-no-read tally so the
            // warning only triggers when the model is genuinely stuck on
            // one directory.
            self.reads_since_last_list = 0;
            None
        }
    }

    /// Record a `read_file` call so the consecutive-list warning can settle.
    pub fn note_read(&mut self) {
        self.reads_since_last_list = self.reads_since_last_list.saturating_add(1);
    }

    /// True when the most-recent prefix has been listed at least
    /// `LIST_REPEAT_WARN_THRESHOLD` times without a `read_file` between
    /// them. Caller appends a hint suggesting the model move on.
    pub fn should_warn_list_repeat(&self) -> bool {
        if self.recent_list_calls.is_empty() || self.reads_since_last_list > 0 {
            return false;
        }
        // Count how many trailing entries share the most-recent prefix.
        let (last_key, _) = self.recent_list_calls.back().expect("non-empty checked above");
        let mut count = 0usize;
        for (k, _) in self.recent_list_calls.iter().rev() {
            if k == last_key {
                count += 1;
            } else {
                break;
            }
        }
        count >= LIST_REPEAT_WARN_THRESHOLD
    }

    /// True when the most-recent prefix has been listed at least
    /// `LIST_HARD_BLOCK_THRESHOLD` times without a `read_file` between
    /// them — caller MUST refuse to dispatch this call and return a
    /// structured `ToolResult { ok: false, error: DuplicateCall }` instead.
    /// Without this, the screenshot showed the model happily making 10
    /// list_files calls in a row, burning through the 6-round budget.
    pub fn should_hard_block_list_repeat(&self) -> bool {
        if self.recent_list_calls.is_empty() || self.reads_since_last_list > 0 {
            return false;
        }
        let (last_key, _) = self
            .recent_list_calls
            .back()
            .expect("non-empty checked above");
        let mut count = 0usize;
        for (k, _) in self.recent_list_calls.iter().rev() {
            if k == last_key {
                count += 1;
            } else {
                break;
            }
        }
        count >= LIST_HARD_BLOCK_THRESHOLD
    }

    /// Clear the dedup state after a hard-block fires so the next legitimate
    /// `list_files` (e.g. on a different prefix next turn) starts fresh.
    /// Without this, the model would loop forever hitting the block on every
    /// subsequent list_files attempt.
    pub fn note_hard_block(&mut self) {
        self.recent_list_calls.clear();
        self.reads_since_last_list = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_list_call_does_not_repeat() {
        let mut d = TurnDedup::new();
        assert!(d.note_list_prefix(Some("crates/"), 1).is_none());
        assert!(!d.should_warn_list_repeat());
    }

    #[test]
    fn same_prefix_twice_returns_repeat_marker() {
        let mut d = TurnDedup::new();
        assert!(d.note_list_prefix(Some("crates/"), 1).is_none());
        let second = d.note_list_prefix(Some("crates/"), 2);
        assert_eq!(second.as_deref(), Some("crates/"));
        assert!(!d.should_warn_list_repeat());
    }

    #[test]
    fn three_consecutive_same_prefix_warns_without_read() {
        let mut d = TurnDedup::new();
        d.note_list_prefix(Some("crates/"), 1);
        d.note_list_prefix(Some("crates/"), 2);
        d.note_list_prefix(Some("crates/"), 3);
        assert!(d.should_warn_list_repeat());
    }

    #[test]
    fn three_same_prefix_with_a_read_in_between_does_not_warn() {
        let mut d = TurnDedup::new();
        d.note_list_prefix(Some("crates/"), 1);
        d.note_list_prefix(Some("crates/"), 2);
        d.note_read();
        d.note_list_prefix(Some("crates/"), 3);
        assert!(!d.should_warn_list_repeat());
    }

    #[test]
    fn different_prefix_resets_the_counter() {
        let mut d = TurnDedup::new();
        d.note_list_prefix(Some("crates/"), 1);
        d.note_list_prefix(Some("crates/"), 2);
        d.note_list_prefix(Some("apps/"), 3);
        // The new prefix only has one occurrence → no warning.
        assert!(!d.should_warn_list_repeat());
        d.note_list_prefix(Some("apps/"), 4);
        d.note_list_prefix(Some("apps/"), 5);
        // Three apps/ repeats in a row now → warn.
        assert!(d.should_warn_list_repeat());
    }

    #[test]
    fn window_overflow_drops_oldest() {
        let mut d = TurnDedup::new();
        for i in 0..(LIST_DEDUP_WINDOW + 4) {
            d.note_list_prefix(Some(&format!("p{i}/")), i as u64);
        }
        // After the overflow, only the last `LIST_DEDUP_WINDOW` entries are
        // kept; the warning logic should still work on the tail.
        // The deque's last entry already matches the new prefix (p19/ was
        // the last loop iteration); one extra push brings the trailing
        // run to length 2 → still under the threshold of 3.
        let new_prefix_idx = LIST_DEDUP_WINDOW + 3;
        d.note_list_prefix(Some(&format!("p{new_prefix_idx}/")), 99);
        assert!(!d.should_warn_list_repeat());
    }

    /// Phase 0 second-pass — five same-prefix calls (no read between) →
    /// hard-block predicate fires.
    #[test]
    fn hard_block_at_threshold_5_resets_after_block() {
        let mut d = TurnDedup::new();
        for i in 0..4 {
            d.note_list_prefix(Some("crates/"), i);
        }
        assert!(!d.should_hard_block_list_repeat(), "4 in a row is still allowed");
        assert!(d.should_warn_list_repeat(), "4 already triggers the soft warn");

        // 5th call → block fires.
        d.note_list_prefix(Some("crates/"), 4);
        assert!(d.should_hard_block_list_repeat());

        // Caller calls note_hard_block → state resets, next list_files starts fresh.
        d.note_hard_block();
        assert!(!d.should_hard_block_list_repeat());
        assert!(!d.should_warn_list_repeat());
    }

    /// Phase 0 second-pass — read_file interleaved between same-prefix
    /// lists prevents the hard block (just like soft warn).
    #[test]
    fn hard_block_does_not_fire_when_read_intervenes() {
        let mut d = TurnDedup::new();
        for _ in 0..4 {
            d.note_list_prefix(Some("crates/"), 0);
        }
        assert!(!d.should_hard_block_list_repeat());
        d.note_read();
        d.note_list_prefix(Some("crates/"), 1);
        assert!(!d.should_hard_block_list_repeat());
    }
}