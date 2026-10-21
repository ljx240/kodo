//! Locally installed skill roots.
//!
//! Kodo reads the Agent Skills convention (`<name>/SKILL.md`) from its own
//! store and from the other code agents installed on this machine. This
//! module only names the directories and their precedence; parsing and
//! merging live in `kodo-agent::skill`.
//!
//! Precedence is ascending: a root with a higher `priority` wins a name
//! collision. Kodo's own stores sit above external roots (a workspace can
//! pin a skill with `./.kodo/skills`), and built-in skills are anchored
//! separately in the registry.
//!
//! ## Global roots
//!
//! | source       | path                                    | priority |
//! |--------------|-----------------------------------------|----------|
//! | `claude`     | `~/.claude/skills`                      | 10       |
//! | `codex`      | `~/.codex/skills`                       | 11       |
//! | `qoder`      | `~/.qoder/skills`                       | 12       |
//! | `agents`     | `~/.agents/skills`                      | 13       |
//! | `plugin:*`   | `~/.<tool>/plugins/cache/**/skills`     | 14       |
//! | `kodo-home`  | `~/.kodo/skills`                        | 18       |
//!
//! Kodo's own user store (`state_dir()/skills`, priority 20) is added by the
//! entry points because its platform-specific location lives in `settings`.
//!
//! ## Project roots
//!
//! `./.claude/skills` (30), `./.codex/skills` (31), `./.qoder/skills` (32),
//! `./.agents/skills` (33), `./.<tool>/plugins/cache/**/skills` (34). Kodo's
//! project store (`./.kodo/skills`, priority 40) is added by the caller.
//!
//! Every external root is read-only: Kodo never writes into another agent's
//! directory (the 插件 page hides edit/delete for these rows).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Where a root sits relative to the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillScope {
    Global,
    Project,
}

impl SkillScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

/// One directory scanned for skills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRoot {
    /// Stable label shown in the UI (`claude`, `plugin:qoder`, `kodo-home`, …).
    pub source: String,
    pub scope: SkillScope,
    pub path: PathBuf,
    /// Higher wins a name collision.
    pub priority: u8,
    /// External roots are read-only; Kodo's own stores are writable.
    pub read_only: bool,
}

impl SkillRoot {
    pub fn new(
        source: impl Into<String>,
        scope: SkillScope,
        path: impl Into<PathBuf>,
        priority: u8,
        read_only: bool,
    ) -> Self {
        Self {
            source: source.into(),
            scope,
            path: path.into(),
            priority,
            read_only,
        }
    }
}

/// `$HOME`, or `None` when it is unset or empty.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// Every external root on this machine (global first, then project).
pub fn local_skill_roots(project: Option<&Path>) -> Vec<SkillRoot> {
    match home_dir() {
        Some(home) => local_skill_roots_in(&home, project),
        None => project.map(project_roots).unwrap_or_default(),
    }
}

/// Testable form of [`local_skill_roots`].
pub fn local_skill_roots_in(home: &Path, project: Option<&Path>) -> Vec<SkillRoot> {
    let mut roots = Vec::new();
    for (tool, priority) in [("claude", 10), ("codex", 11), ("qoder", 12), ("agents", 13)] {
        roots.push(SkillRoot::new(
            tool,
            SkillScope::Global,
            home.join(format!(".{tool}")).join("skills"),
            priority,
            true,
        ));
    }
    for (source, path) in plugin_skill_dirs(home) {
        roots.push(SkillRoot::new(source, SkillScope::Global, path, 14, true));
    }
    roots.push(SkillRoot::new(
        "kodo-home",
        SkillScope::Global,
        home.join(".kodo").join("skills"),
        18,
        true,
    ));
    if let Some(project) = project {
        roots.extend(project_roots(project));
    }
    roots
}

fn project_roots(project: &Path) -> Vec<SkillRoot> {
    let mut roots = Vec::new();
    for (tool, priority) in [("claude", 30), ("codex", 31), ("qoder", 32), ("agents", 33)] {
        roots.push(SkillRoot::new(
            tool,
            SkillScope::Project,
            project.join(format!(".{tool}")).join("skills"),
            priority,
            true,
        ));
    }
    for (source, path) in plugin_skill_dirs(project) {
        roots.push(SkillRoot::new(source, SkillScope::Project, path, 34, true));
    }
    roots
}

/// Installed plugin skill directories, sorted by source then path.
///
/// Only `plugins/cache` is scanned: the `marketplaces` tree is the catalog of
/// everything *available*, not what this user installed, and listing it would
/// flood the picker with skills that were never enabled.
fn plugin_skill_dirs(base: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for tool in ["claude", "codex", "qoder"] {
        let cache = base.join(format!(".{tool}")).join("plugins").join("cache");
        if !cache.is_dir() {
            continue;
        }
        let mut found = Vec::new();
        walk_for_skills(&cache, 0, &mut found);
        found.sort();
        found.dedup();
        for path in found {
            out.push((format!("plugin:{tool}"), path));
        }
    }
    out
}

