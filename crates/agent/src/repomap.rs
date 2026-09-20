//! Lightweight repo map (L1) — files, packages, modules, symbols, imports.
//!
//! Lexical ContextManager remains L0. No vector DB and no online service.
//! Rust / TypeScript / TSX get a lightweight **syntax-aware** index (line
//! prefixes + import graph), not a full parser dependency.
//!
//! [`RepoMap::find_references`] reports **confidence**: high-confidence
//! syntactic hits vs lexical fallback — never pretends precision it does not
//! have.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoFileEntry {
    pub path: String,
    pub language: String,
    pub lines: usize,
    pub bytes: u64,
}

/// One indexed definition: symbol, kind, file, line, optional end line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolEntry {
    pub name: String,
    /// `fn` | `struct` | `enum` | `trait` | `type` | `class` | `interface`
    /// | `const` | `method` | `export` …
    pub kind: String,
    pub path: String,
    pub line: usize,
    /// Inclusive end line when known (same-line defs use `line`).
    pub end_line: usize,
    /// True when the definition is `pub` / `export`.
    pub exported: bool,
}

/// Reference hit with explicit confidence (never a silent guess).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceEntry {
    pub path: String,
    pub line: usize,
    /// `high` = word-boundary syntactic hit in source; `lexical` = fallback.
    pub confidence: String,
    /// Short snippet (trimmed).
    pub snippet: String,
    /// True when this line is the definition itself (not a call site).
    pub is_definition: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleEntry {
    pub path: String,
    pub language: String,
    /// Nested module path e.g. `src::auth`
    pub dotted: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportEdge {
    pub from: String,
    /// Raw import path (`use crate::foo`, `import { x } from './y'`)
    pub specifier: String,
    pub line: usize,
}

/// Snapshot of RepoMap used for incremental rebuild of a single file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct FileIndex {
    file: Option<RepoFileEntry>,
    symbols: Vec<SymbolEntry>,
    imports: Vec<ImportEdge>,
    is_test: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoMap {
    pub root: String,
    pub files: Vec<RepoFileEntry>,
    /// packages/crates (`crate:name`, `npm:name`, manifest paths)
    pub packages: Vec<String>,
    pub modules: Vec<ModuleEntry>,
    pub symbols: Vec<SymbolEntry>,
    pub imports: Vec<ImportEdge>,
    pub tests: Vec<String>,
    /// Baseline dirty paths (pre-turn) + Kodo-touched when known.
    pub git_dirty: Vec<String>,
    pub languages: BTreeMap<String, usize>,
}

impl RepoMap {
    pub fn build(root: &Path, alive: &dyn Fn() -> bool) -> Result<Self, String> {
        let mut map = Self {
            root: root.display().to_string(),
            ..Default::default()
        };
        collect_manifests(root, &mut map);
        walk_files(root, root, &mut map, alive)?;
        map.git_dirty = crate::checkpoint::TurnChangeSet::capture_baseline(root)
            .baseline_dirty
            .into_iter()
            .collect();
        map.languages = map.files.iter().fold(BTreeMap::new(), |mut acc, f| {
            *acc.entry(f.language.clone()).or_insert(0) += 1;
            acc
        });
        map.modules = map
            .files
            .iter()
            .filter(|f| is_source_lang(&f.language))
            .map(|f| ModuleEntry {
                path: f.path.clone(),
                language: f.language.clone(),
                dotted: module_dotted(&f.path),
            })
            .collect();
        Ok(map)
    }

    /// Definitions matching `query` (exact first, then substring), ranked.
    /// Returns symbol / kind / file / line+range via [`SymbolEntry`].
    pub fn find_symbol(&self, query: &str) -> Vec<&SymbolEntry> {
        let q = query.trim();
        if q.is_empty() {
            return Vec::new();
        }
        let mut exact: Vec<&SymbolEntry> = self.symbols.iter().filter(|s| s.name == q).collect();
        if !exact.is_empty() {
            exact.truncate(40);
            return exact;
        }
        let mut sub: Vec<&SymbolEntry> = self
            .symbols
            .iter()
            .filter(|s| s.name.contains(q) || s.name.eq_ignore_ascii_case(q))
            .collect();
        sub.truncate(40);
        sub
    }

    /// References for `name` with confidence:
    /// * `high` — word-boundary match in a source file line (definition lines
    ///   flagged `is_definition`).
    /// * `lexical` — no high-confidence hits, or non-source files only.
    ///
    /// Never invents precision: empty high-confidence ⇒ lexical fallback label.
    pub fn find_references(&self, name: &str) -> Vec<ReferenceEntry> {
        let q = name.trim();
        if q.is_empty() {
            return Vec::new();
        }
        let def_lines: BTreeSet<(String, usize)> = self
            .symbols
            .iter()
            .filter(|s| s.name == q)
            .map(|s| (s.path.clone(), s.line))
            .collect();

        let mut high: Vec<ReferenceEntry> = Vec::new();
        let mut lexical: Vec<ReferenceEntry> = Vec::new();

        for file in &self.files {
            if file.bytes > 256 * 1024 {
                continue;
            }
            let Some(lang) = source_language(&file.language) else {
                continue;
            };
            let Ok(text) = fs::read_to_string(Path::new(&self.root).join(&file.path)) else {
                continue;
            };
            for (idx, line) in text.lines().enumerate() {
                let lineno = idx + 1;
                let is_def = def_lines.contains(&(file.path.clone(), lineno));
                if word_boundary_match(line, q) {
                    high.push(ReferenceEntry {
                        path: file.path.clone(),
                        line: lineno,
                        confidence: "high".into(),
                        snippet: clip(line.trim(), 160),
                        is_definition: is_def,
                    });
                } else if lang && line.contains(q) {
                    // Inside source but not a word boundary (e.g. `foo_bar`).
                    lexical.push(ReferenceEntry {
                        path: file.path.clone(),
                        line: lineno,
                        confidence: "lexical".into(),
                        snippet: clip(line.trim(), 160),
                        is_definition: is_def,
                    });
                }
            }
        }

        if high.is_empty() && !lexical.is_empty() {
            // Honest: only weak hits exist — label all as lexical fallback.
            for r in lexical.iter_mut() {
                r.confidence = "lexical".into();
            }
            lexical.truncate(80);
            return lexical;
        }
        high.truncate(80);
        high.extend(lexical.into_iter().take(20));
        high
    }

    pub fn list_files(&self, prefix: Option<&str>, limit: usize) -> Vec<&RepoFileEntry> {
        self.files
            .iter()
            .filter(|f| prefix.map(|p| f.path.starts_with(p)).unwrap_or(true))
            .take(limit)
            .collect()
    }

    /// Imports/uses from files that mention `name` in a specifier or nearby.
    pub fn imports_for(&self, name: &str) -> Vec<&ImportEdge> {
        self.imports
            .iter()
            .filter(|e| e.specifier.contains(name))
            .take(40)
            .collect()
    }

    pub fn to_prompt_block(&self) -> String {
        let mut out = String::from("## Repo map\n");
        out.push_str(&format!("- files: {}\n", self.files.len()));
        out.push_str(&format!("- languages: {:?}\n", self.languages));
        out.push_str(&format!("- packages: {}\n", self.packages.join(", ")));
        out.push_str(&format!("- modules: {}\n", self.modules.len()));
        out.push_str(&format!("- symbols: {}\n", self.symbols.len()));
        out.push_str(&format!("- imports: {}\n", self.imports.len()));
        out.push_str(&format!("- tests: {}\n", self.tests.len()));
        out.push_str(&format!("- git_dirty: {}\n", self.git_dirty.join(", ")));
        for file in self.files.iter().take(24) {
            out.push_str(&format!(
                "  - {} ({}{} )\n",
                file.path,
                file.language,
                if file.bytes > 0 {
                    format!(", {} lines", file.lines)
                } else {
                    String::new()
                }
            ));
        }
        // Exported symbols sample for orientation (not a full dump).
        let exported: Vec<&SymbolEntry> = self
            .symbols
            .iter()
            .filter(|s| s.exported)
            .take(20)
            .collect();
        if !exported.is_empty() {
            out.push_str("- exported/public symbols:\n");
            for s in exported {
                out.push_str(&format!(
                    "  - {} {} at {}:{}-{}\n",
                    s.kind, s.name, s.path, s.line, s.end_line
                ));
            }
        }
        out.push_str(
            "Search policy: RepoMap → find_symbol/find_references → read_range on hit files \
             (avoid reading whole unrelated files).\n",
        );
        out
    }
}

// ---------------------------------------------------------------------------
// Incremental cache (mtime + size): rebuild only changed files after edits.
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct CachedFile {
    mtime: Option<SystemTime>,
    bytes: u64,
    index: FileIndex,
}

/// Process-wide incremental RepoMap cache. Keyed by canonical root.
#[derive(Default)]
pub struct RepoMapCache {
    inner: Mutex<BTreeMap<String, CacheEntry>>,
}

#[derive(Default)]
struct CacheEntry {
    files: BTreeMap<String, CachedFile>,
    assembled: Option<Arc<RepoMap>>,
}

impl RepoMapCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build or refresh the map, re-indexing only files whose mtime/size changed.
    pub fn get_or_build(
        &self,
        root: &Path,
        alive: &dyn Fn() -> bool,
    ) -> Result<Arc<RepoMap>, String> {
        let key = root.display().to_string();
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let entry = guard.entry(key).or_default();
        // Fast path: assembled map is fresh if no file metadata changed.
        // Full rebuild is still correct and cheap for small repos; we still
        // refresh when any source file's mtime/size differs.
        let discovered = list_source_paths(root, alive)?;
        let mut dirty = entry.assembled.is_none();
        for rel in &discovered {
            let full = root.join(rel);
            let meta = fs::metadata(&full).ok();
            let mtime = meta.as_ref().and_then(|m| m.modified().ok());
            let bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            match entry.files.get(rel) {
                Some(c) if c.mtime == mtime && c.bytes == bytes => {}
                _ => {
                    dirty = true;
                    entry.files.insert(
                        rel.clone(),
                        CachedFile {
                            mtime,
                            bytes,
                            index: FileIndex::default(),
                        },
                    );
                }
            }
        }
        // Drop cache entries for deleted files.
        let set: BTreeSet<&String> = discovered.iter().collect();
        let stale: Vec<String> = entry
            .files
            .keys()
            .filter(|k| !set.contains(k))
            .cloned()
            .collect();
        if !stale.is_empty() {
            dirty = true;
            for k in stale {
                entry.files.remove(&k);
            }
        }

        if dirty || entry.assembled.is_none() {
            // Re-index only files whose CachedFile.index is empty (new/stale).
            for (rel, cached) in entry.files.iter_mut() {
                let needs = cached.index.file.is_none();
                if needs {
                    cached.index = index_file(root, rel, alive)?;
                }
            }
            let map = assemble_from_cache(root, &entry.files, alive)?;
            entry.assembled = Some(Arc::new(map));
        }
        Ok(entry.assembled.clone().expect("just built"))
    }
}

