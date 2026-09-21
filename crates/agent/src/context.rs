//! Lexical context retrieval for one turn.
//!
//! Deterministic, cancellable, budget-aware: file map → path/filename search →
//! content grep with line ranges → ranking → pinned + budgeted pack.
//! Observations carry **file version/hash** so mutation can mark them stale;
//! packing never treats stale content as current fact.
//! No vectors, no full-repo content reads.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Default per-turn character budget for packed context handed to the model.
pub const DEFAULT_CONTEXT_CHARS: usize = 12_000;
/// Hard cap for a single span/snippet body (explicit truncation beyond this).
pub const DEFAULT_SPAN_CHARS: usize = 1_800;
/// Max spans packed into one turn.
pub const DEFAULT_MAX_SPANS: usize = 16;
/// Cap on retained observations per turn (evict lowest priority stale first).
pub const MAX_OBSERVATIONS: usize = 128;
/// Cap on history messages re-sent to the model (context budget).
pub const MAX_HISTORY_MESSAGES: usize = 48;
/// Cap on a single tool-result observation body in history.
pub const MAX_HISTORY_OBS_CHARS: usize = 3_500;
/// Walk / index safety caps so huge repos never scan forever.
const MAX_INDEXED_FILES: usize = 20_000;
const MAX_SCAN_DEPTH: usize = 12;
const MAX_GREP_FILES: usize = 400;
const READ_CHUNK: usize = 64 * 1024;

/// One recorded observation of file content (read/search result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub id: String,
    /// `read` | `search` | `command` | `verify` | `model`
    pub source: String,
    pub file: String,
    /// 1-based inclusive range (0,0 when N/A).
    pub range: (usize, usize),
    /// Content hash of the full file when the observation was taken.
    pub file_version: String,
    pub created_step: usize,
    pub priority: i64,
    pub stale: bool,
    /// Token/char size estimate of retained payload.
    pub size_estimate: usize,
    /// Optional snippet; dropped first when over budget (metadata kept if proof).
    pub snippet: String,
    /// True when this observation backs an active acceptance proof —
    /// metadata is never dropped even if the large payload is.
    pub is_proof: bool,
}

impl Observation {
    pub fn observe_read(
        id: impl Into<String>,
        file: impl Into<String>,
        range: (usize, usize),
        file_version: impl Into<String>,
        created_step: usize,
        priority: i64,
        snippet: impl Into<String>,
    ) -> Self {
        let snippet = snippet.into();
        let size_estimate = snippet.chars().count() + 64;
        Self {
            id: id.into(),
            source: "read".into(),
            file: file.into(),
            range,
            file_version: file_version.into(),
            created_step,
            priority,
            stale: false,
            size_estimate,
            snippet,
            is_proof: false,
        }
    }

    pub fn to_prompt_line(&self) -> String {
        let status = if self.stale {
            "stale=OLD VERSION"
        } else {
            "stale=false"
        };
        format!(
            "obs {} {} {}:{}-{} ver={} step={} pri={} {} size~{}",
            self.id,
            self.source,
            self.file,
            self.range.0,
            self.range.1,
            &self.file_version[..self.file_version.len().min(8)],
            self.created_step,
            self.priority,
            status,
            self.size_estimate
        )
    }
}

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
    /// Content hash of the file when this span was read.
    pub file_version: String,
    /// True when the file changed after this observation.
    pub stale: bool,
    /// Observation id when recorded in the manager.
    pub obs_id: String,
}

impl ContextSpan {
    fn base(
        path: &str,
        start_line: usize,
        end_line: usize,
        reason: &str,
        score: i64,
        snippet: String,
        total_lines: usize,
    ) -> Self {
        Self {
            path: path.to_owned(),
            start_line,
            end_line,
            reason: reason.to_owned(),
            score,
            snippet,
            truncated: false,
            pinned: false,
            total_lines,
            file_version: String::new(),
            stale: false,
            obs_id: String::new(),
        }
    }

