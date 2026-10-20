//! Attachment readability: one vocabulary for the user and the model.
//!
//! Before this module the two sides spoke different languages. The model got a
//! trailing line inside the tool result (`共 201 行，本次显示 1-120`), while the
//! UI showed whatever the view layer could infer from the step kind. Neither
//! could answer the only question that matters when a read is partial: *can I
//! continue from here, and from which version?*
//!
//! [`Readability`] is that answer. [`describe`] is the single function that
//! turns a read into a state **and** the sentence both sides show, so a partial
//! read cannot look complete in one place and partial in another.
//!
//! [`AttachmentScope`] is the other half: a projectless session may only read
//! the attachments the user actually attached, and only while the handle is
//! fresh and the file has not changed underneath. Granting is restricted to a
//! set of roots, so a handle can never be widened into "read the directory".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::read_receipt::{ReadReceipt, Representation};

/// What a reader (human or model) can conclude from one read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readability {
    /// The whole representation was delivered.
    Complete,
    /// A contiguous window of a larger representation. The rest is reachable
    /// by continuing from the same version.
    Partial,
    /// Extraction hit its own budget (DOCX XML / workbook size), so the text
    /// itself is incomplete and continuing will not reach the remainder.
    Clipped,
    /// The format has no text extraction at all (PDF, PPTX, legacy binary
    /// Office). Stated upfront instead of failing halfway.
    Unsupported,
    /// Extraction failed for another reason (corrupt archive, encoding).
    Unreadable,
}

impl Readability {
    /// Stable machine tag. The UI switches on this; it never parses prose.
    pub fn as_str(self) -> &'static str {
        match self {
            Readability::Complete => "complete",
            Readability::Partial => "partial",
            Readability::Clipped => "clipped",
            Readability::Unsupported => "unsupported",
            Readability::Unreadable => "unreadable",
        }
    }

    /// True when continuing the read can still reach the rest of the content.
    pub fn can_continue(self) -> bool {
        matches!(self, Readability::Partial)
    }

    /// True when the reader has seen the entire representation.
    pub fn is_whole(self) -> bool {
        matches!(self, Readability::Complete)
    }
}

/// One attachment's readability, in the shape both the UI and the prompt use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadabilityView {
    pub path: String,
    pub representation: &'static str,
    pub state: Readability,
    pub version: String,
    pub total_units: Option<usize>,
    pub shown: Option<(usize, usize)>,
    /// The sentence the model sees appended to the tool result *and* the one
    /// the UI shows. Never two different phrasings of the same fact.
    pub sentence: String,
}

/// The one place a read is turned into a state plus its sentence.
///
/// `truncated` means extraction hit its budget, which is strictly worse than a
/// window: the missing content is not reachable by continuing.
pub fn describe(
    representation: Representation,
    total_units: Option<usize>,
    shown: (usize, usize),
    truncated: bool,
) -> (Readability, String) {
    let state = state_for(total_units, shown, truncated);
    (
        state,
        sentence_for(state, representation, total_units, shown),
    )
}

fn state_for(total_units: Option<usize>, shown: (usize, usize), truncated: bool) -> Readability {
    let (first, last) = shown;
    if truncated {
        return Readability::Clipped;
    }
    if total_units == Some(0) {
        return Readability::Complete;
    }
    if total_units.is_some_and(|total| first <= 1 && last >= total) {
        return Readability::Complete;
    }
    if last < first {
        // Nothing was shown at all (empty extraction).
        return Readability::Clipped;
    }
    Readability::Partial
}

fn sentence_for(
    state: Readability,
    representation: Representation,
    total_units: Option<usize>,
    shown: (usize, usize),
) -> String {
    let (first, last) = shown;
    let noun = representation.unit_noun();
    match state {
        Readability::Complete => match total_units {
            Some(0) => "已完整读取（内容为空）".to_string(),
            Some(total) => format!("已完整读取全部 {total} {noun}"),
            None => "已完整读取".to_string(),
        },
        Readability::Partial => match total_units {
            Some(total) => format!(
                "本次读取第 {first}-{last} {noun}（共 {total} {noun}）；可在同一版本下继续读取剩余部分"
            ),
            None => format!("本次读取第 {first}-{last} {noun}；可在同一版本下继续读取剩余部分"),
        },
        Readability::Clipped => match total_units {
            Some(total) => format!(
                "本次读取第 {first}-{last} {noun}（共 {total} {noun}），但本次读取已按预算截断；剩余部分无法继续读取"
            ),
            None => format!("本次读取第 {first}-{last} {noun}，但本次读取已按预算截断；剩余部分无法继续读取"),
        },
        Readability::Unsupported => format!("{noun}格式暂不支持读取"),
        Readability::Unreadable => format!("{noun}读取失败"),
    }
}

