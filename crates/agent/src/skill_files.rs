//! File access for one skill folder.
//!
//! The 插件 · 技能 detail view shows the real files a skill ships
//! (`SKILL.md`, `references/…`, `scripts/…`) instead of only the parsed
//! markdown. This module owns the boundary: every path is relative to a
//! canonical skill directory, `..`/absolute/empty paths are refused, and
//! symlinks are never followed or written through.
//!
//! Preview and write limits are deliberately small — a skill is prompt
//! material, not a general-purpose file browser.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Largest auxiliary file this module will load for preview or accept for
/// writing. `SKILL.md` has a lower cap enforced by discovery / the caller.
pub const MAX_SKILL_FILE_BYTES: u64 = 512 * 1024;
/// Upper bound on the tree listing; a runaway directory must not freeze the
/// modal. Real skills have a handful of reference files.
pub const MAX_SKILL_TREE_ENTRIES: usize = 300;
const MAX_TREE_DEPTH: usize = 4;
const MAX_RELATIVE_PATH_CHARS: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFileEntry {
    /// Relative path with `/` separators, e.g. `references/style-guide.md`.
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFileContent {
    pub path: String,
    /// `None` for binary files (or a directory that disappeared mid-read).
    pub content: Option<String>,
    pub binary: bool,
    pub truncated: bool,
    pub size: u64,
}

/// Every regular file under `root`, sorted with `SKILL.md` first. Hidden
/// entries and symlinks are skipped; the walk is bounded by depth and count.
pub fn list_skill_files(root: &Path) -> Result<Vec<SkillFileEntry>, String> {
    let root = canonical_root(root)?;
    let mut out = Vec::new();
    walk(&root, &root, 0, &mut out);
    out.sort_by(|a, b| {
        let a_skill = a.path == "SKILL.md";
        let b_skill = b.path == "SKILL.md";
        b_skill.cmp(&a_skill).then_with(|| a.path.cmp(&b.path))
    });
    Ok(out)
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<SkillFileEntry>) {
    if depth > MAX_TREE_DEPTH || out.len() >= MAX_SKILL_TREE_ENTRIES {
        return;
    }
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = read.flatten().map(|entry| entry.path()).collect();
    children.sort();
    for path in children {
        if out.len() >= MAX_SKILL_TREE_ENTRIES {
            return;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            walk(root, &path, depth + 1, out);
        } else if meta.is_file() {
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            out.push(SkillFileEntry {
                path: relative.to_string_lossy().replace('\\', "/"),
                size: meta.len(),
            });
        }
    }
}

/// Read one file inside `root`. Oversized (or still growing) files are
/// truncated in the preview; binary files come back with `content: None`
/// so the UI can say so instead of rendering replacement characters.
pub fn read_skill_file(root: &Path, relative: &str) -> Result<SkillFileContent, String> {
    let root = canonical_root(root)?;
    let relative = relative_path(relative)?;
    let path = checked_file(&root, &relative)?;
    let meta = fs::metadata(&path).map_err(|error| format!("无法读取文件：{error}"))?;
    let mut file = fs::File::open(&path).map_err(|error| format!("无法读取文件：{error}"))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_SKILL_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("无法读取文件：{error}"))?;
    let truncated = meta.len() > MAX_SKILL_FILE_BYTES;
    if truncated {
        bytes.truncate(MAX_SKILL_FILE_BYTES as usize);
    }
    let has_nul = bytes.iter().take(8192).any(|byte| *byte == 0);
    let (binary, content) = match String::from_utf8(bytes) {
        Ok(_) if has_nul => (true, None),
        Ok(text) => (false, Some(text)),
        Err(error) if truncated => {
            // A truncated multi-byte character is not evidence of a binary
            // file; cut at the last valid boundary instead.
            let bytes = error.into_bytes();
            let mut end = bytes.len();
            while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
                end -= 1;
            }
            (
                false,
                Some(String::from_utf8_lossy(&bytes[..end]).into_owned()),
            )
        }
        Err(_) => (true, None),
    };
    Ok(SkillFileContent {
        path: relative.to_string_lossy().replace('\\', "/"),
        content,
        binary,
        truncated,
        size: meta.len(),
    })
}

/// Overwrite one existing regular file inside `root`. New files and symlinks
/// are refused: the viewer edits what the skill ships, it does not grow it.
pub fn write_skill_file(root: &Path, relative: &str, content: &str) -> Result<(), String> {
    if content.len() as u64 > MAX_SKILL_FILE_BYTES {
        return Err(format!(
            "文件过大（上限 {} KiB）",
            MAX_SKILL_FILE_BYTES / 1024
        ));
    }
    let root = canonical_root(root)?;
    let relative = relative_path(relative)?;
    let path = checked_file(&root, &relative)?;
    fs::write(&path, content).map_err(|error| format!("无法保存文件：{error}"))
}

fn canonical_root(root: &Path) -> Result<PathBuf, String> {
    let root = fs::canonicalize(root).map_err(|error| format!("无法读取技能目录：{error}"))?;
    if !root.is_dir() {
        return Err("技能目录不存在".to_owned());
    }
    Ok(root)
}

