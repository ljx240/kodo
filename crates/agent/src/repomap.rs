//! Lightweight repo map (L1) — files, languages, packages, symbols.
//! No vector DB. Lexical ContextManager remains L0.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoFileEntry {
    pub path: String,
    pub language: String,
    pub lines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolEntry {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub line: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoMap {
    pub root: String,
    pub files: Vec<RepoFileEntry>,
    pub packages: Vec<String>,
    pub symbols: Vec<SymbolEntry>,
    pub tests: Vec<String>,
    pub git_dirty: Vec<String>,
    pub languages: BTreeMap<String, usize>,
}

impl RepoMap {
    pub fn build(root: &Path, alive: &dyn Fn() -> bool) -> Result<Self, String> {
        let mut map = Self {
            root: root.display().to_string(),
            ..Default::default()
        };
        // Manifests / packages
        if root.join("Cargo.toml").is_file() {
            map.packages.push("rust:cargo".into());
            if let Ok(text) = fs::read_to_string(root.join("Cargo.toml")) {
                for line in text.lines() {
                    let line = line.trim();
                    if let Some(name) = line.strip_prefix("name = ") {
                        map.packages
                            .push(format!("crate:{}", name.trim_matches('"')));
                    }
                    if line.starts_with("members") || line.contains("crates/") {
                        // workspace members discovered via walk below
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
        // Workspace crates / packages directories
        for entry in walk_dirs(root, 0, alive)? {
            if entry.ends_with("Cargo.toml") {
                if let Ok(text) = fs::read_to_string(root.join(&entry)) {
                    if let Some(name) = text.lines().find_map(|l| l.trim().strip_prefix("name = "))
                    {
                        map.packages
                            .push(format!("crate:{}", name.trim_matches('"')));
                    }
                }
            }
            if entry.ends_with("package.json") {
                map.packages.push(format!("pkg:{}", entry));
            }
        }
        map.packages.sort();
        map.packages.dedup();

        walk_files(root, root, &mut map, alive)?;
        map.git_dirty = crate::checkpoint::TurnChangeSet::capture_baseline(root)
            .baseline_dirty
            .into_iter()
            .collect();
        map.languages = map.files.iter().fold(BTreeMap::new(), |mut acc, f| {
            *acc.entry(f.language.clone()).or_insert(0) += 1;
            acc
        });
        Ok(map)
    }

    pub fn find_symbol(&self, query: &str) -> Vec<&SymbolEntry> {
        let q = query.trim();
        self.symbols
            .iter()
            .filter(|s| s.name.contains(q))
            .take(40)
            .collect()
    }

    pub fn list_files(&self, prefix: Option<&str>, limit: usize) -> Vec<&RepoFileEntry> {
        self.files
            .iter()
            .filter(|f| prefix.map(|p| f.path.starts_with(p)).unwrap_or(true))
            .take(limit)
            .collect()
    }

    pub fn to_prompt_block(&self) -> String {
        let mut out = String::from("## Repo map\n");
        out.push_str(&format!("- files: {}\n", self.files.len()));
        out.push_str(&format!("- languages: {:?}\n", self.languages));
        out.push_str(&format!("- packages: {}\n", self.packages.join(", ")));
        out.push_str(&format!("- symbols: {}\n", self.symbols.len()));
        out.push_str(&format!("- tests: {}\n", self.tests.len()));
        out.push_str(&format!("- git_dirty: {}\n", self.git_dirty.join(", ")));
        for file in self.files.iter().take(30) {
            out.push_str(&format!("  - {} ({})\n", file.path, file.language));
        }
        out
    }
}

fn walk_dirs(root: &Path, depth: usize, alive: &dyn Fn() -> bool) -> Result<Vec<String>, String> {
    if depth > 4 || !alive() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(out);
    };
    for entry in entries.flatten() {
        if !alive() {
            return Err("cancelled".into());
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.')
            || matches!(name.as_str(), "node_modules" | "target" | "dist" | "build")
        {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            for sub in walk_dirs(&path, depth + 1, alive)? {
                out.push(format!("{name}/{sub}"));
            }
        } else if name == "Cargo.toml" || name == "package.json" {
            out.push(name);
        }
    }
    Ok(out)
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
        if name.starts_with('.')
            || matches!(name.as_str(), "node_modules" | "target" | "dist" | "build")
        {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk_files(root, &path, map, alive)?;
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| name.clone());
        let language = language_of(&path);
        let is_source = matches!(
            language.as_str(),
            "rust" | "typescript" | "javascript" | "python"
        );
        let lines = if is_source || language == "markdown" {
            fs::read_to_string(&path)
                .map(|t| t.lines().count())
                .unwrap_or(0)
        } else {
            0
        };
        if is_source || language == "markdown" || language == "toml" || language == "json" {
            map.files.push(RepoFileEntry {
                path: rel.clone(),
                language: language.clone(),
                lines,
            });
        }
        if language == "rust" || language == "typescript" || language == "javascript" {
            if let Ok(text) = fs::read_to_string(&path) {
                extract_symbols(&rel, &language, &text, map);
            }
        }
        let lower = rel.to_ascii_lowercase();
        if lower.contains("test")
            || lower.contains("__tests__")
            || lower.ends_with(".test.ts")
            || lower.ends_with(".test.rs")
        {
            map.tests.push(rel);
        }
    }
    map.files.sort_by(|a, b| a.path.cmp(&b.path));
    map.symbols
        .sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    Ok(())
}

/// Naive but useful regex-free symbol scrape for Rust/TS.
fn extract_symbols(path: &str, language: &str, text: &str, map: &mut RepoMap) {
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if language == "rust" {
            for (prefix, kind) in [
                ("pub fn ", "fn"),
                ("fn ", "fn"),
                ("pub struct ", "struct"),
                ("struct ", "struct"),
                ("pub enum ", "enum"),
                ("enum ", "enum"),
                ("pub trait ", "trait"),
                ("trait ", "trait"),
                ("pub type ", "type"),
            ] {
                if let Some(rest) = trimmed.strip_prefix(prefix) {
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                        .collect();
                    if !name.is_empty() {
                        map.symbols.push(SymbolEntry {
                            name,
                            kind: kind.into(),
                            path: path.into(),
                            line: idx + 1,
                        });
                    }
                }
            }
        } else if language == "typescript" || language == "javascript" {
            for (prefix, kind) in [
                ("export function ", "fn"),
                ("function ", "fn"),
                ("export const ", "const"),
                ("const ", "const"),
                ("export class ", "class"),
                ("class ", "class"),
                ("export type ", "type"),
                ("export interface ", "interface"),
            ] {
                if let Some(rest) = trimmed.strip_prefix(prefix) {
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$' || *c == '<')
                        .collect();
                    if !name.is_empty() {
                        map.symbols.push(SymbolEntry {
                            name,
                            kind: kind.into(),
                            path: path.into(),
                            line: idx + 1,
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn builds_map_for_rust_and_ts() {
        let dir = std::env::temp_dir().join(format!("kodo_repomap_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
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
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_range_tool_reads_slice() {
        let dir = std::env::temp_dir().join(format!("kodo_range_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("f.rs"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
        let body = crate::tools::read_text_range(&dir.join("f.rs"), 2, 4, 1000).unwrap();
        assert!(body.contains("l2"));
        assert!(body.contains("l4"));
        assert!(!body.contains("l5"));
        let _ = fs::remove_dir_all(&dir);
    }
}