/// Project a recorded receipt into the shared view.
///
/// Completeness comes from the receipt, not from the first window: a turn that
/// read 1-120 and then 121-201 has seen the whole file even though no single
/// call did.
pub fn view_of(path: &str, receipt: &ReadReceipt) -> ReadabilityView {
    let shown = receipt.merged_ranges().first().copied().unwrap_or((0, 0));
    let state = if receipt.truncated {
        Readability::Clipped
    } else if receipt.complete {
        Readability::Complete
    } else {
        state_for(receipt.total_units, shown, receipt.truncated)
    };
    let sentence = sentence_for(state, receipt.representation, receipt.total_units, shown);
    ReadabilityView {
        path: path.to_string(),
        representation: receipt.representation.label(),
        state,
        version: receipt.version.clone(),
        total_units: receipt.total_units,
        shown: Some(shown),
        sentence,
    }
}

/// Formats the reader cannot extract text from. Stated upfront (T023) so the
/// user is not told "done" after a write nothing can verify.
pub fn unsupported_format(path: &str) -> Option<&'static str> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => Some("PDF"),
        "pptx" | "ppt" => Some("PPTX"),
        "doc" | "xls" => Some("旧版二进制 Office"),
        _ => None,
    }
}

/// Why a continuation was refused. Each variant is a distinct user-visible
/// reason — "expired" and "changed" are not the same repair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContinuationError {
    /// No handle was ever granted for this path.
    Unknown,
    /// The handle's window closed.
    Expired,
    /// The file changed since the handle was granted.
    VersionChanged { granted: String, current: String },
    /// The path is outside the granted roots (a directory pivot).
    OutOfScope,
}

impl ContinuationError {
    pub fn sentence(&self, path: &str) -> String {
        match self {
            ContinuationError::Unknown => {
                format!("`{path}` 不是本次会话可读的附件；请让用户重新附加该文件")
            }
            ContinuationError::Expired => {
                format!("`{path}` 的读取句柄已过期；请重新附加或重新读取")
            }
            ContinuationError::VersionChanged { granted, current } => format!(
                "`{path}` 在读取后已被修改（读取版本 {granted}，当前版本 {current}）；请重新读取后再继续"
            ),
            ContinuationError::OutOfScope => {
                format!("`{path}` 不在本次允许读取的附件范围内；不能通过续读访问目录或其它文件")
            }
        }
    }
}

/// One grant: a path, the version it was read at, and when it stops being
/// valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentHandle {
    pub path: String,
    pub version: String,
    pub expires_at: u64,
}

/// The set of attachments a projectless session may read.
///
/// A project session reads through the project root; a projectless one has no
/// root, so the only thing that may be read is what the user attached. Grants
/// are restricted to `roots` and each grant carries a version and a deadline,
/// which is what makes "continue reading" safe to offer.
#[derive(Debug, Clone, Default)]
pub struct AttachmentScope {
    roots: Vec<PathBuf>,
    handles: BTreeMap<String, AttachmentHandle>,
    ttl_seconds: u64,
}

impl AttachmentScope {
    pub fn new(roots: Vec<PathBuf>, ttl_seconds: u64) -> Self {
        Self {
            // Roots are canonicalised once, because grants are compared
            // against canonical file paths. On macOS `/var` is a symlink to
            // `/private/var`, so an uncanonicalised root would reject every
            // file under it. A root that does not exist yet keeps its literal
            // form and simply matches nothing.
            roots: roots
                .into_iter()
                .map(|root| canonical(&root).unwrap_or(root))
                .collect(),
            handles: BTreeMap::new(),
            ttl_seconds,
        }
    }

    /// Restrict a project session to its root (the common case).
    pub fn for_project(root: &Path, ttl_seconds: u64) -> Self {
        Self::new(vec![root.to_path_buf()], ttl_seconds)
    }

    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    pub fn len(&self) -> usize {
        self.handles.len()
    }

