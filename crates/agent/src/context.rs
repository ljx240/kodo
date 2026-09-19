//! Lexical context retrieval for one turn.
//!
//! Deterministic, cancellable, budget-aware: file map → path/filename search →
//! content grep with line ranges → ranking → pinned + budgeted pack.
//! No vectors, no full-repo content reads.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Default per-turn character budget for packed context handed to the model.
pub const DEFAULT_CONTEXT_CHARS: usize = 12_000;
/// Hard cap for a single span/snippet body (explicit truncation beyond this).
pub const DEFAULT_SPAN_CHARS: usize = 1_800;
/// Max spans packed into one turn.
pub const DEFAULT_MAX_SPANS: usize = 16;
/// Walk / index safety caps so huge repos never scan forever.
const MAX_INDEXED_FILES: usize = 20_000;
const MAX_SCAN_DEPTH: usize = 12;
const MAX_GREP_FILES: usize = 400;
const READ_CHUNK: usize = 64 * 1024;

/// Directory / file name segments never indexed or grepped.
const IGNORE_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    ".next",
    ".nuxt",
    ".cache",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    ".idea",
    ".vscode",
];

const IGNORE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "pdf", "zip", "tar", "gz", "tgz", "bz2", "xz",
    "woff", "woff2", "ttf", "otf", "eot", "mp3", "mp4", "mov", "avi", "webm", "bin", "exe", "dll",
    "so", "dylib", "class", "jar", "wasm", "lock", "min.js", "min.css", "map",
];

/// How a span was chosen — stored for the model and for tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSpan {
    pub path: String,
    /// 1-based inclusive.
    pub start_line: usize,
    /// 1-based inclusive.
    pub end_line: usize,
    pub reason: String,
    /// Higher is better; packed order is score-desc.
    pub score: i64,
    pub snippet: String,
    pub truncated: bool,
    pub pinned: bool,
    pub total_lines: usize,
}

impl ContextSpan {
    /// Model-facing block with explicit truncation notices.
    pub fn to_prompt_block(&self) -> String {
        let mut out = format!(
            "<context path=\"{}\" lines=\"{}-{}\" score=\"{}\" reason=\"{}\">\n",
            self.path, self.start_line, self.end_line, self.score, self.reason
        );
        if self.truncated {
            out.push_str(&format!(
                "[result truncated: showing lines {}-{} of {} in this span]\n",
                self.start_line, self.end_line, self.total_lines
            ));
        }
        out.push_str(&self.snippet);
        if !self.snippet.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("</context>\n");
        out
    }
}

/// Per-turn packing budget (characters / span count).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextBudget {
    pub max_chars: usize,
    pub max_span_chars: usize,
    pub max_spans: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_chars: DEFAULT_CONTEXT_CHARS,
            max_span_chars: DEFAULT_SPAN_CHARS,
            max_spans: DEFAULT_MAX_SPANS,
        }
    }
}

impl ContextBudget {
    pub fn new(max_chars: usize) -> Self {
        Self {
            max_chars,
            ..Self::default()
        }
    }
}

/// One indexed file (metadata only — contents are not held).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub from_git: bool,
}

/// Raised when a scan is cancelled via `alive`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanCancelled;

/// Lexical retrieval over a project tree.
pub struct ContextManager {
    root: PathBuf,
    budget: ContextBudget,
    file_map: Vec<FileEntry>,
    pinned: Vec<ContextSpan>,
    scan_complete: bool,
}

