//! The tab-separated line codec shared by `workspace` and `session`.
//!
//! Both logs are append-only text where a record is a verb followed by fields.
//! A field may hold anything the user typed — CJK, quotes, tab, newline — so
//! each field is escaped on the way out and unescaped on the way back in.
//!
//! The order matters, and it is *split first, unescape second*. Unescaping can
//! synthesize a real delimiter: a field containing a literal tab is stored as
//! the two characters `\` `t`, and unescaping that before splitting would turn
//! it into a real tab and cut one field into two. Splitting on the raw, still
//! escaped boundaries first avoids that entirely.
//!
//! Unescaping is a single left-to-right pass, never chained `str::replace`
//! calls: `replace("\\n", …)` followed by `replace("\\\\", …)` would unescape
//! twice and turn a literal `\n` in the data into a newline.
//!
//! Known bound, deliberately not defended against in code: a field must not
//! contain a NUL byte.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// Escapes one field so it contains no tab, newline or carriage return.
pub fn escape(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    for ch in field.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// Reverses [`escape`] in one left-to-right pass.
///
/// An unknown escape such as `\x` is kept literally (both characters), so it
/// round-trips. A trailing lone `\` — which [`escape`] never produces — is kept
/// as a lone `\` rather than dropped.
pub fn unescape(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Splits a raw line into unescaped fields: cut on real tabs first, then
/// reverse the escaping of each piece.
pub fn split(line: &str) -> Vec<String> {
    line.split('\t').map(unescape).collect()
}

/// Joins already-raw fields into one escaped line body (no trailing newline).
pub fn join(fields: &[&str]) -> String {
    fields
        .iter()
        .map(|field| escape(field))
        .collect::<Vec<_>>()
        .join("\t")
}

/// Appends one line, creating the parent directory when it is missing.
pub fn append(path: &Path, body: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{body}")
}

/// Every line of the log, in order.
///
/// A log that does not exist yet is an empty log, not an error — that is what
/// "nothing has been recorded" means for append-only state.
pub fn read_lines(path: &Path) -> io::Result<Vec<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text.lines().map(str::to_owned).collect()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(field: &str) {
        let line = join(&["verb", field, "tail"]);
        let parsed = split(&line);
        assert_eq!(
            parsed,
            vec!["verb".to_owned(), field.to_owned(), "tail".to_owned()],
            "line was {line:?}"
        );
        assert_eq!(parsed.len(), 3, "a field was cut in two: {line:?}");
    }

    #[test]
    fn a_tab_inside_a_field_does_not_split_it() {
        // This is the bug the ordering exists to prevent: unescaping first would
        // turn this value's `\t` into a real tab and produce four fields.
        round_trip("before\tafter");
    }

    #[test]
    fn a_newline_inside_a_field_survives() {
        round_trip("line one\nline two\r\nline three");
    }

    #[test]
    fn a_backslash_before_a_t_is_not_a_tab() {
        // `\` then `t` must come back as those two characters, not as a tab.
        round_trip("\\t");
        assert_eq!(unescape("\\\\t"), "\\t");
    }

    #[test]
    fn chained_replacement_would_have_double_unescaped() {
        // A literal `\n` (backslash, n) must stay two characters.
        assert_eq!(unescape(&escape("\\n")), "\\n");
        assert_eq!(
            unescape("\\n").len(),
            1,
            "a real newline is produced for the escape"
        );
    }

    #[test]
    fn cjk_and_quotes_pass_through() {
        round_trip("修复 k2k-rust 未知表路由");
        round_trip("he said \"hi\" and 'bye'");
        round_trip("");
    }

    #[test]
    fn an_unknown_escape_round_trips() {
        assert_eq!(unescape("\\x"), "\\x");
        assert_eq!(unescape(&escape("\\x")), "\\x");
    }

    #[test]
    fn a_trailing_lone_backslash_is_kept() {
        assert_eq!(unescape("abc\\"), "abc\\");
    }

    #[test]
    fn split_of_an_empty_line_is_one_empty_field() {
        assert_eq!(split(""), vec![String::new()]);
    }
}
