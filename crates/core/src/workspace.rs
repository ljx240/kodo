//! The project list the user manages by hand.
//!
//! There is no scanning and no workspace root: a project is on this list because
//! the user added it or created it. The list lives in an append-only
//! `projects.log` and the current state is the **ordered fold** of its lines.
//!
//! An ordered fold, not last-value-wins: `add`, `remove`, then `add` again must
//! end with the project present. `settings::read` cannot express that (it keeps
//! the last value of a key), which is why this module splits its own lines.
//!
//! ```text
//! add     <path>
//! rename  <path>  <display>
//! remove  <path>
//! order   <path>  <path>  ...
//! ```
//!
//! Reordering writes the **whole order** rather than a single move. A move needs
//! every earlier move replayed correctly to know where things ended up; a whole
//! order is idempotent, so the last one is simply the answer.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::line;
use crate::settings;

const PROJECTS_FILE: &str = "projects.log";

/// A project on the list. `name` is a display label only — renaming one never
/// touches the directory on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub path: PathBuf,
    pub name: String,
}

#[derive(Debug)]
pub enum Error {
    /// `name` is not a single ordinary path component.
    BadName(String),
    /// The directory to create is already there.
    Exists(PathBuf),
    /// Not an existing directory — missing, or a file.
    NotADirectory(PathBuf),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::BadName(name) => write!(f, "not a usable project name: {name:?}"),
            Error::Exists(path) => write!(f, "already exists: {}", path.display()),
            Error::NotADirectory(path) => {
                write!(f, "not an existing directory: {}", path.display())
            }
            Error::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::Io(error)
    }
}

/// The project list, or `None` when `HOME` is unset.
pub fn log_path() -> Option<PathBuf> {
    Some(settings::state_dir()?.join(PROJECTS_FILE))
}

/// The current list, in the order the user arranged it.
pub fn load(log: &Path) -> io::Result<Vec<Project>> {
    Ok(fold(&line::read_lines(log)?))
}

/// Replays the log. Blank lines, `#` comments, unknown verbs and lines with the
/// wrong number of fields are skipped: a damaged line costs one record, not the
/// whole file.
fn fold(lines: &[String]) -> Vec<Project> {
    let mut projects: Vec<Project> = Vec::new();

    for raw in lines {
        if raw.trim().is_empty() || raw.starts_with('#') {
            continue;
        }
        let fields = line::split(raw);
        match (fields[0].as_str(), &fields[1..]) {
            ("add", [path]) => {
                let path = PathBuf::from(path);
                if !projects.iter().any(|entry| entry.path == path) {
                    let name = display_name(&path);
                    projects.push(Project { path, name });
                }
            }
            ("rename", [path, name]) => {
                let path = PathBuf::from(path);
                if let Some(entry) = projects.iter_mut().find(|entry| entry.path == path) {
                    entry.name = name.clone();
                }
            }
            ("remove", [path]) => {
                let path = PathBuf::from(path);
                projects.retain(|entry| entry.path != path);
            }
            ("order", paths) => {
                let mut ordered: Vec<Project> = Vec::with_capacity(projects.len());
                for wanted in paths.iter().map(PathBuf::from) {
                    if let Some(index) = projects.iter().position(|entry| entry.path == wanted) {
                        ordered.push(projects.remove(index));
                    }
                }
                // Anything the order line did not name keeps its relative place
                // at the end, so a partial order is still a total order.
                ordered.append(&mut projects);
                projects = ordered;
            }
            _ => {}
        }
    }

    projects
}

/// Registers an existing directory. Adding one that is already listed is a
/// no-op, so the log does not grow on a double click.
pub fn add(log: &Path, project: &Path) -> Result<(), Error> {
    if !project.is_dir() {
        return Err(Error::NotADirectory(project.to_path_buf()));
    }
    if load(log)?.iter().any(|entry| entry.path == project) {
        return Ok(());
    }
    append(log, &["add", &project.to_string_lossy()])
}

/// Changes the display name only. The directory is untouched.
pub fn rename(log: &Path, project: &Path, name: &str) -> Result<(), Error> {
    if !load(log)?.iter().any(|entry| entry.path == project) {
        return Ok(());
    }
    append(log, &["rename", &project.to_string_lossy(), name])
}

