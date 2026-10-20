//! Structured read evidence for whole-file overwrite permission.
//!
//! Replaces the old per-turn `BTreeSet<path>` "the model has seen this path"
//! boolean. A boolean cannot answer the only question a whole-file write
//! actually asks: *did this turn put the current bytes of this file in front
//! of the model, and is the file still the same file?*
//!
//! A [`ReadReceipt`] records:
//!
//! - the representation that reached the model (raw text / DOCX body / sheet
//!   cells) — a DOCX body is not the whole OOXML document;
//! - the 1-based inclusive ranges the model actually received;
//! - `total_units` when the reader could count them, so "covered everything"
//!   is a fact rather than an assumption;
//! - the `source_version` digest of the bytes the read observed;
//! - `truncated` (the reader cut output) and `complete` (the ranges plus the
//!   absence of truncation add up to the whole representation).
//!
//! The version digest is **FNV-1a 64 over the raw file bytes plus the byte
//! length** — a change detector, not a cryptographic commitment. It is never
//! taken from the model.
//!
//! Design notes:
//!
//! - Only a read that reached the model may record a receipt. Callers record
//!   after the tool result is built, never before formatting/compaction, so a
//!   clipped observation cannot become "complete".
//! - Ranges from different versions never merge: a new version starts a new
//!   receipt.
//! - Overwrite permission is a query (`overwrite_block`), not a flag, so a
//!   file that changed on disk after the read is refused at write time.

use std::collections::BTreeMap;

/// Which representation of the source the model received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Representation {
    /// UTF-8 (or otherwise decoded) text lines.
    RawText,
    /// DOCX body paragraphs/table text extracted from the OOXML package.
    DocxText,
    /// Spreadsheet cell values (never formula recalculation).
    SheetCells,
}

impl Representation {
    pub fn label(self) -> &'static str {
        match self {
            Self::RawText => "raw_text",
            Self::DocxText => "docx_text",
            Self::SheetCells => "sheet_cells",
        }
    }

    /// What one unit of this representation is called in user-facing prose.
    /// Every representation is counted in extracted text lines, including a
    /// sheet (whose extracted lines are rows plus `## sheet:` headers).
    pub fn unit_noun(self) -> &'static str {
        "行"
    }
}

/// One read observation for one path in one turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadReceipt {
    /// Project-relative path as the caller resolves it (display form).
    pub path: String,
    pub representation: Representation,
    /// Opaque digest of the source bytes at read time. Never model-provided.
    pub version: String,
    /// Total lines/rows when the reader could count them.
    pub total_units: Option<usize>,
    /// 1-based inclusive ranges the model received.
    pub ranges: Vec<(usize, usize)>,
    /// The reader cut the output (line cap, char cap, extraction budget).
    pub truncated: bool,
    /// The model received the whole representation (not the whole file:
    /// a DOCX body still excludes images/styles/comments).
    pub complete: bool,
}