fn list_source_paths(root: &Path, alive: &dyn Fn() -> bool) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    collect_paths(root, root, &mut out, alive)?;
    out.sort();
    Ok(out)
}

fn assemble_from_cache(
    root: &Path,
    files: &BTreeMap<String, CachedFile>,
    alive: &dyn Fn() -> bool,
) -> Result<RepoMap, String> {
    let _ = alive;
    let mut map = RepoMap {
        root: root.display().to_string(),
        ..Default::default()
    };
    collect_manifests(root, &mut map);
    for (rel, cached) in files {
        if let Some(f) = &cached.index.file {
            map.files.push(f.clone());
        }
        map.symbols.extend(cached.index.symbols.iter().cloned());
        map.imports.extend(cached.index.imports.iter().cloned());
        if cached.index.is_test {
            map.tests.push(rel.clone());
        }
    }
    map.files.sort_by(|a, b| a.path.cmp(&b.path));
    map.symbols
        .sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    map.imports
        .sort_by(|a, b| a.from.cmp(&b.from).then(a.line.cmp(&b.line)));
    map.git_dirty = crate::checkpoint::TurnChangeSet::capture_baseline(root)
        .baseline_dirty
        .into_iter()
        .collect();
    map.languages = map.files.iter().fold(BTreeMap::new(), |mut acc, f| {
        *acc.entry(f.language.clone()).or_insert(0) += 1;
        acc
    });
    map.modules = map
        .files
        .iter()
        .filter(|f| is_source_lang(&f.language))
        .map(|f| ModuleEntry {
            path: f.path.clone(),
            language: f.language.clone(),
            dotted: module_dotted(&f.path),
        })
        .collect();
    Ok(map)
}