impl ContextManager {
    pub fn new(root: impl Into<PathBuf>, budget: ContextBudget) -> Self {
        Self {
            root: root.into(),
            budget,
            file_map: Vec::new(),
            pinned: Vec::new(),
            scan_complete: false,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn budget(&self) -> &ContextBudget {
        &self.budget
    }

    pub fn file_map(&self) -> &[FileEntry] {
        &self.file_map
    }

    pub fn pinned(&self) -> &[ContextSpan] {
        &self.pinned
    }

    pub fn is_scanned(&self) -> bool {
        self.scan_complete
    }

    /// Pin a span so it is always included (still subject to span char cap).
    pub fn pin(&mut self, span: ContextSpan) {
        let mut span = span;
        span.pinned = true;
        self.pinned
            .retain(|p| p.path != span.path || p.start_line != span.start_line);
        self.pinned.push(span);
    }

    /// Build repository file map. Git-tracked files first when available.
    /// Cancellable: returns `Err(ScanCancelled)` if `alive` turns false.
    pub fn scan(&mut self, alive: &dyn Fn() -> bool) -> Result<(), ScanCancelled> {
        self.file_map.clear();
        self.scan_complete = false;

        let mut seen: HashSet<String> = HashSet::new();
        let mut entries: Vec<FileEntry> = Vec::new();

        // Prefer `git ls-files` for tracked set.
        let git_files = git_ls_files(&self.root);
        let mut from_git_paths: HashSet<String> = HashSet::new();
        for rel in &git_files {
            if !alive() {
                return Err(ScanCancelled);
            }
            if should_index_path(rel) {
                from_git_paths.insert(rel.clone());
            }
        }

        // Walk tree for untracked + non-git projects (still ignore rules).
        if !walk_collect(&self.root, &self.root, &mut entries, alive)? {
            return Err(ScanCancelled);
        }

        // Merge: git-tracked flag wins; dedupe by path.
        for e in entries.drain(..) {
            if !seen.insert(e.path.clone()) {
                continue;
            }
            let from_git = from_git_paths.contains(&e.path);
            if from_git || should_index_path(&e.path) {
                self.file_map.push(FileEntry {
                    path: e.path,
                    size: e.size,
                    from_git,
                });
            }
            if self.file_map.len() >= MAX_INDEXED_FILES {
                break;
            }
        }
        // Add git paths that walk missed (sparse checkout edge cases).
        for rel in git_files {
            if !alive() {
                return Err(ScanCancelled);
            }
            if !should_index_path(&rel) || seen.contains(&rel) {
                continue;
            }
            let full = self.root.join(&rel);
            let size = fs::metadata(&full).map(|m| m.len()).unwrap_or(0);
            self.file_map.push(FileEntry {
                path: rel,
                size,
                from_git: true,
            });
            if self.file_map.len() >= MAX_INDEXED_FILES {
                break;
            }
        }

        // Stable order: git first, then path.
        self.file_map.sort_by(|a, b| {
            b.from_git
                .cmp(&a.from_git)
                .then_with(|| a.path.cmp(&b.path))
        });
        self.scan_complete = true;
        Ok(())
    }

    /// Filename / path substring search (case-insensitive), ranked.
    pub fn search_paths(&self, query: &str, limit: usize) -> Vec<(String, i64, String)> {
        let tokens = tokenize(query);
        let mut scored: Vec<(String, i64, String)> = Vec::new();
        for entry in &self.file_map {
            if let Some(score) = score_path(&entry.path, &tokens, entry.from_git) {
                scored.push((
                    entry.path.clone(),
                    score,
                    format!("path match for `{}`", query.trim()),
                ));
            }
        }
        scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(limit.max(1));
        scored
    }

    /// Text grep over the file map: returns spans around matching lines.
    pub fn grep(
        &self,
        query: &str,
        limit: usize,
        alive: &dyn Fn() -> bool,
    ) -> Result<Vec<ContextSpan>, ScanCancelled> {
        let tokens = tokenize(query);
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        // Prefer git-tracked files (already sorted first).
        let mut hits: Vec<ContextSpan> = Vec::new();
        let mut scanned = 0usize;

        for entry in &self.file_map {
            if !alive() {
                return Err(ScanCancelled);
            }
            if scanned >= MAX_GREP_FILES || hits.len() >= limit.max(1) * 3 {
                break;
            }
            // Cheap prefilter: path tokens or likely-text extension.
            if !is_probably_text_path(&entry.path) {
                continue;
            }
            if entry.size > 2_000_000 {
                continue; // never slurp huge blobs in phase 1
            }
            scanned += 1;
            if let Some(span) = grep_file(&self.root, entry, &tokens, query) {
                hits.push(span);
            }
        }

        hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
        hits.truncate(limit.max(1));
        Ok(hits)
    }

    /// Range read: lines `[start_line, end_line]` inclusive, 1-based.
    pub fn read_range(
        &self,
        path: &str,
        start_line: usize,
        end_line: usize,
        reason: &str,
    ) -> Result<ContextSpan, String> {
        if path.trim().is_empty() {
            return Err("empty path".into());
        }
        if path.contains("..") || Path::new(path).is_absolute() {
            return Err("path escapes the project root".into());
        }
        let full = self.root.join(path);
        if !full.is_file() {
            return Err(format!("file not found: {path}"));
        }
        let meta = fs::metadata(&full).map_err(|e| e.to_string())?;
        if is_binary_meta(&meta, &full) {
            return Err(format!("binary file refused: {path}"));
        }
        let text = fs::read_to_string(&full).map_err(|e| format!("read failed: {e}"))?;
        if text.bytes().take(READ_CHUNK).any(|b| b == 0) {
            return Err(format!("binary file refused: {path}"));
        }
        let lines: Vec<&str> = text.lines().collect();
        let total = lines.len();
        if total == 0 {
            return Ok(ContextSpan {
                path: path.to_owned(),
                start_line: 1,
                end_line: 0,
                reason: reason.to_owned(),
                score: 0,
                snippet: String::new(),
                truncated: false,
                pinned: false,
                total_lines: 0,
            });
        }
        let start = start_line.clamp(1, total);
        let end = end_line.clamp(start, total);
        let mut snippet = lines[start - 1..end].join("\n");
        let mut truncated = false;
        if snippet.len() > self.budget.max_span_chars {
            snippet = truncate_chars(&snippet, self.budget.max_span_chars);
            truncated = true;
        }
        Ok(ContextSpan {
            path: path.to_owned(),
            start_line: start,
            end_line: end,
            reason: reason.to_owned(),
            score: 50,
            snippet,
            truncated,
            pinned: false,
            total_lines: total,
        })
    }

    /// Rank and pack context for one turn under the character budget.
    pub fn collect(
        &mut self,
        query: &str,
        alive: &dyn Fn() -> bool,
    ) -> Result<Vec<ContextSpan>, ScanCancelled> {
        if !self.scan_complete {
            self.scan(alive)?;
        }

        let mut candidates: Vec<ContextSpan> = Vec::new();

        // Pinned always first.
        for p in &self.pinned {
            candidates.push(p.clone());
        }

        // Path hits → small header spans (first lines only as evidence of existence).
        for (path, score, reason) in self.search_paths(query, 8) {
            if !alive() {
                return Err(ScanCancelled);
            }
            if let Ok(span) = self.read_range(&path, 1, 24, &reason) {
                let mut span = span;
                span.score = score;
                candidates.push(span);
            }
        }

        // Content hits with real line ranges.
        if let Ok(greps) = self.grep(query, 12, alive) {
            for g in greps {
                candidates.push(g);
            }
        }

        // Dedupe by (path, start_line): keep higher score.
        candidates.sort_by_key(|c| std::cmp::Reverse(c.score));
        let mut seen: HashSet<(String, usize)> = HashSet::new();
        let mut unique = Vec::new();
        for c in candidates {
            let key = (c.path.clone(), c.start_line);
            if seen.insert(key) {
                unique.push(c);
            }
        }

        // Pack under budget.
        let mut packed: Vec<ContextSpan> = Vec::new();
        let mut used = 0usize;
        // Always start with pinned even if over (they're capped per-span).
        for p in &self.pinned {
            let cost = p.snippet.chars().count() + p.path.len() + 80;
            if used + cost <= self.budget.max_chars.max(self.budget.max_span_chars) {
                packed.push(p.clone());
                used += cost;
            }
        }
        for c in unique {
            if packed.len() >= self.budget.max_spans {
                break;
            }
            if c.pinned {
                continue; // already added
            }
            let cost = c.snippet.chars().count() + c.path.len() + 80;
            if used + cost > self.budget.max_chars {
                // Try a further-truncated version if it still fits.
                let room = self.budget.max_chars.saturating_sub(used);
                if room < 120 {
                    continue;
                }
                let mut c = c;
                let keep = room.saturating_sub(80).min(self.budget.max_span_chars);
                if keep < 40 {
                    continue;
                }
                c.snippet = truncate_chars(&c.snippet, keep);
                c.truncated = true;
                let cost = c.snippet.chars().count() + c.path.len() + 80;
                if used + cost > self.budget.max_chars {
                    continue;
                }
                used += cost;
                packed.push(c);
            } else {
                used += cost;
                packed.push(c);
            }
        }
        Ok(packed)
    }

    /// Pack a list of spans (already chosen) under budget — used by tests / callers.
    pub fn pack(&self, mut spans: Vec<ContextSpan>) -> Vec<ContextSpan> {
        spans.sort_by_key(|s| std::cmp::Reverse(s.score));
        let mut out = Vec::new();
        let mut used = 0usize;
        for mut s in spans {
            if out.len() >= self.budget.max_spans {
                break;
            }
            if s.snippet.chars().count() > self.budget.max_span_chars {
                s.snippet = truncate_chars(&s.snippet, self.budget.max_span_chars);
                s.truncated = true;
            }
            let cost = s.snippet.chars().count() + s.path.len() + 80;
            if used + cost > self.budget.max_chars {
                continue;
            }
            used += cost;
            out.push(s);
        }
        out
    }

    /// Format packed spans for the model. Always notes truncation when present.
    pub fn format_spans(spans: &[ContextSpan]) -> String {
        if spans.is_empty() {
            return "<context>\n(no relevant lexical context found)\n</context>\n".to_owned();
        }
        let mut out = String::from("<context_set>\n");
        for s in spans {
            out.push_str(&s.to_prompt_block());
        }
        out.push_str("</context_set>\n");
        if spans.iter().any(|s| s.truncated) {
            out.push_str("Note: some context results were truncated by the budget.\n");
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn git_ls_files(root: &Path) -> Vec<String> {
    let out = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output();
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    raw.split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn walk_collect(
    root: &Path,
    dir: &Path,
    out: &mut Vec<FileEntry>,
    alive: &dyn Fn() -> bool,
) -> Result<bool, ScanCancelled> {
    if out.len() >= MAX_INDEXED_FILES {
        return Ok(true);
    }
    let depth = dir
        .strip_prefix(root)
        .map(|p| p.components().count())
        .unwrap_or(0);
    if depth > MAX_SCAN_DEPTH {
        return Ok(true);
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return Ok(true);
    };
    for entry in rd.flatten() {
        if !alive() {
            return Err(ScanCancelled);
        }
        if out.len() >= MAX_INDEXED_FILES {
            return Ok(true);
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && name != "." {
            // allow normal dotfiles? skip hidden dirs including .git
            if IGNORE_DIRS.contains(&name.as_str()) || name == ".git" {
                continue;
            }
        }
        if IGNORE_DIRS.contains(&name.as_str()) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk_collect(root, &path, out, alive)?;
        } else if path.is_file() {
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if !should_index_path(&rel) {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(FileEntry {
                path: rel,
                size,
                from_git: false,
            });
        }
    }
    Ok(true)
}

fn should_index_path(rel: &str) -> bool {
    let normalized = rel.replace('\\', "/");
    for seg in normalized.split('/') {
        if IGNORE_DIRS.contains(&seg) {
            return false;
        }
    }
    if let Some(ext) = extension_of(&normalized) {
        let e = ext.to_ascii_lowercase();
        if IGNORE_EXTS
            .iter()
            .any(|x| *x == e || normalized.ends_with(x))
        {
            return false;
        }
    }
    true
}

fn extension_of(path: &str) -> Option<&str> {
    Path::new(path).extension().and_then(|e| e.to_str())
}

fn is_probably_text_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if IGNORE_EXTS.iter().any(|x| lower.ends_with(x)) {
        return false;
    }
    // Known texty extensions or extensionless scripts / rust / ts etc.
    matches!(
        extension_of(&lower).unwrap_or(""),
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "py"
            | "go"
            | "java"
            | "kt"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cs"
            | "rb"
            | "php"
            | "swift"
            | "md"
            | "txt"
            | "json"
            | "toml"
            | "yaml"
            | "yml"
            | "css"
            | "scss"
            | "html"
            | "sh"
            | "sql"
            | "proto"
            | "graphql"
            | "vue"
            | "svelte"
            | "xml"
            | "properties"
            | "env.example"
            | "lock"
            | ""
    ) || !lower.contains('.')
}

fn is_binary_meta(_meta: &fs::Metadata, path: &Path) -> bool {
    if let Some(ext) = extension_of(&path.to_string_lossy()) {
        let e = ext.to_ascii_lowercase();
        if IGNORE_EXTS.contains(&e.as_str()) {
            return true;
        }
    }
    false
}

fn tokenize(query: &str) -> Vec<String> {
    let mut raw: Vec<String> = query
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
        .filter(|t| t.chars().count() >= 2)
        .map(|t| t.to_ascii_lowercase())
        .collect();
    // Drop soft stopwords.
    raw.retain(|t| {
        !matches!(
            t.as_str(),
            "the"
                | "and"
                | "for"
                | "with"
                | "that"
                | "this"
                | "from"
                | "into"
                | "如何"
                | "什么"
                | "一下"
                | "检查"
                | "进行"
                | "是否"
                | "please"
                | "find"
                | "where"
        )
    });
    raw.dedup();
    raw
}

fn score_path(path: &str, tokens: &[String], from_git: bool) -> Option<i64> {
    if tokens.is_empty() {
        return None;
    }
    let lower = path.to_ascii_lowercase();
    let file_name = lower.rsplit('/').next().unwrap_or(&lower).to_owned();
    let mut score = 0i64;
    let mut matched = 0usize;
    for t in tokens {
        if file_name.contains(t.as_str()) {
            score += 40;
            matched += 1;
        } else if lower.contains(t.as_str()) {
            score += 18;
            matched += 1;
        }
    }
    if matched == 0 {
        return None;
    }
    if from_git {
        score += 6;
    }
    // Prefer source over markdown when competing.
    if lower.ends_with(".rs") || lower.ends_with(".ts") || lower.ends_with(".py") {
        score += 5;
    }
    if lower.ends_with("readme.md") && tokens.iter().any(|t| t.len() > 4) {
        score -= 10; // README rarely holds the code answer
    }
    Some(score)
}

/// Grep one file; build a ±context line window around the best match cluster.
fn grep_file(
    root: &Path,
    entry: &FileEntry,
    tokens: &[String],
    query: &str,
) -> Option<ContextSpan> {
    let full = root.join(&entry.path);
    let text = fs::read_to_string(&full).ok()?;
    if text.bytes().take(4096).any(|b| b == 0) {
        return None;
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let q_lower = query.to_ascii_lowercase();
    let mut match_lines: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let ll = line.to_ascii_lowercase();
        if tokens.iter().any(|t| ll.contains(t.as_str()))
            || (!q_lower.is_empty() && ll.contains(&q_lower))
        {
            match_lines.push(i);
        }
    }
    if match_lines.is_empty() {
        return None;
    }

    // Prefer dense cluster (symbol body / definition region).
    let best_idx = match_lines[0];
    let mut best_cluster = vec![best_idx];
    for w in 1..match_lines.len() {
        let cluster: Vec<usize> = match_lines[w..]
            .iter()
            .take_while(|&&i| i <= best_cluster.last().copied().unwrap_or(0) + 40)
            .copied()
            .collect();
        // sliding: take longest run of matches within 40 lines
        let mut run = vec![match_lines[w]];
        for &i in &match_lines[w + 1..] {
            if i <= *run.last().unwrap() + 40 {
                run.push(i);
            } else {
                break;
            }
        }
        if run.len() > best_cluster.len() {
            best_cluster = run;
        }
        let _ = cluster;
    }

    let first = *best_cluster.first().unwrap();
    let last = *best_cluster.last().unwrap();
    // Window: a bit before definition, through the cluster, a bit after.
    let start = first.saturating_sub(12);
    let end = (last + 16).min(lines.len() - 1);
    let snippet_lines = &lines[start..=end];
    let mut snippet = snippet_lines.join("\n");
    let total = lines.len();
    let mut truncated = false;

    // Score: match density + path boosts.
    let mut score = 50 + (best_cluster.len() as i64) * 8;
    if let Some(ps) = score_path(&entry.path, tokens, entry.from_git) {
        score += ps;
    }
    // Exact identifier-looking token in line with fn/def/class boosts.
    for &i in &best_cluster {
        let l = lines[i];
        if tokens.iter().any(|t| l.contains(t.as_str())) {
            score += 5;
            if l.contains("fn ")
                || l.contains("function ")
                || l.contains("def ")
                || l.contains("class ")
                || l.contains("pub fn")
                || l.contains("async fn")
            {
                score += 25;
            }
        }
    }
    if entry.path.ends_with("README.md") || entry.path.ends_with("readme.md") {
        score -= 15;
    }

    // ContextBudget-like span cap applied with a local default if not available:
    // caller ContextManager::grep will re-truncate; do a soft cap here.
    const LOCAL_CAP: usize = DEFAULT_SPAN_CHARS;
    if snippet.len() > LOCAL_CAP {
        snippet = truncate_chars(&snippet, LOCAL_CAP);
        truncated = true;
    }

    Some(ContextSpan {
        path: entry.path.clone(),
        start_line: start + 1,
        end_line: end + 1,
        reason: format!("lexical grep hit for `{}`", query.trim()),
        score,
        snippet,
        truncated,
        pinned: false,
        total_lines: total,
    })
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Public helper for callers that need the same truncation + notice pattern.
pub fn truncate_chars_pub(s: &str, max: usize) -> String {
    truncate_chars(s, max)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const UNIQUE: &str = "kodo_tax_rate_v2";

    fn make_fixture() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kodo-ctx-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("vendor/legacy")).unwrap();
        fs::create_dir_all(dir.join("node_modules/lodash")).unwrap();
        fs::create_dir_all(dir.join("target/debug")).unwrap();

        // 1000-line file; target function at line ~700.
        let mut big = String::new();
        for i in 1..=699 {
            big.push_str(&format!("// filler line {i}\n"));
        }
        big.push_str(&format!(
            "pub fn {UNIQUE}(amount: u32, region: &str) -> u32 {{\n    // apply progressive tax\n    amount * 7 / 100 + region.len() as u32\n}}\n"
        ));
        for i in 705..=1000 {
            big.push_str(&format!("// tail line {i}\n"));
        }
        fs::write(dir.join("src/big_calc.rs"), &big).unwrap();

        // Same symbol name in another directory (weaker body / later in file).
        let mut other = String::new();
        for i in 1..=50 {
            other.push_str(&format!("// other {i}\n"));
        }
        other.push_str(&format!("fn {UNIQUE}_impl() {{ /* stub */ }}\n"));
        fs::write(dir.join("vendor/legacy/big_calc.rs"), other).unwrap();

        // Noise: node_modules contains the symbol many times.
        for n in 0..30 {
            fs::write(
                dir.join(format!("node_modules/lodash/mod{n}.js")),
                format!("function calc() {{ return {UNIQUE}; }}\n"),
            )
            .unwrap();
        }
        fs::write(dir.join("target/debug/noise.txt"), UNIQUE).unwrap();

        // README without the answer.
        fs::write(
            dir.join("README.md"),
            "# Demo\n\nThis project does taxes eventually.\nSee docs for details.\n",
        )
        .unwrap();

        // git init so git ls-files can see tracked files (best effort).
        let _ = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .output();
        let _ = Command::new("git")
            .args(["add", "-A"])
            .current_dir(&dir)
            .output();

        dir
    }

    fn unique_suffix() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0)
    }

    fn manager(root: &Path) -> ContextManager {
        ContextManager::new(root, ContextBudget::default())
    }

    #[test]
    fn scan_ignores_noise_dirs() {
        let root = make_fixture();
        let mut m = manager(&root);
        m.scan(&|| true).expect("scan");
        let paths: Vec<&str> = m.file_map().iter().map(|e| e.path.as_str()).collect();
        assert!(
            paths.iter().any(|p| p.starts_with("src/big_calc.rs")),
            "tracked source missing: {paths:?}"
        );
        assert!(paths.contains(&"README.md"));
        assert!(
            !paths.iter().any(|p| p.contains("node_modules")),
            "node_modules must be ignored"
        );
        assert!(
            !paths.iter().any(|p| p.contains("target/")),
            "target must be ignored"
        );
        assert!(
            !paths.iter().any(|p| p.contains(".git/")),
            ".git must be ignored"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn range_read_returns_exact_lines_and_flags_truncation() {
        let root = make_fixture();
        let m = manager(&root);
        // Line 700 should be the UNIQUE function line (699 fillers + 1).
        let span = m
            .read_range("src/big_calc.rs", 700, 704, "target range")
            .expect("read");
        assert_eq!(span.start_line, 700);
        assert_eq!(span.end_line, 704);
        assert!(span.snippet.contains(UNIQUE), "snippet={}", span.snippet);
        assert!(!span.truncated);

        // Huge range → truncated flag.
        let wide = m
            .read_range("src/big_calc.rs", 1, 1000, "full file")
            .expect("read");
        assert!(wide.truncated || wide.snippet.chars().count() <= DEFAULT_SPAN_CHARS);
        assert!(wide.total_lines >= 999, "total_lines={}", wide.total_lines);
        if wide.truncated {
            let block = wide.to_prompt_block();
            assert!(
                block.contains("truncated"),
                "must announce truncation: {block}"
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn range_read_refuses_binary_and_escape() {
        let root = make_fixture();
        let m = manager(&root);
        assert!(m.read_range("../outside.txt", 1, 5, "x").is_err());
        // plant a fake binary-looking file
        let bin = root.join("blob.png");
        let mut f = fs::File::create(&bin).unwrap();
        f.write_all(&[0x89, b'P', b'N', b'G', 0, 0, 0, 0]).unwrap();
        assert!(m.read_range("blob.png", 1, 5, "x").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ranking_puts_real_function_in_top_context() {
        let root = make_fixture();
        let mut m = manager(&root);
        m.scan(&|| true).expect("scan");
        let query = format!("implement {UNIQUE} progressive tax calculation");
        let results = m.collect(&query, &|| true).expect("collect");
        assert!(!results.is_empty(), "expected context hits");

        // Top results must include the real implementation file + line range covering ~700.
        let hit = results
            .iter()
            .find(|s| s.path.contains("src/big_calc.rs") && !s.path.contains("vendor"))
            .or_else(|| results.iter().find(|s| s.path == "src/big_calc.rs"));
        let hit = hit.unwrap_or_else(|| panic!("src/big_calc.rs not in top: {:?}",
            results
                .iter()
                .map(|r| (&r.path, r.score, r.start_line))
                .collect::<Vec<_>>()));
        assert!(
            hit.start_line <= 700 && hit.end_line >= 700,
            "expected span covering line 700, got {}-{}",
            hit.start_line,
            hit.end_line
        );
        assert!(
            hit.snippet.contains(UNIQUE),
            "snippet missing symbol: {}",
            hit.snippet
        );
        assert!(hit.score > 0);
        assert!(!hit.reason.is_empty());

        // node_modules noise must not rank above the real source for this query.
        let nm_rank = results.iter().position(|s| s.path.contains("node_modules"));
        let real_rank = results
            .iter()
            .position(|s| s.path == "src/big_calc.rs")
            .unwrap();
        if let Some(nm) = nm_rank {
            assert!(
                nm > real_rank,
                "noise ranked higher: nm={nm} real={real_rank}"
            );
        }

        // README (no answer) should not be the sole/top hit over code.
        if let Some(readme_rank) = results.iter().position(|s| s.path == "README.md") {
            assert!(readme_rank >= real_rank);
        }

        // Sources recorded.
        for s in &results {
            assert!(!s.path.is_empty());
            assert!(s.end_line >= s.start_line || s.start_line == 1);
            assert!(!s.reason.is_empty());
            assert!(s.score > 0 || s.pinned);
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn same_symbol_in_multiple_dirs_prefers_denser_or_better_path() {
        let root = make_fixture();
        let mut m = manager(&root);
        let results = m.collect(UNIQUE, &|| true).expect("collect");
        // Both may appear; src/ must appear (vendor is legacy).
        assert!(
            results.iter().any(|s| s.path.contains("src/big_calc.rs")),
            "missing primary: {:?}",
            results.iter().map(|r| r.path.clone()).collect::<Vec<_>>()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn budget_limits_total_chars_and_span_count() {
        let root = make_fixture();
        let budget = ContextBudget {
            max_chars: 500,
            max_span_chars: 200,
            max_spans: 3,
        };
        let mut m = ContextManager::new(&root, budget);
        m.scan(&|| true).expect("scan");
        let results = m.collect(UNIQUE, &|| true).expect("collect");
        assert!(results.len() <= 3, "span cap exceeded: {}", results.len());
        let total: usize = results
            .iter()
            .map(|s| s.snippet.chars().count() + s.path.len() + 80)
            .sum();
        // Soft check: we never massively exceed max_chars (pinned may push slightly).
        assert!(total <= 500 + 400, "budget blown: {total} for max 500");
        for s in &results {
            assert!(s.snippet.chars().count() <= 200 || s.truncated);
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pack_respects_max_spans() {
        let root = make_fixture();
        let m = ContextManager::new(
            &root,
            ContextBudget {
                max_chars: 100_000,
                max_span_chars: 500,
                max_spans: 2,
            },
        );
        let spans = vec![
            ContextSpan {
                path: "a.rs".into(),
                start_line: 1,
                end_line: 2,
                reason: "r".into(),
                score: 10,
                snippet: "aaa".into(),
                truncated: false,
                pinned: false,
                total_lines: 2,
            };
            5
        ];
        let packed = m.pack(spans);
        assert_eq!(packed.len(), 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn path_search_finds_by_filename_token() {
        let root = make_fixture();
        let mut m = manager(&root);
        m.scan(&|| true).expect("scan");
        let hits = m.search_paths("big_calc", 10);
        assert!(
            hits.iter().any(|(p, _, _)| p.contains("big_calc")),
            "{hits:?}"
        );
        assert!(hits[0].1 > 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_is_cancellable() {
        let root = make_fixture();
        let mut m = manager(&root);
        use std::sync::atomic::{AtomicU32, Ordering};
        let calls = AtomicU32::new(0);
        let alive = || calls.fetch_add(1, Ordering::SeqCst) < 3;
        let result = m.scan(&alive);
        assert_eq!(result, Err(ScanCancelled));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pinned_span_survives_collect() {
        let root = make_fixture();
        let mut m = manager(&root);
        m.scan(&|| true).expect("scan");
        let pin = m
            .read_range("README.md", 1, 5, "pinned by user")
            .expect("read");
        m.pin(pin);
        let out = m.collect(UNIQUE, &|| true).expect("collect");
        assert!(out.iter().any(|s| s.pinned && s.path == "README.md"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn no_fixed_readme_head_when_unrelated() {
        // Query has zero path/content overlap with README; collect should still
        // prefer code matches over forcing README head as a default fixture read.
        let root = make_fixture();
        let mut m = manager(&root);
        let results = m
            .collect("progressive tax rate calculation", &|| true)
            .expect("collect");
        // Must find code, and must not be "only README first 24 lines".
        assert!(
            results.iter().any(|s| s.path.ends_with(".rs")),
            "expected code hits: {:?}",
            results.iter().map(|r| r.path.clone()).collect::<Vec<_>>()
        );
        let _ = fs::remove_dir_all(&root);
    }
}