impl ReadReceipt {
    pub fn new(
        path: impl Into<String>,
        representation: Representation,
        version: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            representation,
            version: version.into(),
            total_units: None,
            ranges: Vec::new(),
            truncated: false,
            complete: false,
        }
    }

    pub fn with_total(mut self, total: Option<usize>) -> Self {
        self.total_units = total;
        self
    }

    /// Record one delivered range and recompute completeness.
    pub fn add_range(&mut self, start: usize, end: usize) {
        let start = start.max(1);
        let end = end.max(start);
        self.ranges.push((start, end));
        self.ranges.sort_unstable();
        self.recompute_complete();
    }

    /// Record the whole representation in one shot.
    pub fn mark_whole(&mut self, units: usize) {
        self.ranges.clear();
        if units > 0 {
            self.ranges.push((1, units));
        }
        self.recompute_complete();
    }

    pub fn note_truncated(&mut self) {
        self.truncated = true;
        self.complete = false;
    }

    /// Merge overlapping/adjacent ranges. Returns the merged set.
    pub fn merged_ranges(&self) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = Vec::new();
        for (start, end) in &self.ranges {
            match out.last_mut() {
                Some(last) if *start <= last.1.saturating_add(1) => {
                    last.1 = last.1.max(*end);
                }
                _ => out.push((*start, *end)),
            }
        }
        out
    }

    /// Recompute `complete` from the recorded ranges. Callers may change
    /// `ranges`/`total_units` directly and then call this.
    pub fn recompute_complete(&mut self) {
        if self.truncated {
            self.complete = false;
            return;
        }
        let Some(total) = self.total_units else {
            self.complete = false;
            return;
        };
        if total == 0 {
            // An empty representation has nothing to lose; reading it is
            // complete by definition.
            self.complete = true;
            return;
        }
        let merged = self.merged_ranges();
        self.complete = merged.len() == 1 && merged[0].0 == 1 && merged[0].1 >= total;
    }

    /// Human-readable range summary for conflict messages.
    pub fn range_label(&self) -> String {
        if self.ranges.is_empty() {
            return "未收到内容".to_owned();
        }
        let merged = self.merged_ranges();
        let parts: Vec<String> = merged
            .iter()
            .map(|(start, end)| {
                if start == end {
                    format!("第 {start} 行")
                } else {
                    format!("第 {start}-{end} 行")
                }
            })
            .collect();
        match self.total_units {
            Some(total) => format!("{}（共 {total} 行）", parts.join("、")),
            None => parts.join("、"),
        }
    }
}

/// Per-turn receipts keyed by normalized project-relative path.
#[derive(Debug, Default, Clone)]
pub struct ReadLedger {
    receipts: BTreeMap<String, ReadReceipt>,
}

impl ReadLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Model-provided paths drift (`./src/a.ts` vs `src/a.ts`); the reader and
    /// the writer must land on the same key.
    pub fn key(path: &str) -> String {
        let trimmed = path.trim();
        let trimmed = trimmed.strip_prefix("./").unwrap_or(trimmed);
        trimmed.trim_start_matches('/').to_owned()
    }

    pub fn get(&self, path: &str) -> Option<&ReadReceipt> {
        self.receipts.get(&Self::key(path))
    }

    /// Record a delivered read. A receipt for a different version replaces the
    /// old one — ranges from two versions can never form a complete read.
    pub fn record(&mut self, receipt: ReadReceipt) {
        let key = Self::key(&receipt.path);
        match self.receipts.get(&key) {
            Some(existing) if existing.version == receipt.version => {
                let mut merged = existing.clone();
                merged.total_units = receipt.total_units.or(merged.total_units);
                if receipt.truncated {
                    merged.truncated = true;
                }
                merged.ranges.extend(receipt.ranges.iter().copied());
                merged.ranges.sort_unstable();
                merged.recompute_complete();
                self.receipts.insert(key, merged);
            }
            _ => {
                self.receipts.insert(key, receipt);
            }
        }
    }

    /// The model authored these bytes this turn, so a rewrite is a repair.
    pub fn note_authored(
        &mut self,
        path: &str,
        representation: Representation,
        version: impl Into<String>,
        units: Option<usize>,
    ) {
        let mut receipt = ReadReceipt::new(path, representation, version);
        receipt.total_units = units;
        receipt.mark_whole(units.unwrap_or(0));
        if units == Some(0) {
            receipt.complete = true;
        }
        self.record(receipt);
    }

    /// Why a whole-file overwrite of `path` is not allowed right now, or
    /// `None` when the turn holds a complete read of the current version.
    ///
    /// `current_version` is the digest of the file on disk *now*; `None`
    /// means the file does not exist (always allowed — nothing to lose).
    pub fn overwrite_block(&self, path: &str, current_version: Option<&str>) -> Option<String> {
        let Some(current) = current_version else {
            return None;
        };
        let Some(receipt) = self.get(path) else {
            return Some(format!(
                "`{path}` 已存在，本轮还没有读取过它。先 read_file 看清现有内容再整体重写；\
                 只改局部请改用 apply_patch 或 replace_range。"
            ));
        };
        if receipt.version != current {
            return Some(format!(
                "`{path}` 在本轮读取后已被修改（读取版本 {}，当前版本 {current}）。\
                 整体重写会覆盖未看到的内容：请重新 read_file 确认最新内容，\
                 或用 apply_patch 以唯一锚点做局部修改。",
                receipt.version
            ));
        }
        if !receipt.complete {
            return Some(format!(
                "`{path}` 本轮只收到 {} 的内容，不是完整读取，整体重写会删除未读部分。\
                 用 read_range 读完剩余范围后再整体重写，或用 apply_patch / replace_range 局部修改。",
                receipt.range_label()
            ));
        }
        None
    }

    pub fn clear(&mut self) {
        self.receipts.clear();
    }
}