// ---------------------------------------------------------------------------
// Walking / indexing
// ---------------------------------------------------------------------------

fn excluded_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules" | "target" | "dist" | "build" | "out" | ".next" | "coverage" | "__pycache__"
    ) || name.starts_with('.')
}

fn is_source_lang(language: &str) -> bool {
    matches!(language, "rust" | "typescript" | "javascript")
}

fn source_language(language: &str) -> Option<bool> {
    match language {
        "rust" | "typescript" | "javascript" => Some(true),
        _ if matches!(language, "markdown" | "json" | "toml" | "yaml") => None,
        _ => Some(false),
    }
}

fn collect_manifests(root: &Path, map: &mut RepoMap) {
    if root.join("Cargo.toml").is_file() {
        map.packages.push("rust:cargo".into());
        if let Ok(text) = fs::read_to_string(root.join("Cargo.toml")) {
            for line in text.lines() {
                let line = line.trim();
                if let Some(name) = line.strip_prefix("name = ") {
                    map.packages
                        .push(format!("crate:{}", name.trim_matches('"')));
                }
            }
        }
    }
    if root.join("package.json").is_file() {
        map.packages.push("node:npm".into());
        if let Ok(text) = fs::read_to_string(root.join("package.json")) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(name) = value.get("name").and_then(|v| v.as_str()) {
                    map.packages.push(format!("npm:{name}"));
                }
            }
        }
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if let Ok(sub) = fs::read_dir(&path) {
                for e in sub.flatten() {
                    let p = e.path();
                    if p.file_name().map(|n| n == "Cargo.toml").unwrap_or(false) {
                        if let Ok(text) = fs::read_to_string(&p) {
                            if let Some(name) =
                                text.lines().find_map(|l| l.trim().strip_prefix("name = "))
                            {
                                map.packages
                                    .push(format!("crate:{}", name.trim_matches('"')));
                            }
                        }
                    }
                    if p.file_name().map(|n| n == "package.json").unwrap_or(false) {
                        if let Ok(rel) = p.strip_prefix(root) {
                            map.packages.push(format!("pkg:{}", rel.display()));
                        }
                    }
                }
            }
        }
    }
    map.packages.sort();
    map.packages.dedup();
}