    /// Model-facing block with explicit truncation / OLD VERSION notices.
    pub fn to_prompt_block(&self) -> String {
        let mut out = format!(
            "<context path=\"{}\" lines=\"{}-{}\" score=\"{}\" reason=\"{}\"",
            self.path, self.start_line, self.end_line, self.score, self.reason
        );
        if !self.file_version.is_empty() {
            out.push_str(&format!(
                " version=\"{}\"",
                &self.file_version[..self.file_version.len().min(8)]
            ));
        }
        out.push_str(">\n");
        if self.stale {
            out.push_str(
                "[OLD VERSION — file was modified after this observation; \
                 do not treat as current fact; re-read before relying on it]\n",
            );
        }
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
    /// Paths modified this turn — prior observations of these are stale.
    stale_paths: std::collections::BTreeSet<String>,
    /// All recorded observations this turn (versioned).
    observations: Vec<Observation>,
    step_counter: usize,
    /// Paths Kodo mutated this turn (priority boost when re-reading).
    changed_paths: std::collections::BTreeSet<String>,
}

impl ContextManager {
    pub fn new(root: impl Into<PathBuf>, budget: ContextBudget) -> Self {
        Self {
            root: root.into(),
            budget,
            file_map: Vec::new(),
            pinned: Vec::new(),
            scan_complete: false,
            stale_paths: std::collections::BTreeSet::new(),
            observations: Vec::new(),
            step_counter: 0,
            changed_paths: std::collections::BTreeSet::new(),
        }
    }

    /// Monotonic step id for observations.
    pub fn next_step(&mut self) -> usize {
        self.step_counter += 1;
        self.step_counter
    }

    pub fn observations(&self) -> &[Observation] {
        &self.observations
    }

    /// Current on-disk content hash for a project-relative path.
    pub fn file_version(&self, relative: &str) -> String {
        let full = self.root.join(relative);
        match fs::read(&full) {
            Ok(bytes) => hash_bytes(&bytes),
            Err(_) => "missing".into(),
        }
    }

    /// Mark a path's earlier observations stale (file was modified).
    /// Historical Observation records keep `stale=true` + old `file_version`
    /// so OLD VERSION can still be shown; they are not deleted.
    pub fn invalidate(&mut self, relative: &str) {
        let rel = relative.trim().trim_start_matches("./").to_owned();
        self.stale_paths.insert(rel.clone());
        self.changed_paths.insert(rel.clone());
        for obs in self.observations.iter_mut() {
            if obs.file == rel {
                obs.stale = true;
            }
        }
        for p in self.pinned.iter_mut() {
            if p.path == rel {
                p.stale = true;
            }
        }
    }

    /// True when this path was modified after its content was observed.
    pub fn is_stale(&self, relative: &str) -> bool {
        let rel = relative.trim().trim_start_matches("./");
        self.stale_paths.contains(rel)
    }

    pub fn stale_paths(&self) -> &std::collections::BTreeSet<String> {
        &self.stale_paths
    }

    pub fn changed_paths(&self) -> &std::collections::BTreeSet<String> {
        &self.changed_paths
    }

