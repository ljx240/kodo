//! Local settings, stored as an append-only `key=value` log.
//!
//! Writing appends a line; reading scans the file and takes the last value of a
//! key. Nothing is ever rewritten or truncated, so no locking and no atomic
//! replace is needed — the same shape as the append-only event log the design
//! doc asks for. It also keeps the core free of a JSON parser: `split_once('=')`
//! is complete for a scalar setting, while a hand-written JSON reader would be
//! an implementation that only pretends to work.
//!
//! Known bounds, deliberately not defended against in code: a value must not
//! contain a newline, and a value may contain `=` (the split is on the first).
//!
//! The last-value-wins semantics here are right for *settings* and wrong for an
//! ordered event log, so `workspace` and `session` do not reuse [`read`] — they
//! fold their own lines in order.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const SETTINGS_FILE: &str = "settings.log";
const CREDENTIALS_FILE: &str = "credentials.log";
const HEADER: &str = "# Kodo settings. Append-only: the last value of a key wins.";

/// Where the core keeps its state: `~/Library/Application Support/Kodo` on
/// macOS, `~/.config/kodo` elsewhere.
///
/// Deliberately not Tauri's `app_data_dir()`: that derives the folder from the
/// bundle identifier and would give `dev.kodo.desktop`, which does not match the
/// `~/Library/Application Support/Kodo` the settings screen already shows.
pub fn config_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support").join("Kodo")
    } else {
        home.join(".config").join("kodo")
    }
}

/// The current user's settings file, or `None` when `HOME` is unset.
pub fn settings_path() -> Option<PathBuf> {
    Some(config_dir(&home()?).join(SETTINGS_FILE))
}

/// API keys live apart from ordinary settings so a shared `settings.log` never
/// contains them.
pub fn credentials_path() -> Option<PathBuf> {
    Some(config_dir(&home()?).join(CREDENTIALS_FILE))
}

/// The settings key one provider's secret is stored under.
pub fn credential_key(provider_id: &str) -> String {
    format!("provider.{provider_id}")
}

/// Last stored secret for `provider_id`, if any.
pub fn read_credential(path: &Path, provider_id: &str) -> Option<String> {
    read(path, &credential_key(provider_id))
}

/// Appends one provider secret and locks the file down on Unix.
pub fn write_credential(path: &Path, provider_id: &str, secret: &str) -> io::Result<()> {
    append(path, &credential_key(provider_id), secret)?;
    restrict(path);
    Ok(())
}

/// Drops every stored secret for the given providers (others stay).
pub fn clear_credentials(path: &Path, keep: &[(String, String)]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut body = String::from("# Kodo credentials. Append-only. Not shared with settings.log.\n");
    for (id, secret) in keep {
        body.push_str(&credential_key(id));
        body.push('=');
        body.push_str(secret);
        body.push('\n');
    }
    fs::write(path, body)?;
    restrict(path);
    Ok(())
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_path: &Path) {}

/// The directory the core keeps its state in, or `None` when `HOME` is unset.
pub fn state_dir() -> Option<PathBuf> {
    Some(config_dir(&home()?))
}

/// The last value written for `key`.
pub fn read(path: &Path, key: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()?
        .lines()
        .filter_map(parse_line)
        .filter(|(candidate, _)| *candidate == key)
        .map(|(_, value)| value.to_owned())
        .last()
}

/// Appends `key=value`, creating the config directory when it is missing.
pub fn append(path: &Path, key: &str, value: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let fresh = !path.exists();
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if fresh {
        writeln!(file, "{HEADER}")?;
    }
    writeln!(file, "{key}={value}")
}

/// `key=value`, or `None` for blank lines, `#` comments and anything without a
/// separator. Both sides are trimmed; the value keeps any further `=`.
fn parse_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    (!key.is_empty()).then(|| (key, value.trim()))
}

/// `$HOME`, or `None` when it is unset or empty.
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    const KEY: &str = "example";

    #[test]
    fn parse_line_skips_blank_comments_and_junk() {
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line("   "), None);
        assert_eq!(parse_line("# a comment"), None);
        assert_eq!(parse_line("no separator here"), None);
        assert_eq!(parse_line("=  value"), None);
        assert_eq!(parse_line("  key = value  "), Some(("key", "value")));
    }

    #[test]
    fn read_returns_the_last_value() {
        let tmp = TempDir::new("settings-last");
        let path = tmp.file("settings.log", "example=/first\nexample=/second\n");
        assert_eq!(read(&path, KEY).as_deref(), Some("/second"));
    }

    #[test]
    fn read_keeps_equals_signs_in_the_value() {
        let tmp = TempDir::new("settings-equals");
        let path = tmp.file("settings.log", "example=/a=b\n");
        assert_eq!(read(&path, KEY).as_deref(), Some("/a=b"));
    }

    #[test]
    fn read_of_a_missing_file_is_none() {
        let tmp = TempDir::new("settings-missing");
        assert_eq!(read(&tmp.path().join("nope.log"), KEY), None);
    }

    #[test]
    fn append_creates_the_parent_directory_and_a_header() {
        let tmp = TempDir::new("settings-parent");
        let path = tmp.path().join("nested").join("deeper").join("settings.log");
        append(&path, KEY, "/tmp/ws").expect("append should create the parents");

        assert_eq!(read(&path, KEY).as_deref(), Some("/tmp/ws"));
        let text = fs::read_to_string(&path).expect("the file should exist");
        assert!(text.starts_with(HEADER), "unexpected first line: {text:?}");
    }

    #[test]
    fn append_only_adds_lines() {
        let tmp = TempDir::new("settings-append");
        let path = tmp.path().join("settings.log");
        append(&path, KEY, "/first").expect("first append");
        append(&path, KEY, "/second").expect("second append");

        let text = fs::read_to_string(&path).expect("the file should exist");
        assert_eq!(text.matches("example=").count(), 2, "the file was rewritten: {text:?}");
        assert_eq!(text.matches(HEADER).count(), 1, "the header was written twice: {text:?}");
        assert_eq!(read(&path, KEY).as_deref(), Some("/second"));
    }

    #[test]
    fn credentials_are_stored_apart_and_restored() {
        let tmp = TempDir::new("settings-creds");
        let path = tmp.path().join("credentials.log");
        write_credential(&path, "p1", "sk-secret").expect("write");
        write_credential(&path, "p2", "sk-other").expect("write");
        assert_eq!(read_credential(&path, "p1").as_deref(), Some("sk-secret"));

        clear_credentials(&path, &[("p2".to_owned(), "sk-other".to_owned())]).expect("clear");
        assert_eq!(read_credential(&path, "p1"), None);
        assert_eq!(read_credential(&path, "p2").as_deref(), Some("sk-other"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "credentials must not be world-readable");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn config_dir_uses_the_macos_layout() {
        assert_eq!(
            config_dir(Path::new("/Users/me")),
            PathBuf::from("/Users/me/Library/Application Support/Kodo"),
        );
    }
}