/// Drops the project from the list. The directory is untouched.
pub fn remove(log: &Path, project: &Path) -> Result<(), Error> {
    append(log, &["remove", &project.to_string_lossy()])
}

/// Moves `project` to `index`, writing the resulting order.
pub fn reorder(log: &Path, project: &Path, index: usize) -> Result<(), Error> {
    let mut projects = load(log)?;
    let Some(from) = projects.iter().position(|entry| entry.path == project) else {
        return Ok(());
    };

    let moved = projects.remove(from);
    let to = index.min(projects.len());
    projects.insert(to, moved);

    let paths: Vec<String> = projects
        .iter()
        .map(|entry| entry.path.to_string_lossy().into_owned())
        .collect();
    let mut fields: Vec<&str> = Vec::with_capacity(paths.len() + 1);
    fields.push("order");
    fields.extend(paths.iter().map(String::as_str));
    append(log, &fields)
}

/// Creates the directory `parent/name` and returns it.
///
/// This is the only path in Kodo that writes to the user's disk, so it is
/// deliberately narrow:
///
/// - the name must be **exactly one ordinary component**. That single positive
///   check rejects `/etc/x` (which `parent.join` would silently turn into an
///   absolute path, discarding `parent`), `..`, `.`, `""` and `a/b`;
/// - it calls `create_dir`, never `create_dir_all`, so it cannot materialize a
///   chain of directories as a side effect;
/// - it never overwrites, and does not pre-check with `exists()` — letting
///   `create_dir` report `AlreadyExists` avoids a time-of-check/time-of-use race
///   and covers case-insensitive collisions on APFS.
pub fn create(parent: &Path, name: &str) -> Result<PathBuf, Error> {
    let mut components = Path::new(name).components();
    let target = match (components.next(), components.next()) {
        (Some(Component::Normal(part)), None) => parent.join(part),
        _ => return Err(Error::BadName(name.to_owned())),
    };

    if let Err(error) = fs::create_dir(&target) {
        return Err(match error.kind() {
            io::ErrorKind::AlreadyExists => Error::Exists(target),
            _ => Error::Io(error),
        });
    }
    Ok(target)
}

/// The branch a project is on, read straight out of `.git/HEAD`.
///
/// `None` for anything that is not a checkout, and for a repository whose HEAD
/// cannot be read. A detached HEAD has no branch, so it reports the short commit
/// instead — which is what the chip should show, rather than an empty label.
pub fn branch(project: &Path) -> Option<String> {
    let head = fs::read_to_string(git_dir(project)?.join("HEAD")).ok()?;
    let head = head.trim();

    Some(match head.strip_prefix("ref: refs/heads/") {
        Some(name) => name.to_owned(),
        None => head.chars().take(7).collect(),
    })
}

/// `.git` is a directory in an ordinary checkout and a *file* holding
/// `gitdir: <path>` in a worktree or a submodule. Both are common enough that
/// reading only the first would report no branch for a perfectly normal repo.
fn git_dir(project: &Path) -> Option<PathBuf> {
    let dot = project.join(".git");
    if dot.is_dir() {
        return Some(dot);
    }

    let pointer = fs::read_to_string(&dot).ok()?;
    let target = pointer.trim().strip_prefix("gitdir:")?.trim();
    let target = Path::new(target);
    Some(if target.is_absolute() {
        target.to_path_buf()
    } else {
        project.join(target)
    })
}

fn append(log: &Path, fields: &[&str]) -> Result<(), Error> {
    line::append(log, &line::join(fields))?;
    Ok(())
}