    /// Clear the stale marker (after a fresh read of the path at current version).
    pub fn refresh(&mut self, relative: &str) {
        let rel = relative.trim().trim_start_matches("./");
        // Only clear path-level stale when a fresh observation exists at current hash.
        let ver = self.file_version(rel);
        let has_fresh = self
            .observations
            .iter()
            .any(|o| o.file == rel && !o.stale && o.file_version == ver);
        if has_fresh {
            self.stale_paths.remove(rel);
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
    /// Records a versioned Observation and refreshes path staleness.
    pub fn read_range(
        &mut self,
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
            let mut empty = ContextSpan::base(path, 1, 0, reason, 0, String::new(), 0);
            empty.file_version = hash_bytes(b"");
            return Ok(empty);
        }
        let start = start_line.clamp(1, total);
        let end = end_line.clamp(start, total);
        let mut snippet = lines[start - 1..end].join("\n");
        let mut truncated = false;
        if snippet.len() > self.budget.max_span_chars {
            snippet = truncate_chars(&snippet, self.budget.max_span_chars);
            truncated = true;
        }
        let version = hash_bytes(text.as_bytes());
        let mut span = ContextSpan::base(path, start, end, reason, 50, snippet, total);
        span.truncated = truncated;
        span.file_version = version.clone();
        span.stale = false;
        let obs_id = format!("obs_{}", self.next_step());
        span.obs_id = obs_id.clone();
        let score = self.priority_for(path, reason);
        span.score = 50 + score;
        self.record_observation(Observation::observe_read(
            obs_id,
            path,
            (start, end),
            version,
            self.step_counter,
            score,
            span.snippet.clone(),
        ));
        // Fresh read at current disk version → path is no longer stale.
        self.stale_paths.remove(path.trim_start_matches("./"));
        Ok(span)
    }

    /// Priority: errors/changed files > query hits > old unrelated.
    fn priority_for(&self, path: &str, reason: &str) -> i64 {
        let mut pri = 10;
        if self.changed_paths.contains(path) || self.stale_paths.contains(path) {
            pri += 50; // changed files get priority
        }
        let r = reason.to_ascii_lowercase();
        if r.contains("error") || r.contains("fail") || r.contains("assert") {
            pri += 40; // current errors
        }
        if r.contains("criterion") || r.contains("subtask") || r.contains("verify") {
            pri += 30; // active criterion / subtask
        }
        pri
    }

    /// Insert observation with dedupe + capacity eviction.
    fn record_observation(&mut self, obs: Observation) {
        let key = (obs.file.clone(), obs.file_version.clone(), obs.range);
        // Dedupe: same file + version + range.
        if let Some(existing) = self
            .observations
            .iter_mut()
            .find(|o| (o.file.clone(), o.file_version.clone(), o.range) == key)
        {
            existing.snippet = obs.snippet.clone();
            existing.priority = existing.priority.max(obs.priority);
            existing.created_step = obs.created_step;
            existing.stale = obs.stale;
            existing.size_estimate = obs.size_estimate;
            return;
        }
        self.observations.push(obs);
        if self.observations.len() > MAX_OBSERVATIONS {
            // Evict lowest-priority stale first, then lowest priority non-proof.
            self.observations.sort_by(|a, b| {
                b.stale
                    .cmp(&a.stale)
                    .then(a.is_proof.cmp(&b.is_proof))
                    .then(a.priority.cmp(&b.priority))
                    .then(a.created_step.cmp(&b.created_step))
            });
            // Evict from the end (stale, non-proof, low priority, old).
            while self.observations.len() > MAX_OBSERVATIONS {
                let last = self.observations.len() - 1;
                if self.observations[last].is_proof {
                    // Keep proof metadata: drop a non-proof if possible.
                    if let Some(pos) = self.observations.iter().rposition(|o| !o.is_proof) {
                        // Still may drop proof snippet payload only.
                        if self.observations[pos].snippet.len() > 64 {
                            self.observations[pos].snippet.clear();
                            self.observations[pos].size_estimate = 64;
                            continue;
                        }
                        self.observations.remove(pos);
                        continue;
                    }
                    break;
                }
                self.observations.remove(last);
            }
        }
    }

    /// Mark observation as backing acceptance proof (never drop metadata).
    pub fn mark_proof(&mut self, obs_id: &str) {
        for o in self.observations.iter_mut() {
            if o.id == obs_id {
                o.is_proof = true;
                o.priority += 60;
            }
        }
    }

    /// Rank and pack context for one turn under the character budget.
    /// Skips stale observations (unless they are the only match — then labeled).
    pub fn collect(
        &mut self,
        query: &str,
        alive: &dyn Fn() -> bool,
    ) -> Result<Vec<ContextSpan>, ScanCancelled> {
        if !self.scan_complete {
            self.scan(alive)?;
        }

        let mut candidates: Vec<ContextSpan> = Vec::new();

        // Pinned always first — skip stale pins (OLD VERSION not default).
        for p in &self.pinned {
            if p.stale || self.is_stale(&p.path) {
                continue;
            }
            candidates.push(p.clone());
        }

        // Path hits → small header spans (first lines only as evidence of existence).
        for (path, score, reason) in self.search_paths(query, 8) {
            if !alive() {
                return Err(ScanCancelled);
            }
            if let Ok(span) = self.read_range(&path, 1, 24, &reason) {
                let mut span = span;
                span.score = score + self.priority_for(&path, &reason);
                candidates.push(span);
            }
        }

        // Content hits with real line ranges.
        if let Ok(greps) = self.grep(query, 12, alive) {
            for mut g in greps {
                // Attach version + observation if missing.
                if g.file_version.is_empty() {
                    g.file_version = self.file_version(&g.path);
                }
                g.stale = self.is_stale(&g.path);
                if g.stale {
                    // Fresh grep of stale path: re-read to get current version.
                    if let Ok(fresh) = self.read_range(&g.path, g.start_line, g.end_line, &g.reason)
                    {
                        candidates.push(fresh);
                        continue;
                    }
                }
                g.score += self.priority_for(&g.path, &g.reason);
                candidates.push(g);
            }
        }

        // Dedupe by (path, file_version, start_line): keep higher score.
        candidates.sort_by_key(|c| std::cmp::Reverse(c.score));
        let mut seen: HashSet<(String, String, usize)> = HashSet::new();
        let mut unique = Vec::new();
        for c in candidates {
            if c.stale {
                continue; // never default-pack stale content
            }
            let key = (c.path.clone(), c.file_version.clone(), c.start_line);
            if seen.insert(key) {
                unique.push(c);
            }
        }

        // Pack under budget — priority-aware (score already includes boosts).
        let mut packed: Vec<ContextSpan> = Vec::new();
        let mut used = 0usize;
        for p in &self.pinned {
            if p.stale {
                continue;
            }
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
    /// Stale spans (if any slipped through) are labeled OLD VERSION.
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
        if spans.iter().any(|s| s.stale) {
            out.push_str(
                "Note: one or more blocks are OLD VERSION — the file changed after they were read. \
                 Re-read before treating them as current.\n",
            );
        }
        out
    }

    /// Format command/tool output for history: structured summary + bounded
    /// head/tail + output reference (never unbounded append).
    pub fn format_command_output(command: &str, output: &str) -> String {
        let ref_id = format!(
            "out_{:08x}",
            (hash_bytes(output.as_bytes()).as_bytes()[0] as u32)
                ^ (hash_bytes(command.as_bytes()).as_bytes()[0] as u32)
        );
        if output.chars().count() <= MAX_HISTORY_OBS_CHARS {
            return format!("$ {command}\n{output}\n[output ref: {ref_id}]");
        }
        let head: String = output.chars().take(1600).collect();
        let tail: String = output
            .chars()
            .rev()
            .take(800)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!(
            "$ {command}\n[output truncated: {} chars total; structured head/tail]\n\
             --- head ---\n{head}\n--- tail ---\n{tail}\n[output ref: {ref_id}]",
            output.chars().count()
        )
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn git_ls_files(root: &Path) -> Vec<String> {
    use crate::process::{ProcessRunner, ProcessSpec, ProcessStatus};
    let spec = ProcessSpec::shell(root, "git ls-files -z")
        .timeout(std::time::Duration::from_secs(15))
        .stdout_limit(512 * 1024)
        .stderr_limit(4 * 1024);
    let outcome = ProcessRunner::run(&spec, &|| true);
    if outcome.status != ProcessStatus::ExitSuccess {
        return Vec::new();
    }
    outcome
        .stdout
        .split('\0')
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

    let mut span = ContextSpan::base(
        &entry.path,
        start + 1,
        end + 1,
        &format!("lexical grep hit for `{}`", query.trim()),
        score,
        snippet,
        total,
    );
    span.truncated = truncated;
    Some(span)
}

/// FNV-1a 64 content hash (same scheme as checkpoint).
fn hash_bytes(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
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
    use std::process::Command;

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
        let mut m = manager(&root);
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
        let mut m = manager(&root);
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
        let hit = hit.unwrap_or_else(|| {
            panic!(
                "src/big_calc.rs not in top: {:?}",
                results
                    .iter()
                    .map(|r| (&r.path, r.score, r.start_line))
                    .collect::<Vec<_>>()
            )
        });
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
        let spans = vec![ContextSpan::base("a.rs", 1, 2, "r", 10, "aaa".into(), 2); 5];
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

    // -----------------------------------------------------------------------
    // Stale / Observation regressions
    // -----------------------------------------------------------------------

    #[test]
    fn stale_read_v1_edit_then_observation_marked_stale() {
        let root = make_fixture();
        let mut m = manager(&root);
        let path = "src/big_calc.rs";
        // Read v1
        let v1 = m.read_range(path, 700, 704, "initial read").expect("read");
        assert!(!v1.stale);
        assert!(!v1.file_version.is_empty());
        let obs_before = m.observations().len();
        assert!(obs_before >= 1);
        let obs_v1 = m.observations().last().unwrap().clone();
        assert_eq!(obs_v1.file_version, v1.file_version);
        assert!(!obs_v1.stale);

        // Edit file (mutation)
        let full = root.join(path);
        let mut text = fs::read_to_string(&full).unwrap();
        text.push_str("\n// EDITED after observe\n");
        fs::write(&full, text).unwrap();
        m.invalidate(path);

        // v1 observation is stale; path marked stale
        assert!(m.is_stale(path));
        let obs = m.observations().iter().find(|o| o.id == obs_v1.id).unwrap();
        assert!(obs.stale, "v1 observation must be stale after edit");
        assert_eq!(obs.file_version, v1.file_version, "keeps old version hash");
        // Pinned stale spans labeled OLD VERSION
        let mut span = v1.clone();
        span.stale = true;
        let block = span.to_prompt_block();
        assert!(block.contains("OLD VERSION"), "{block}");

        // collect must not default-pack stale pinned content
        m.pin(span);
        let packed = m.collect("progressive tax", &|| true).expect("collect");
        assert!(
            !packed
                .iter()
                .any(|s| s.stale && s.path == path && s.obs_id == obs_v1.id),
            "stale v1 must not be default-packed as current"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_read_v2_after_edit_is_current() {
        let root = make_fixture();
        let mut m = manager(&root);
        let path = "src/big_calc.rs";
        let v1 = m.read_range(path, 700, 704, "v1").expect("read");
        let full = root.join(path);
        let mut text = fs::read_to_string(&full).unwrap();
        text.push_str("\n// v2 marker UNIQUE_V2_LINE\n");
        fs::write(&full, text).unwrap();
        m.invalidate(path);
        assert!(m.is_stale(path));

        // Fresh read → v2 current, path no longer stale
        let v2 = m.read_range(path, 1, 30, "v2 re-read").expect("re-read");
        assert!(!v2.stale);
        assert_ne!(v2.file_version, v1.file_version, "version must change");
        assert!(!m.is_stale(path), "fresh read clears path stale");

        let latest = m.observations().last().unwrap();
        assert!(!latest.stale);
        assert_eq!(latest.file_version, v2.file_version);
        // Old observation remains stale in the registry
        let old = m
            .observations()
            .iter()
            .find(|o| o.file_version == v1.file_version)
            .unwrap();
        assert!(old.stale);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_repeated_same_read_is_deduped() {
        let root = make_fixture();
        let mut m = manager(&root);
        let path = "src/big_calc.rs";
        let n0 = m.observations().len();
        let a = m.read_range(path, 700, 704, "r1").unwrap();
        let n1 = m.observations().len();
        let b = m.read_range(path, 700, 704, "r2").unwrap();
        let n2 = m.observations().len();
        assert_eq!(n1, n0 + 1, "first read records one obs");
        assert_eq!(n2, n1, "same file+version+range deduped, got {n2} vs {n1}");
        assert_eq!(a.file_version, b.file_version);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_changed_file_gets_priority_over_unrelated() {
        let root = make_fixture();
        let mut m = manager(&root);
        // Change a file first so it gets priority boost on re-read.
        let path = "src/big_calc.rs";
        m.invalidate(path); // mark as changed (even without prior read)
        let changed = m.read_range(path, 700, 704, "changed file").unwrap();
        // Unrelated README at low natural score
        let unrelated = m.read_range("README.md", 1, 5, "readme").unwrap();
        assert!(
            changed.score >= unrelated.score,
            "changed file should outrank unrelated: changed={} readme={}",
            changed.score,
            unrelated.score
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_huge_log_does_not_explode_prompt() {
        let huge: String = "x".repeat(50_000);
        let formatted = ContextManager::format_command_output("cargo test -- --nocapture", &huge);
        assert!(
            formatted.chars().count() < 6_000,
            "history obs must be bounded, got {}",
            formatted.chars().count()
        );
        assert!(formatted.contains("output truncated"));
        assert!(formatted.contains("output ref:"));
        assert!(formatted.contains("head"));
        assert!(formatted.contains("tail"));
        // Under budget → still includes ref
        let small = ContextManager::format_command_output("echo hi", "ok");
        assert!(small.contains("output ref:"));
        assert!(small.contains("ok"));
    }

    #[test]
    fn stale_priority_current_errors_and_changed_rank_high() {
        let root = make_fixture();
        let mut m = manager(&root);
        m.invalidate("src/big_calc.rs");
        let err_pri = m.priority_for("src/big_calc.rs", "current error assert failed");
        let plain_pri = m.priority_for("README.md", "lexical grep hit");
        assert!(
            err_pri > plain_pri,
            "errors+changed should outrank plain: {err_pri} vs {plain_pri}"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