/// FNV-1a 64 over the bytes plus the byte length.
///
/// Change detection only: the goal is "same bytes as the read?" not
/// "cryptographically unforgeable". Documented here so no caller mistakes it
/// for a security boundary.
pub fn version_of(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}:{}", bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(path: &str, version: &str, total: usize) -> ReadReceipt {
        ReadReceipt::new(path, Representation::RawText, version).with_total(Some(total))
    }

    #[test]
    fn partial_range_is_not_complete() {
        let mut r = receipt("doc.md", "v1", 201);
        r.add_range(1, 120);
        assert!(!r.complete);
        assert!(r.range_label().contains("第 1-120 行"));
    }

    #[test]
    fn contiguous_ranges_over_same_version_complete_the_read() {
        let mut ledger = ReadLedger::new();
        let mut first = receipt("doc.md", "v1", 201);
        first.add_range(1, 120);
        ledger.record(first);
        assert!(ledger
            .overwrite_block("doc.md", Some("v1"))
            .expect("partial")
            .contains("不是完整读取"));

        let mut second = receipt("doc.md", "v1", 201);
        second.add_range(121, 201);
        ledger.record(second);
        assert_eq!(ledger.overwrite_block("doc.md", Some("v1")), None);
    }

    #[test]
    fn ranges_from_a_new_version_never_merge() {
        let mut ledger = ReadLedger::new();
        let mut first = receipt("doc.md", "v1", 201);
        first.add_range(1, 120);
        ledger.record(first);
        let mut second = receipt("doc.md", "v2", 201);
        second.add_range(121, 201);
        ledger.record(second);
        let block = ledger
            .overwrite_block("doc.md", Some("v2"))
            .expect("stale ranges must not complete");
        assert!(block.contains("不是完整读取"), "{block}");
    }

    #[test]
    fn truncated_read_never_completes_even_over_the_whole_range() {
        let mut r = receipt("doc.md", "v1", 10);
        r.add_range(1, 10);
        assert!(r.complete);
        r.note_truncated();
        assert!(!r.complete);
    }

    #[test]
    fn version_change_after_read_is_refused() {
        let mut ledger = ReadLedger::new();
        let mut r = receipt("doc.md", "v1", 3);
        r.add_range(1, 3);
        ledger.record(r);
        let block = ledger
            .overwrite_block("doc.md", Some("v2"))
            .expect("version change");
        assert!(block.contains("已被修改"), "{block}");
    }

    #[test]
    fn unknown_path_and_absent_file() {
        let ledger = ReadLedger::new();
        assert!(ledger.overwrite_block("new.md", None).is_none());
        assert!(ledger.overwrite_block("old.md", Some("v1")).is_some());
    }

    #[test]
    fn sight_key_normalizes_dot_prefix() {
        assert_eq!(ReadLedger::key("./src/a.ts"), "src/a.ts");
        assert_eq!(ReadLedger::key(" src/a.ts "), "src/a.ts");
    }

    #[test]
    fn version_digest_detects_change() {
        assert_eq!(version_of(b"abc"), version_of(b"abc"));
        assert_ne!(version_of(b"abc"), version_of(b"abd"));
        assert_ne!(version_of(b"abc"), version_of(b"abc "));
    }
}