/// The directory's own name, falling back to the whole path when there is none
/// (a filesystem root, say).
fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn seeded(tag: &str, lines: &[&str]) -> (TempDir, PathBuf, PathBuf) {
        let tmp = TempDir::new(tag);
        let log = tmp.path().join("projects.log");
        for line in lines {
            line::append(&log, line).expect("seed the log");
        }
        let dir = tmp.dir("a-project");
        (tmp, log, dir)
    }

    #[test]
    fn an_added_project_is_listed_with_its_directory_name() {
        let (_tmp, log, dir) = seeded("ws-add", &[]);
        add(&log, &dir).expect("add");

        let projects = load(&log).expect("load");
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].path, dir);
        assert_eq!(projects[0].name, "a-project");
    }

    #[test]
    fn adding_twice_does_not_grow_the_log() {
        let (_tmp, log, dir) = seeded("ws-add-twice", &[]);
        add(&log, &dir).expect("first add");
        add(&log, &dir).expect("second add");

        assert_eq!(load(&log).expect("load").len(), 1);
        assert_eq!(line::read_lines(&log).expect("read").len(), 1);
    }

    #[test]
    fn adding_something_that_is_not_a_directory_fails() {
        let (_tmp, log, _dir) = seeded("ws-add-missing", &[]);
        let missing = PathBuf::from("/definitely/not/here");
        assert!(matches!(add(&log, &missing), Err(Error::NotADirectory(_))));
    }

    #[test]
    fn renaming_changes_the_label_and_not_the_path() {
        let (_tmp, log, dir) = seeded("ws-rename", &[]);
        add(&log, &dir).expect("add");
        rename(&log, &dir, "实时数仓").expect("rename");

        let projects = load(&log).expect("load");
        assert_eq!(projects[0].name, "实时数仓");
        assert_eq!(projects[0].path, dir);
    }

    #[test]
    fn removing_forgets_the_project_and_leaves_the_directory() {
        let (_tmp, log, dir) = seeded("ws-remove", &[]);
        add(&log, &dir).expect("add");
        remove(&log, &dir).expect("remove");

        assert!(load(&log).expect("load").is_empty());
        assert!(dir.is_dir(), "removing must not touch the directory");
    }

    #[test]
    fn add_remove_add_ends_up_listed() {
        // The reason this module folds its own lines: settings::read would only
        // remember the last value of each key and could not express this.
        let (_tmp, log, dir) = seeded("ws-readd", &[]);
        add(&log, &dir).expect("add");
        remove(&log, &dir).expect("remove");
        add(&log, &dir).expect("add again");

        assert_eq!(load(&log).expect("load").len(), 1);
    }

    #[test]
    fn reorder_writes_the_whole_order() {
        let tmp = TempDir::new("ws-reorder");
        let log = tmp.path().join("projects.log");
        for name in ["one", "two", "three"] {
            add(&log, &tmp.dir(name)).expect("add");
        }

        reorder(&log, &tmp.path().join("three"), 0).expect("reorder");

        let names: Vec<String> = load(&log)
            .expect("load")
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, vec!["three", "one", "two"]);
    }

    #[test]
    fn reorder_clamps_an_index_past_the_end() {
        let tmp = TempDir::new("ws-reorder-clamp");
        let log = tmp.path().join("projects.log");
        for name in ["one", "two"] {
            add(&log, &tmp.dir(name)).expect("add");
        }

        reorder(&log, &tmp.path().join("one"), 99).expect("reorder");

        let names: Vec<String> = load(&log)
            .expect("load")
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, vec!["two", "one"]);
    }

    #[test]
    fn an_order_line_naming_an_unknown_path_keeps_the_rest() {
        let (_tmp, log, dir) = seeded("ws-order-unknown", &["add\t/kept", "order\t/gone\t/kept"]);

        let projects = load(&log).expect("load");
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].path, PathBuf::from("/kept"));
        assert!(dir.is_dir());
    }

    #[test]
    fn a_damaged_line_costs_only_itself() {
        let (_tmp, log, _dir) = seeded(
            "ws-damaged",
            &[
                "add\t/one",
                "this line is nonsense",
                "rename\tonly-one-field",
                "add\t/two",
            ],
        );

        let paths: Vec<PathBuf> = load(&log)
            .expect("load")
            .into_iter()
            .map(|entry| entry.path)
            .collect();
        assert_eq!(paths, vec![PathBuf::from("/one"), PathBuf::from("/two")]);
    }

    #[test]
    fn a_path_containing_a_tab_survives_the_log() {
        let tmp = TempDir::new("ws-tab");
        let log = tmp.path().join("projects.log");
        let odd = tmp.dir("weird\tname");
        add(&log, &odd).expect("add");

        assert_eq!(load(&log).expect("load")[0].path, odd);
    }

    #[test]
    fn create_makes_the_directory() {
        let tmp = TempDir::new("ws-create");
        let made = create(tmp.path(), "new-project").expect("create");

        assert_eq!(made, tmp.path().join("new-project"));
        assert!(made.is_dir());
    }

    #[test]
    fn create_refuses_names_that_are_not_one_component() {
        let tmp = TempDir::new("ws-create-bad");
        for name in [
            "",
            ".",
            "..",
            "a/b",
            "../escape",
            "/etc/passwd",
            "nested/../x",
        ] {
            assert!(
                matches!(create(tmp.path(), name), Err(Error::BadName(_))),
                "{name:?} should have been refused",
            );
        }
        // Nothing was created anywhere.
        assert_eq!(
            fs::read_dir(tmp.path()).expect("read the temp dir").count(),
            0
        );
    }

    #[test]
    fn create_never_overwrites() {
        let tmp = TempDir::new("ws-create-exists");
        let existing = tmp.dir("taken");
        fs::write(existing.join("keep.txt"), "important").expect("seed a file");

        assert!(matches!(create(tmp.path(), "taken"), Err(Error::Exists(_))));
        assert!(
            existing.join("keep.txt").is_file(),
            "the directory was clobbered"
        );
    }

    #[test]
    fn create_does_not_build_missing_parents() {
        let tmp = TempDir::new("ws-create-noparents");
        let missing = tmp.path().join("not-there");

        assert!(matches!(create(&missing, "child"), Err(Error::Io(_))));
        assert!(!missing.exists(), "create_dir_all behaviour leaked in");
    }

    #[test]
    fn a_project_name_keeps_cjk() {
        let (_tmp, log, dir) = seeded("ws-cjk", &[]);
        add(&log, &dir).expect("add");
        rename(&log, &dir, "修复 k2k-rust 未知表路由").expect("rename");

        assert_eq!(
            load(&log).expect("load")[0].name,
            "修复 k2k-rust 未知表路由"
        );
    }

    #[test]
    fn a_missing_log_reads_as_an_empty_list() {
        let tmp = TempDir::new("ws-missing");
        assert!(load(&tmp.path().join("nope.log")).expect("load").is_empty());
    }

    #[test]
    fn a_branch_comes_out_of_head() {
        let tmp = TempDir::new("ws-branch");
        tmp.file("repo/.git/HEAD", "ref: refs/heads/main\n");
        assert_eq!(branch(&tmp.path().join("repo")).as_deref(), Some("main"));
    }

    #[test]
    fn a_branch_name_may_contain_slashes() {
        let tmp = TempDir::new("ws-branch-slash");
        tmp.file("repo/.git/HEAD", "ref: refs/heads/feat/k2k-routing\n");
        assert_eq!(
            branch(&tmp.path().join("repo")).as_deref(),
            Some("feat/k2k-routing")
        );
    }

    #[test]
    fn a_detached_head_reports_a_short_commit() {
        let tmp = TempDir::new("ws-branch-detached");
        tmp.file(
            "repo/.git/HEAD",
            "9f2c1ab4d5e6f708192a3b4c5d6e7f8091a2b3c4\n",
        );
        assert_eq!(branch(&tmp.path().join("repo")).as_deref(), Some("9f2c1ab"));
    }

    #[test]
    fn a_worktree_pointer_is_followed() {
        let tmp = TempDir::new("ws-branch-worktree");
        // The real git directory lives elsewhere and `.git` only records where.
        tmp.file("repo/.git", "gitdir: /somewhere/else/.git/worktrees/repo\n");
        assert_eq!(
            branch(&tmp.path().join("repo")),
            None,
            "an absent target is not a branch"
        );

        let real = tmp.dir("real");
        fs::write(real.join("HEAD"), "ref: refs/heads/work\n").expect("write HEAD");
        fs::write(
            tmp.path().join("repo/.git"),
            format!("gitdir: {}\n", real.display()),
        )
        .expect("point");
        assert_eq!(branch(&tmp.path().join("repo")).as_deref(), Some("work"));
    }

    #[test]
    fn a_directory_that_is_not_a_checkout_has_no_branch() {
        let tmp = TempDir::new("ws-branch-none");
        assert_eq!(branch(&tmp.dir("plain")), None);
        assert_eq!(branch(&tmp.path().join("never-existed")), None);
    }
}