fn walk_files(
    root: &Path,
    dir: &Path,
    map: &mut RepoMap,
    alive: &dyn Fn() -> bool,
) -> Result<(), String> {
    if !alive() {
        return Err("cancelled".into());
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        if !alive() {
            return Err("cancelled".into());
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if excluded_dir(&name) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk_files(root, &path, map, alive)?;
            continue;
        }
        // Skip binaries / huge files.
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if meta.len() > 512 * 1024 {
            continue;
        }
        if is_binary_name(&name) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| name.clone());
        let idx = index_file(root, &rel, alive)?;
        if let Some(f) = idx.file {
            map.files.push(f);
        }
        map.symbols.extend(idx.symbols);
        map.imports.extend(idx.imports);
        if idx.is_test {
            map.tests.push(rel);
        }
    }
    map.files.sort_by(|a, b| a.path.cmp(&b.path));
    map.symbols
        .sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    Ok(())
}

fn is_binary_name(name: &str) -> bool {
    const EXT: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "webp", "ico", "pdf", "zip", "gz", "tar", "woff", "woff2",
        "ttf", "eot", "so", "dylib", "dll", "exe", "bin", "o", "a", "wasm", "class", "jar", "lock",
    ];
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| EXT.contains(&e))
        .unwrap_or(false)
}

fn module_dotted(path: &str) -> String {
    path.trim_end_matches(".rs")
        .trim_end_matches(".ts")
        .trim_end_matches(".tsx")
        .trim_end_matches(".js")
        .trim_end_matches(".jsx")
        .replace('/', "::")
}

fn collect_paths(
    root: &Path,
    dir: &Path,
    out: &mut Vec<String>,
    alive: &dyn Fn() -> bool,
) -> Result<(), String> {
    if !alive() {
        return Err("cancelled".into());
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if excluded_dir(&name) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_paths(root, &path, out, alive)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            let rel_s = rel.display().to_string();
            if is_source_lang(&language_of(&path)) || language_of(&path) == "markdown" {
                out.push(rel_s);
            }
        }
    }
    Ok(())
}

fn language_of(path: &Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
        .as_str()
    {
        "rs" => "rust".into(),
        "ts" | "tsx" => "typescript".into(),
        "js" | "jsx" => "javascript".into(),
        "py" => "python".into(),
        "json" => "json".into(),
        "toml" => "toml".into(),
        "md" => "markdown".into(),
        "css" => "css".into(),
        other => other.to_owned(),
    }
}

