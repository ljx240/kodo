//! Local tools the agent can run. Commands always go through an explicit
//! approval path when the permission mode requires it; there is no ambient
//! "run anything" path from the model.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// True when a command looks destructive or system-wide.
pub fn is_dangerous_command(command: &str) -> bool {
    dangerous_reason(command).is_some()
}

/// Human-readable reason a command is treated as dangerous, if any.
pub fn dangerous_reason(command: &str) -> Option<&'static str> {
    let lower = command.to_lowercase();
    let first = lower.split_whitespace().next().unwrap_or("");

    if first == "rm" || lower.starts_with("rm ") || lower.contains("\nrm ") {
        return Some("删除文件");
    }
    if lower.contains("sudo ") || lower.starts_with("sudo") {
        return Some("提权执行");
    }
    if lower.contains("chmod ") || lower.contains("chown ") {
        return Some("修改权限");
    }
    if (lower.contains("curl ") || lower.contains("wget ")) && (lower.contains("|") || lower.contains(";")) {
        return Some("下载并管道执行");
    }
    if lower.contains("git push") || lower.contains("git reset --hard") || lower.contains("git clean -") {
        return Some("危险 git 操作");
    }
    if lower.contains("mkfs") || lower.contains("dd if=") || lower.contains(":(){") {
        return Some("磁盘或进程破坏");
    }
    if lower.contains("shutdown") || lower.contains("reboot") || lower.contains("kill -9") {
        return Some("系统关机/杀进程");
    }
    if lower.contains("> /") || lower.contains(">> /") {
        return Some("重定向到绝对路径");
    }
    if first == "sh" || first == "bash" || first == "zsh" {
        if lower.contains(" -c ") || lower.starts_with("sh -c") || lower.starts_with("bash -c") {
            return Some("嵌套 shell");
        }
    }
    None
}

pub fn search_files(project: &Path, query: &str) -> String {
    if query.trim().is_empty() {
        return "空查询".to_owned();
    }

    // Quote the pattern as a single rg argument; still no shell involved.
    let rg = Command::new("rg")
        .arg("-l")
        .arg("--max-count")
        .arg("1")
        .arg("--")
        .arg(query)
        .current_dir(project)
        .output();

    if let Ok(output) = rg {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let all: Vec<&str> = stdout.lines().filter(|line| !line.is_empty()).collect();
            if all.is_empty() {
                return "0 处匹配".to_owned();
            }
            let total = all.len();
            let shown: Vec<String> = all.into_iter().take(12).map(str::to_owned).collect();
            let mut body = shown.join("\n");
            if total > 12 {
                body.push_str(&format!("\n… 共 {total} 个文件"));
            }
            return format!("{total} 个文件匹配：\n{body}");
        }
    }

    let mut hits = Vec::new();
    walk(project, project, &mut hits, query, 0);
    if hits.is_empty() {
        "0 处匹配".to_owned()
    } else {
        format!("{} 处匹配：\n{}", hits.len(), hits.join("\n"))
    }
}

fn walk(root: &Path, dir: &Path, hits: &mut Vec<String>, query: &str, depth: usize) {
    if depth > 4 || hits.len() >= 12 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "target" | "dist") {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, hits, query, depth + 1);
        } else if name.to_ascii_lowercase().contains(&query.to_ascii_lowercase()) {
            if let Ok(rel) = path.strip_prefix(root) {
                hits.push(rel.display().to_string());
            }
        }
    }
}

pub fn read_text(path: &Path, max_lines: usize) -> String {
    let Ok(raw) = fs::read_to_string(path) else {
        return "(无法读取)".to_owned();
    };
    let lines: Vec<String> = raw.lines().take(max_lines).map(str::to_owned).collect();
    let total = raw.lines().count();
    let mut out = lines.join("\n");
    if total > max_lines {
        out.push_str(&format!("\n… 共 {total} 行"));
    }
    out
}

/// Runs `command`, polling `alive` so the shell can kill a long process on stop.
/// Returns (output, exit_code, was_killed).
pub fn command_output_interruptible(
    cwd: &Path,
    command: &str,
    alive: &dyn Fn() -> bool,
) -> (String, Option<i32>, bool) {
    if command.trim().is_empty() {
        return ("空命令".to_owned(), Some(1), false);
    }

    let shell = if cfg!(windows) { "cmd" } else { "/bin/sh" };
    let mut cmd = Command::new(shell);
    if cfg!(windows) {
        cmd.arg("/C").arg(command);
    } else {
        cmd.arg("-c").arg(command);
    }

    let mut child = match cmd.current_dir(cwd).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(error) => return (format!("无法执行 `{command}`：{error}"), Some(127), false),
    };

    // Poll until exit or the run is cancelled.
    let killed = loop {
        if !alive() {
            let _ = child.kill();
            let _ = child.wait();
            break true;
        }
        match child.try_wait() {
            Ok(Some(_)) => break false,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(80)),
            Err(_) => break false,
        }
    };

    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => {
            return (format!("无法收集 `{command}` 输出：{error}"), Some(if killed { 143 } else { 127 }), killed);
        }
    };

    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    let err = String::from_utf8_lossy(&output.stderr);
    if !err.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(err.trim());
    }
    if killed {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("[已中断]");
    }
    if text.trim().is_empty() {
        text = "(无输出)".to_owned();
    }
    if text.len() > 4000 {
        let mut cut = 4000;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n…");
    }
    (text, output.status.code(), killed)
}