    /// Grant a handle for `path` at `version`, valid for the scope's TTL.
    ///
    /// Refuses anything outside the roots — a grant is not a way to widen the
    /// readable set, only to name a file that is already inside it.
    pub fn grant(&mut self, path: &Path, version: &str, now: u64) -> Result<(), String> {
        let full = canonical(path)?;
        if !self.roots.is_empty() && !self.roots.iter().any(|root| full.starts_with(root)) {
            return Err(format!(
                "`{}` 不在允许读取的附件范围内",
                path.to_string_lossy()
            ));
        }
        let key = key_of(&full);
        self.handles.insert(
            key.clone(),
            AttachmentHandle {
                path: key,
                version: version.to_owned(),
                expires_at: now.saturating_add(self.ttl_seconds),
            },
        );
        Ok(())
    }

    /// Resolve a continuation. Order matters: scope, then expiry, then version.
    pub fn continue_read(
        &self,
        path: &Path,
        current_version: &str,
        now: u64,
    ) -> Result<&AttachmentHandle, ContinuationError> {
        let full = canonical(path).map_err(|_| ContinuationError::OutOfScope)?;
        if !self.roots.is_empty() && !self.roots.iter().any(|root| full.starts_with(root)) {
            return Err(ContinuationError::OutOfScope);
        }
        let handle = self
            .handles
            .get(&key_of(&full))
            .ok_or(ContinuationError::Unknown)?;
        if now >= handle.expires_at {
            return Err(ContinuationError::Expired);
        }
        if handle.version != current_version {
            return Err(ContinuationError::VersionChanged {
                granted: handle.version.clone(),
                current: current_version.to_owned(),
            });
        }
        Ok(handle)
    }

    /// Drop every handle (a new turn starts from the attachments it was given).
    pub fn clear(&mut self) {
        self.handles.clear();
    }
}