/// Index one file: language metadata + symbols + imports (syntax-aware line scan).
fn index_file(root: &Path, rel: &str, alive: &dyn Fn() -> bool) -> Result<FileIndex, String> {
    if !alive() {
        return Err("cancelled".into());
    }
    let path = root.join(rel);
    let language = language_of(&path);
    let Ok(meta) = fs::metadata(&path) else {
        return Ok(FileIndex::default());
    };
    let is_source = is_source_lang(&language);
    let want_text = is_source || language == "markdown" || language == "toml" || language == "json";
    if !want_text || meta.len() > 512 * 1024 {
        return Ok(FileIndex::default());
    }
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(FileIndex::default());
    };
    let lines = text.lines().count();
    let file = Some(RepoFileEntry {
        path: rel.to_owned(),
        language: language.clone(),
        lines,
        bytes: meta.len(),
    });
    let is_test = {
        let lower = rel.to_ascii_lowercase();
        lower.contains("test")
            || lower.contains("__tests__")
            || lower.ends_with(".test.ts")
            || lower.ends_with(".spec.ts")
            || lower.ends_with(".rs") && (lower.contains("tests/") || lower.ends_with("_test.rs"))
    };
    let mut symbols = Vec::new();
    let mut imports = Vec::new();
    if is_source {
        extract_rust_ts_symbols(rel, &language, &text, &mut symbols);
        extract_imports(rel, &language, &text, &mut imports);
    }
    Ok(FileIndex {
        file,
        symbols,
        imports,
        is_test,
    })
}

// ---------------------------------------------------------------------------
// Syntax-aware symbol extraction (Rust / TS / TSX) — line-prefix scanner.
// ---------------------------------------------------------------------------

fn extract_rust_ts_symbols(path: &str, language: &str, text: &str, out: &mut Vec<SymbolEntry>) {
    for (idx, raw) in text.lines().enumerate() {
        // Skip block comments / strings roughly for definition lines.
        let trimmed = raw.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") {
            continue;
        }
        let line = idx + 1;
        if language == "rust" {
            extract_rust_line(path, trimmed, line, out);
        } else {
            extract_ts_line(path, trimmed, line, out);
        }
    }
    // Close open blocks: end_line = next def or EOF (approximate).
    finalize_ranges(path, out, text.lines().count());
}

fn extract_rust_line(path: &str, trimmed: &str, line: usize, out: &mut Vec<SymbolEntry>) {
    let exported = trimmed.starts_with("pub ");
    let body = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
    for (prefix, kind) in [
        ("fn ", "fn"),
        ("async fn ", "fn"),
        ("struct ", "struct"),
        ("enum ", "enum"),
        ("trait ", "trait"),
        ("type ", "type"),
        ("mod ", "mod"),
        ("const ", "const"),
        ("static ", "static"),
        ("macro_rules! ", "macro"),
    ] {
        if let Some(rest) = body.strip_prefix(prefix) {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                .collect();
            if !name.is_empty()
                && name.chars().next().map(|c| c.is_alphabetic() || c == '_') != Some(false)
            {
                out.push(SymbolEntry {
                    name,
                    kind: kind.into(),
                    path: path.into(),
                    line,
                    end_line: line,
                    exported,
                });
                return;
            }
        }
    }
    // impl blocks: track associated fn later via "fn " inside — already covered.
}

fn extract_ts_line(path: &str, trimmed: &str, line: usize, out: &mut Vec<SymbolEntry>) {
    let mut t = trimmed;
    let mut exported = false;
    if let Some(rest) = t.strip_prefix("export ") {
        exported = true;
        t = rest.trim_start();
    }
    if let Some(rest) = t.strip_prefix("export default ") {
        exported = true;
        t = rest.trim_start();
    }
    // export { foo } — not a definition.
    if t.starts_with('{') {
        return;
    }
    for (prefix, kind) in [
        ("async function ", "fn"),
        ("function ", "fn"),
        ("class ", "class"),
        ("interface ", "interface"),
        ("type ", "type"),
        ("enum ", "enum"),
        ("const ", "const"),
        ("let ", "let"),
        ("var ", "var"),
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                .collect();
            // Avoid `const x = require(...)` noise when it's just a call — still a def.
            if !name.is_empty()
                && rest[name.len()..]
                    .trim_start()
                    .starts_with(['(', '=', '<', ':', ' '])
            {
                out.push(SymbolEntry {
                    name,
                    kind: kind.into(),
                    path: path.into(),
                    line,
                    end_line: line,
                    exported,
                });
                return;
            }
        }
    }
    // class methods: `  methodName(` inside class — only if previous line had class (approximate: 2-space indent + ident().
    if trimmed.len() > 2 && !trimmed.starts_with("//") {
        let indent_ok = raw_indent_is_method(trimmed);
        if indent_ok {
            if let Some(name) = method_name(trimmed) {
                out.push(SymbolEntry {
                    name,
                    kind: "method".into(),
                    path: path.into(),
                    line,
                    end_line: line,
                    exported: false,
                });
            }
        }
    }
}