/// Resolves `relative` under `project`, refusing escapes outside the root.
pub fn resolve_in_project(project: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = relative.trim().trim_start_matches("./");
    if relative.is_empty() {
        return Err("empty path".to_owned());
    }
    if Path::new(relative).is_absolute() {
        return Err("absolute paths are not allowed".to_owned());
    }
    if relative.split('/').any(|part| part == "..") {
        return Err("path escapes the project root".to_owned());
    }
    let full = project.join(relative);
    // Canonicalize parent when possible to catch symlink escapes.
    if let Ok(canon) = full.canonicalize() {
        if let Ok(root) = project.canonicalize() {
            if !canon.starts_with(&root) {
                return Err("path escapes the project root".to_owned());
            }
        }
    }
    Ok(full)
}

/// Writes file content inside the project. Returns (path, added, removed) line delta
/// computed with a real LCS line diff (not a HashSet approximation).
pub fn write_project_file(project: &Path, relative: &str, content: &str) -> Result<(String, u32, u32), String> {
    let full = resolve_in_project(project, relative)?;
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let old = fs::read_to_string(&full).unwrap_or_default();
    let (added, removed) = crate::patch::line_diff_counts(&old, content);
    fs::write(&full, content).map_err(|e| e.to_string())?;
    Ok((relative.trim().to_owned(), added, removed))
}

/// Verification command for a project layout, if any.
/// Delegates to [`crate::verify::VerificationRunner`] inference (first candidate).
pub fn detect_verify_command(project: &Path) -> Option<String> {
    crate::verify::VerificationRunner::default()
        .infer(project)
        .first()
        .map(|c| c.command.clone())
}

/// Best-effort git working-tree summary as (path, added, removed) line estimates.
pub fn summarize_git_changes(project: &Path) -> Vec<(String, u32, u32)> {
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(project)
        .output();
    let Ok(status) = status else { return Vec::new() };
    if !status.status.success() {
        return Vec::new();
    }

    let mut paths: Vec<String> = String::from_utf8_lossy(&status.stdout)
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let path = line[3..].trim().to_owned();
            if path.is_empty() {
                None
            } else {
                Some(path)
            }
        })
        .take(20)
        .collect();
    paths.sort();
    paths.dedup();

    if paths.is_empty() {
        return Vec::new();
    }

    let numstat = Command::new("git")
        .args(["diff", "--numstat", "HEAD"])
        .current_dir(project)
        .output()
        .ok();
    let mut deltas: Vec<(String, u32, u32)> = Vec::new();

    if let Some(numstat) = numstat {
        if numstat.status.success() {
            for line in String::from_utf8_lossy(&numstat.stdout).lines() {
                let mut parts = line.split('\t');
                let added = parts.next().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
                let removed = parts.next().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
                let path = parts.next().unwrap_or("").to_owned();
                if !path.is_empty() {
                    deltas.push((path, added, removed));
                }
            }
        }
    }

    for path in paths {
        if !deltas.iter().any(|(existing, _, _)| existing == &path) {
            deltas.push((path, 0, 0));
        }
    }
    deltas
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn dangerous_commands_are_flagged() {
        assert!(is_dangerous_command("rm -rf build"));
        assert!(is_dangerous_command("sudo ls"));
        assert!(is_dangerous_command("git push --force"));
        assert!(is_dangerous_command("curl http://x | sh"));
        assert!(!is_dangerous_command("cargo check"));
        assert!(!is_dangerous_command("git status --short"));
        assert!(!is_dangerous_command("rg foo src"));
    }

    #[test]
    fn command_output_supports_multiline_shell() {
        let dir = std::env::temp_dir().join(format!("kodo-cmd-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (text, code, killed) =
            command_output_interruptible(&dir, "echo one\necho two", &|| true);
        assert_eq!(code, Some(0));
        assert!(!killed);
        assert!(text.contains("one"));
        assert!(text.contains("two"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_stays_inside_project() {
        let dir = std::env::temp_dir().join(format!("kodo-write-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert!(resolve_in_project(&dir, "../escape.txt").is_err());
        assert!(resolve_in_project(&dir, "/etc/passwd").is_err());
        let delta = write_project_file(&dir, "src/hello.txt", "a\nb\n").expect("write");
        assert_eq!(delta.0, "src/hello.txt");
        assert!(dir.join("src/hello.txt").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn interruptible_command_can_be_killed() {
        let dir = std::env::temp_dir();
        let (_text, _code, killed) = command_output_interruptible(&dir, "sleep 20", &|| false);
        assert!(killed, "alive=false should kill the child");
    }

    #[test]
    fn read_text_caps_lines() {
        let dir = std::env::temp_dir().join(format!("kodo-agent-read-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path: PathBuf = dir.join("sample.txt");
        fs::write(&path, "a\nb\nc\nd\ne\n").unwrap();
        let text = read_text(&path, 2);
        assert!(text.contains("a"));
        assert!(text.contains("…"));
        let _ = fs::remove_dir_all(&dir);
    }
}