/// Depth from `plugins/cache` to a plugin's `skills/` directory:
/// `marketplace/plugin/version/skills` is four levels, plus headroom for
/// nested platform directories.
const MAX_PLUGIN_DEPTH: usize = 6;

fn walk_for_skills(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_PLUGIN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    children.sort();
    for path in children {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if skipped_plugin_dir(name) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            // A marketplace may link outside its cache; stay in the user's own
            // tree and let the direct roots pick up linked skills.
            continue;
        }
        if !meta.is_dir() {
            continue;
        }
        if name == "skills" {
            out.push(path);
            continue;
        }
        walk_for_skills(&path, depth + 1, out);
    }
}

fn skipped_plugin_dir(name: &str) -> bool {
    matches!(name, ".git" | "node_modules" | "data" | ".tmp")
        || name.starts_with(".plugin-appserver")
        || matches!(
            name,
            ".remote-plugin-install-staging" | ".marketplace-plugin-source-staging"
        )
}

/// Roots whose directories exist, with duplicate paths removed (the first,
/// highest-priority root wins).
pub fn existing_roots(roots: Vec<SkillRoot>) -> Vec<SkillRoot> {
    let mut seen = HashSet::new();
    roots
        .into_iter()
        .filter(|root| root.path.is_dir())
        .filter(|root| seen.insert(root.path.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn external_roots_are_ordered_below_kodo_user_store() {
        let tmp = TempDir::new("skill-roots");
        let home = tmp.dir("home");
        let project = tmp.dir("project");
        let roots = local_skill_roots_in(&home, Some(&project));
        let priorities: Vec<u8> = roots
            .iter()
            .filter(|root| root.scope == SkillScope::Global)
            .map(|root| root.priority)
            .collect();
        assert_eq!(priorities, vec![10, 11, 12, 13, 18]);
        let project_priorities: Vec<u8> = roots
            .iter()
            .filter(|root| root.scope == SkillScope::Project)
            .map(|root| root.priority)
            .collect();
        assert_eq!(project_priorities, vec![30, 31, 32, 33]);
        assert!(roots.iter().all(|root| root.read_only));
    }

    #[test]
    fn plugin_skill_dirs_are_found_in_the_installed_cache() {
        let tmp = TempDir::new("plugin-skills");
        let home = tmp.dir("home");
        std::fs::create_dir_all(
            home.join(".codex")
                .join("plugins")
                .join("cache")
                .join("openai-curated")
                .join("superpowers")
                .join("1dc19589")
                .join("skills")
                .join("brainstorming"),
        )
        .unwrap();
        std::fs::create_dir_all(
            home.join(".qoder")
                .join("plugins")
                .join("cache")
                .join("qoder-marketplace")
                .join("Presentations")
                .join("0.1.3")
                .join("skills")
                .join("pptx"),
        )
        .unwrap();
        // The marketplace catalog is not an installed plugin.
        std::fs::create_dir_all(
            home.join(".claude")
                .join("plugins")
                .join("marketplaces")
                .join("official")
                .join("plugin")
                .join("skills")
                .join("unused"),
        )
        .unwrap();
        // Runtime data may have a nested `skills` dir that is not a skill root.
        std::fs::create_dir_all(
            home.join(".claude")
                .join("plugins")
                .join("cache")
                .join("data")
                .join("skills"),
        )
        .unwrap();

        let roots = plugin_skill_dirs(&home);
        let sources: Vec<&str> = roots.iter().map(|(source, _)| source.as_str()).collect();
        assert_eq!(sources, vec!["plugin:codex", "plugin:qoder"]);
        assert!(roots[0].1.ends_with("superpowers/1dc19589/skills"));
        assert!(roots[1].1.ends_with("Presentations/0.1.3/skills"));
    }

    #[test]
    fn project_roots_survive_a_missing_home() {
        let tmp = TempDir::new("skill-project-only");
        let project = tmp.dir("project");
        let roots = local_skill_roots_in(&tmp.path().join("missing-home"), Some(&project));
        assert_eq!(
            roots
                .iter()
                .filter(|root| root.scope == SkillScope::Project)
                .count(),
            4
        );
    }

    #[test]
    fn existing_roots_drops_missing_and_duplicate_paths() {
        let tmp = TempDir::new("skill-existing");
        let dir = tmp.dir("skills");
        let roots = vec![
            SkillRoot::new("a", SkillScope::Global, &dir, 10, true),
            SkillRoot::new("b", SkillScope::Global, &dir, 11, true),
            SkillRoot::new(
                "missing",
                SkillScope::Global,
                tmp.path().join("nope"),
                12,
                true,
            ),
        ];
        let existing = existing_roots(roots);
        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0].source, "a");
    }
}