fn raw_indent_is_method(trimmed: &str) -> bool {
    // Heuristic: `name(...) {` or `name(...): type {` at method-looking indent.
    trimmed.contains('(')
        && !trimmed.starts_with("if ")
        && !trimmed.starts_with("for ")
        && !trimmed.starts_with("while ")
        && !trimmed.starts_with("switch ")
        && !trimmed.starts_with("return ")
        && !trimmed.starts_with("catch ")
        && !trimmed.starts_with("function ")
}

fn method_name(trimmed: &str) -> Option<String> {
    let before = trimmed.split('(').next()?;
    let name = before.split_whitespace().last()?;
    if name.is_empty() || !name.chars().next()?.is_alphabetic() && name.chars().next()? != '_' {
        return None;
    }
    if name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    {
        Some(name.to_owned())
    } else {
        None
    }
}

fn finalize_ranges(path: &str, symbols: &mut [SymbolEntry], total_lines: usize) {
    // For each symbol in file order, end_line = next symbol line - 1 (or EOF).
    let mut order: Vec<usize> = (0..symbols.len()).collect();
    order.sort_by_key(|&i| symbols[i].line);
    for (pos, &i) in order.iter().enumerate() {
        let next_line = order
            .get(pos + 1)
            .map(|&j| symbols[j].line)
            .unwrap_or(total_lines + 1);
        if symbols[i].end_line < symbols[i].line {
            symbols[i].end_line = symbols[i].line;
        }
        if next_line > symbols[i].line && symbols[i].end_line == symbols[i].line {
            // Leave end_line = line for now (accurate for single-line defs).
            // Callers use line as definition; range is best-effort.
            let _ = path;
        }
        if next_line > symbols[i].line {
            symbols[i].end_line = next_line.saturating_sub(1).max(symbols[i].line);
        }
    }
}

// ---------------------------------------------------------------------------
// Imports / use
// ---------------------------------------------------------------------------