fn key_of(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Canonicalise, mapping "missing" to an error: an absent file cannot be
/// granted or continued, and resolving it as a bare path would let a relative
/// traversal look in-scope.
fn canonical(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kodo-attach-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    fn receipt(
        representation: Representation,
        total: Option<usize>,
        shown: (usize, usize),
        truncated: bool,
    ) -> ReadReceipt {
        let mut receipt = ReadReceipt::new("a.txt", representation, "v1").with_total(total);
        if shown.1 >= shown.0 && shown.1 > 0 {
            receipt.add_range(shown.0, shown.1);
        }
        if truncated {
            receipt.note_truncated();
        }
        receipt.recompute_complete();
        receipt
    }

    #[test]
    fn a_window_is_partial_and_says_it_can_continue() {
        let (state, sentence) = describe(Representation::RawText, Some(201), (1, 120), false);
        assert_eq!(state, Readability::Partial);
        assert!(state.can_continue());
        assert!(!state.is_whole());
        assert!(sentence.contains("1-120"), "{sentence}");
        assert!(sentence.contains("201"), "{sentence}");
        assert!(sentence.contains("继续读取"), "{sentence}");
    }

    #[test]
    fn a_whole_read_is_complete() {
        let (state, sentence) = describe(Representation::RawText, Some(201), (1, 201), false);
        assert_eq!(state, Readability::Complete);
        assert!(state.is_whole());
        assert!(sentence.contains("完整读取"), "{sentence}");
    }

    #[test]
    fn a_clipped_extraction_cannot_be_continued() {
        let (state, sentence) = describe(Representation::DocxText, Some(400), (1, 120), true);
        assert_eq!(state, Readability::Clipped);
        assert!(!state.can_continue());
        assert!(sentence.contains("无法继续读取"), "{sentence}");
    }

    #[test]
    fn an_empty_document_is_complete_not_partial() {
        let (state, _) = describe(Representation::RawText, Some(0), (1, 0), false);
        assert_eq!(state, Readability::Complete);
    }

    #[test]
    fn the_view_and_the_tool_result_use_the_same_sentence() {
        let receipt = receipt(Representation::RawText, Some(201), (1, 120), false);
        let view = view_of("a.txt", &receipt);
        let (_, sentence) = describe(Representation::RawText, Some(201), (1, 120), false);
        assert_eq!(view.sentence, sentence);
        assert_eq!(view.state, Readability::Partial);
        assert_eq!(view.representation, "raw_text");
        assert_eq!(view.version, "v1");
    }

    #[test]
    fn accumulated_windows_project_as_complete() {
        let mut receipt =
            ReadReceipt::new("a.txt", Representation::RawText, "v1").with_total(Some(201));
        receipt.add_range(1, 120);
        receipt.add_range(121, 201);
        receipt.recompute_complete();
        let view = view_of("a.txt", &receipt);
        assert_eq!(view.state, Readability::Complete);
        assert!(view.sentence.contains("201"), "{}", view.sentence);
    }

    #[test]
    fn unsupported_formats_are_named_upfront() {
        assert_eq!(unsupported_format("a.pdf"), Some("PDF"));
        assert_eq!(unsupported_format("A.PPTX"), Some("PPTX"));
        assert_eq!(unsupported_format("a.doc"), Some("旧版二进制 Office"));
        assert_eq!(unsupported_format("a.md"), None);
        assert_eq!(unsupported_format("a.docx"), None);
    }

    #[test]
    fn a_grant_outside_the_roots_is_refused() {
        let root = scratch("scope-root");
        let outside = scratch("scope-outside");
        let inside_file = root.join("in.md");
        std::fs::write(&inside_file, "x").unwrap();
        let outside_file = outside.join("out.md");
        std::fs::write(&outside_file, "x").unwrap();

        let mut scope = AttachmentScope::for_project(&root, 60);
        scope.grant(&inside_file, "v1", 100).expect("inside");
        let error = scope
            .grant(&outside_file, "v1", 100)
            .expect_err("outside must be refused");
        assert!(error.contains("不在允许读取的附件范围内"), "{error}");
        assert_eq!(scope.len(), 1);
    }

    #[test]
    fn a_continuation_needs_a_live_handle_of_the_same_version() {
        let root = scratch("scope-continue");
        let file = root.join("in.md");
        std::fs::write(&file, "x").unwrap();
        let mut scope = AttachmentScope::for_project(&root, 60);
        scope.grant(&file, "v1", 1_000).expect("grant");

        // Same version, inside the TTL: allowed.
        assert!(scope.continue_read(&file, "v1", 1_030).is_ok());
        // Past the TTL.
        assert_eq!(
            scope.continue_read(&file, "v1", 1_060).unwrap_err(),
            ContinuationError::Expired
        );
        // Changed on disk.
        assert_eq!(
            scope.continue_read(&file, "v2", 1_030).unwrap_err(),
            ContinuationError::VersionChanged {
                granted: "v1".into(),
                current: "v2".into()
            }
        );
        // Never granted.
        let other = root.join("other.md");
        std::fs::write(&other, "x").unwrap();
        assert_eq!(
            scope.continue_read(&other, "v1", 1_030).unwrap_err(),
            ContinuationError::Unknown
        );
    }

    #[test]
    fn a_continuation_cannot_pivot_to_a_sibling_directory() {
        let root = scratch("scope-pivot");
        let nested = root.join("attachments");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("in.md");
        std::fs::write(&file, "x").unwrap();
        let mut scope = AttachmentScope::new(vec![nested.clone()], 60);
        scope.grant(&file, "v1", 1_000).expect("grant");

        // A sibling directory is outside the granted root, even though it is
        // under the user's home and exists.
        let sibling = root.join("secrets");
        std::fs::create_dir_all(&sibling).unwrap();
        let secret = sibling.join("token.txt");
        std::fs::write(&secret, "x").unwrap();
        assert_eq!(
            scope.continue_read(&secret, "v1", 1_030).unwrap_err(),
            ContinuationError::OutOfScope
        );
        // Traversal through the granted file's parent is refused too.
        let escape = nested.join("../secrets/token.txt");
        assert!(scope.continue_read(&escape, "v1", 1_030).is_err());
    }

    #[test]
    fn every_refusal_explains_itself() {
        let cases = [
            ContinuationError::Unknown,
            ContinuationError::Expired,
            ContinuationError::VersionChanged {
                granted: "a".into(),
                current: "b".into(),
            },
            ContinuationError::OutOfScope,
        ];
        for case in cases {
            let sentence = case.sentence("x.md");
            assert!(sentence.contains("x.md"), "{sentence}");
            assert!(
                sentence.contains("请") || sentence.contains("不能"),
                "{sentence}"
            );
        }
    }
}