/// Relative paths only: no absolute roots, no `.`/`..`, no empty components.
fn relative_path(raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty() || raw.chars().count() > MAX_RELATIVE_PATH_CHARS {
        return Err("非法文件路径".to_owned());
    }
    let mut out = PathBuf::new();
    for component in Path::new(raw).components() {
        let Component::Normal(part) = component else {
            return Err("非法文件路径".to_owned());
        };
        let Some(text) = part.to_str() else {
            return Err("非法文件路径".to_owned());
        };
        if text.is_empty() || text.len() > 255 {
            return Err("非法文件路径".to_owned());
        }
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        return Err("非法文件路径".to_owned());
    }
    Ok(out)
}

/// Resolve `relative` under `root`, refusing symlinks at every level. The
/// final path must already be a regular file.
fn checked_file(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err("非法文件路径".to_owned());
        };
        current.push(part);
        let meta = fs::symlink_metadata(&current).map_err(|_| "文件不存在".to_owned())?;
        if meta.file_type().is_symlink() {
            return Err("技能目录中不允许符号链接".to_owned());
        }
    }
    let meta = fs::symlink_metadata(&current).map_err(|_| "文件不存在".to_owned())?;
    if !meta.is_file() {
        return Err("不是文件".to_owned());
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "kodo_skill_files_{tag}_{}_{}",
            std::process::id(),
            stamp
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn write(root: &Path, relative: &str, body: impl AsRef<[u8]>) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        fs::write(path, body).expect("write");
    }

    #[test]
    fn lists_real_files_with_skill_md_first_and_skips_hidden() {
        let dir = temp_dir("list");
        write(&dir, "references/style-guide.md", "# Style\n");
        write(&dir, "scripts/init.py", "print('hi')\n");
        write(&dir, "LICENSE.txt", "MIT\n");
        write(&dir, ".hidden", "nope\n");
        write(&dir, ".git/config", "nope\n");

        let files = list_skill_files(&dir).expect("list");
        let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "LICENSE.txt",
                "references/style-guide.md",
                "scripts/init.py"
            ]
        );

        write(&dir, "SKILL.md", "# Skill: demo\n");
        let files = list_skill_files(&dir).expect("list");
        assert_eq!(files[0].path, "SKILL.md");
        assert_eq!(files[0].size, "# Skill: demo\n".len() as u64);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reads_text_binary_and_large_files() {
        let dir = temp_dir("read");
        write(&dir, "SKILL.md", "# Skill: demo\n");
        write(&dir, "references/notes.md", "hello\n");
        write(&dir, "assets/icon.bin", [0u8, 159, 146, 150]);
        write(
            &dir,
            "references/large.txt",
            vec![b'a'; (MAX_SKILL_FILE_BYTES + 1) as usize],
        );

        let text = read_skill_file(&dir, "references/notes.md").expect("text");
        assert_eq!(text.content.as_deref(), Some("hello\n"));
        assert!(!text.binary && !text.truncated);

        let binary = read_skill_file(&dir, "assets/icon.bin").expect("binary");
        assert!(binary.binary);
        assert!(binary.content.is_none());

        let large = read_skill_file(&dir, "references/large.txt").expect("large");
        assert!(large.truncated);
        assert_eq!(
            large.content.as_deref().map(str::len),
            Some(MAX_SKILL_FILE_BYTES as usize)
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rejects_traversal_absolute_and_unknown_paths() {
        let dir = temp_dir("guards");
        write(&dir, "SKILL.md", "# Skill: demo\n");
        write(&dir, "outside.txt", "must stay\n");

        for bad in [
            "../outside.txt",
            "/etc/passwd",
            "",
            "./SKILL.md",
            "missing.md",
        ] {
            assert!(
                read_skill_file(&dir, bad).is_err(),
                "expected `{bad}` to be refused"
            );
        }
        assert!(write_skill_file(&dir, "../outside.txt", "x").is_err());
        assert!(write_skill_file(&dir, "new.md", "x").is_err());
        fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_listed_or_followed() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("symlink");
        let outside = temp_dir("symlink_outside");
        write(&dir, "SKILL.md", "# Skill: demo\n");
        fs::write(outside.join("secret.txt"), "secret\n").expect("secret");
        symlink(outside.join("secret.txt"), dir.join("secret.txt")).expect("link file");
        symlink(&outside, dir.join("linked")).expect("link dir");
        symlink(&outside, dir.join("linked-dir")).expect("link dir 2");

        let files = list_skill_files(&dir).expect("list");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "SKILL.md");
        assert!(read_skill_file(&dir, "secret.txt").is_err());
        assert!(read_skill_file(&dir, "linked/secret.txt").is_err());
        assert!(write_skill_file(&dir, "secret.txt", "x").is_err());
        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(outside).ok();
    }

    #[test]
    fn writes_an_existing_text_file_without_creating_files() {
        let dir = temp_dir("write");
        write(&dir, "SKILL.md", "# Skill: old\n");
        write(&dir, "references/notes.md", "old\n");

        write_skill_file(&dir, "references/notes.md", "new\n").expect("write");
        assert_eq!(
            fs::read_to_string(dir.join("references/notes.md")).expect("read"),
            "new\n"
        );
        assert!(!dir.join("references/new.md").exists());

        let too_large = "x".repeat((MAX_SKILL_FILE_BYTES + 1) as usize);
        assert!(write_skill_file(&dir, "references/notes.md", &too_large).is_err());
        fs::remove_dir_all(dir).ok();
    }
}