fn extract_imports(path: &str, language: &str, text: &str, out: &mut Vec<ImportEdge>) {
    for (idx, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        let line = idx + 1;
        if language == "rust" {
            if let Some(rest) = trimmed.strip_prefix("use ") {
                let spec = rest.split(';').next().unwrap_or(rest).trim().to_owned();
                if !spec.is_empty() {
                    out.push(ImportEdge {
                        from: path.into(),
                        specifier: spec,
                        line,
                    });
                }
            } else if let Some(rest) = trimmed.strip_prefix("mod ") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    out.push(ImportEdge {
                        from: path.into(),
                        specifier: format!("mod {name}"),
                        line,
                    });
                }
            }
        } else {
            if trimmed.starts_with("import ") {
                // import x from 'y' | import { a } from 'y'
                if let Some(idx_f) = trimmed.rfind(" from ") {
                    let spec = trimmed[idx_f + 6..]
                        .trim()
                        .trim_matches(|c| c == '\'' || c == '"')
                        .to_owned();
                    if !spec.is_empty() {
                        out.push(ImportEdge {
                            from: path.into(),
                            specifier: spec,
                            line,
                        });
                    }
                }
            } else if let Some(rest) = trimmed.strip_prefix("export ") {
                if rest.starts_with('{') || rest.starts_with("*") {
                    if let Some(idx_f) = rest.find(" from ") {
                        let spec = rest[idx_f + 6..]
                            .trim()
                            .trim_matches(|c| c == '\'' || c == '"')
                            .to_owned();
                        if !spec.is_empty() {
                            out.push(ImportEdge {
                                from: path.into(),
                                specifier: format!("export from {spec}"),
                                line,
                            });
                        }
                    }
                }
            } else if let Some(rest) = trimmed.strip_prefix("export * from ") {
                let spec = rest
                    .trim()
                    .trim_matches(|c| c == '\'' || c == '"')
                    .to_owned();
                out.push(ImportEdge {
                    from: path.into(),
                    specifier: spec,
                    line,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Word-boundary match (no regex dependency)
// ---------------------------------------------------------------------------

/// True when `name` appears as a whole identifier token in `line`.
fn word_boundary_match(line: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let bytes = line.as_bytes();
    let needle = name.as_bytes();
    let mut start = 0;
    while start + needle.len() <= bytes.len() {
        if &bytes[start..start + needle.len()] == needle {
            let before_ok = start == 0 || !is_ident_byte(bytes[start - 1]);
            let after = start + needle.len();
            let after_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
            if before_ok && after_ok {
                return true;
            }
            start += 1;
            continue;
        }
        start += 1;
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut o: String = s.chars().take(max.saturating_sub(1)).collect();
        o.push('…');
        o
    }
}

// ---------------------------------------------------------------------------
// Process-global cache used by the agent loop (built once per run, refreshed
// on file change via mtime).
// ---------------------------------------------------------------------------

static GLOBAL_CACHE: once_cell_free::Lazy<RepoMapCache> =
    once_cell_free::Lazy::new(RepoMapCache::new);

/// Tiny lazy static without extra crate.
mod once_cell_free {
    use super::RepoMapCache;
    use std::sync::OnceLock;
    pub struct Lazy<T>(OnceLock<T>, fn() -> T);
    impl<T> Lazy<T> {
        pub const fn new(f: fn() -> T) -> Self {
            Self(OnceLock::new(), f)
        }
    }
    impl std::ops::Deref for Lazy<RepoMapCache> {
        type Target = RepoMapCache;
        fn deref(&self) -> &RepoMapCache {
            self.0.get_or_init(self.1)
        }
    }
}

/// Agent-facing helper: cached RepoMap with incremental file invalidation.
pub fn repo_map_cached(root: &Path, alive: &dyn Fn() -> bool) -> Result<Arc<RepoMap>, String> {
    GLOBAL_CACHE.get_or_build(root, alive)
}

/// Force full rebuild (tests / after bulk mutation).
pub fn repo_map_invalidate(root: &Path) {
    // Drop by inserting empty — get_or_build will rebuild all indices.
    if let Ok(mut guard) = GLOBAL_CACHE.inner.lock() {
        guard.remove(&root.display().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_project(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("kodo_repo_{name}_{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn builds_map_for_rust_and_ts() {
        let dir = temp_project("basic");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("apps/web")).unwrap();
        fs::write(dir.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        fs::write(
            dir.join("src/lib.rs"),
            "pub fn add(a: i32, b: i32) -> i32 { a + b }\nstruct Point;\n",
        )
        .unwrap();
        fs::write(
            dir.join("apps/web/app.ts"),
            "export function hello() { return 1 }\n",
        )
        .unwrap();
        fs::write(dir.join("apps/web/app.test.ts"), "test('x', () => {})\n").unwrap();

        let map = RepoMap::build(&dir, &|| true).unwrap();
        assert!(map
            .files
            .iter()
            .any(|f| f.path == "src/lib.rs" && f.language == "rust"));
        assert!(map.packages.iter().any(|p| p.contains("demo")));
        assert!(map.find_symbol("add").iter().any(|s| s.name == "add"));
        assert!(map
            .find_symbol("hello")
            .iter()
            .any(|s| s.path.contains("app.ts")));
        assert!(map.tests.iter().any(|t| t.contains("app.test.ts")));
        assert!(!map.modules.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repo_symbol_index_returns_kind_file_line_range() {
        let dir = temp_project("sym");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("src/auth.rs"),
            "use crate::config;\npub fn login() {}\npub struct Session {}\n",
        )
        .unwrap();
        fs::write(
            dir.join("src/main.rs"),
            "mod auth;\nuse crate::auth::login;\nfn main() { login(); }\n",
        )
        .unwrap();

        let map = RepoMap::build(&dir, &|| true).unwrap();
        let hits = map.find_symbol("login");
        assert!(!hits.is_empty());
        let def = hits[0];
        assert_eq!(def.name, "login");
        assert_eq!(def.kind, "fn");
        assert_eq!(def.path, "src/auth.rs");
        assert!(def.line >= 1 && def.end_line >= def.line);
        assert!(def.exported);

        let st = map.find_symbol("Session");
        assert_eq!(st[0].kind, "struct");
        assert!(st[0].exported);

        assert!(map
            .imports
            .iter()
            .any(|i| i.specifier.contains("auth::login")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repo_references_distinguish_high_confidence_vs_lexical() {
        let dir = temp_project("refs");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("src/math.rs"),
            "pub fn total(nums: &[i32]) -> i32 { 0 }\npub fn uses_total() { let _ = total(&[]); }\n",
        )
        .unwrap();
        fs::write(
            dir.join("src/app.rs"),
            "use crate::math::total;\nfn run() { total(&[1]); }\n// subtotal helper\n",
        )
        .unwrap();

        let map = RepoMap::build(&dir, &|| true).unwrap();
        let refs = map.find_references("total");
        assert!(!refs.is_empty(), "expected hits");
        let high: Vec<_> = refs.iter().filter(|r| r.confidence == "high").collect();
        assert!(
            !high.is_empty(),
            "word-boundary hits should be high: {refs:?}"
        );
        // Definition flagged.
        assert!(
            high.iter().any(|r| r.is_definition),
            "definition line should be marked: {high:?}"
        );
        // Call site in another file (cross-file).
        assert!(
            high.iter()
                .any(|r| r.path == "src/app.rs" && !r.is_definition),
            "cross-file call site: {high:?}"
        );
        // `subtotal` must not be a high-confidence hit for `total`.
        assert!(
            !high
                .iter()
                .any(|r| r.snippet.contains("subtotal") && !r.snippet.contains("total(")),
            "word boundary failed: {high:?}"
        );

        // Pure lexical: query that only appears as substring.
        let weak = map.find_references("ota");
        if !weak.is_empty() {
            assert!(weak.iter().all(|r| r.confidence == "lexical"));
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repo_excludes_generated_and_node_modules() {
        let dir = temp_project("excl");
        fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(dir.join("target/debug")).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/lib.rs"), "pub fn keep() {}\n").unwrap();
        fs::write(
            dir.join("node_modules/pkg/index.js"),
            "export function skip() {}\n",
        )
        .unwrap();
        fs::write(dir.join("target/debug/x.rs"), "pub fn skip2() {}\n").unwrap();
        // binary-ish
        fs::write(dir.join("src/logo.png"), [0u8, 1, 2, 3]).unwrap();

        let map = RepoMap::build(&dir, &|| true).unwrap();
        assert!(map.find_symbol("keep").iter().any(|s| s.name == "keep"));
        assert!(map.find_symbol("skip").is_empty());
        assert!(map.find_symbol("skip2").is_empty());
        assert!(!map.files.iter().any(|f| f.path.contains("node_modules")));
        assert!(!map.files.iter().any(|f| f.path.contains("logo.png")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repo_incremental_cache_invalidates_only_changed_file() {
        let dir = temp_project("incr");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/a.rs"), "pub fn alpha() {}\n").unwrap();
        fs::write(dir.join("src/b.rs"), "pub fn beta() {}\n").unwrap();

        let cache = RepoMapCache::new();
        let m1 = cache.get_or_build(&dir, &|| true).unwrap();
        assert!(m1.find_symbol("alpha").iter().any(|s| s.name == "alpha"));

        // Change a.rs only — sleep to ensure mtime tick.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(
            dir.join("src/a.rs"),
            "pub fn alpha() {}\npub fn gamma() {}\n",
        )
        .unwrap();
        let m2 = cache.get_or_build(&dir, &|| true).unwrap();
        assert!(
            m2.find_symbol("gamma").iter().any(|s| s.name == "gamma"),
            "stale index: gamma missing after edit"
        );
        assert!(m2.find_symbol("beta").iter().any(|s| s.name == "beta"));

        // Untouched file symbols remain.
        assert!(m2.files.iter().any(|f| f.path == "src/b.rs"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_range_tool_reads_slice() {
        let dir = temp_project("range");
        fs::write(dir.join("f.rs"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
        let body = crate::tools::read_text_range(&dir.join("f.rs"), 2, 4, 1000).unwrap();
        assert!(body.contains("l2"));
        assert!(body.contains("l4"));
        assert!(!body.contains("l5"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tsx_and_exports_indexed() {
        let dir = temp_project("tsx");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("src/Widget.tsx"),
            "export function Widget() {\n  return null;\n}\nexport const theme = 'dark';\n",
        )
        .unwrap();
        fs::write(
            dir.join("src/useThing.ts"),
            "import { Widget } from './Widget';\nexport function useThing() {}\n",
        )
        .unwrap();
        let map = RepoMap::build(&dir, &|| true).unwrap();
        let w = map.find_symbol("Widget");
        assert!(w.iter().any(|s| s.name == "Widget" && s.exported));
        assert!(map.imports.iter().any(|i| i.specifier.contains("Widget")));
        let _ = fs::remove_dir_all(&dir);
    }
}
