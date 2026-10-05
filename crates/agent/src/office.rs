//! Spreadsheet (xlsx / xls / ods) text extraction for pinned attachments.
//!
//! Attachment bodies must reach the model on every arm (DESIGN.md §11), and
//! spreadsheets are binary — `ContextManager::read_range` would refuse them.
//! This module turns workbook cells into TSV text capped at the caller's
//! character budget, stopping on row boundaries so a cut never lands inside a
//! cell. Extraction failure is a `Err` the caller degrades to a path
//! descriptor; sending never fails because of an attachment.

use calamine::Reader;
use std::path::Path;

/// Extensions treated as spreadsheets (mirrors the TS `OFFICE_EXTS` split).
pub fn is_spreadsheet(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(ext.as_str(), "xlsx" | "xls" | "xlsm" | "xlsb" | "ods")
}

/// Extracted workbook text: one `## sheet: <name>` header per sheet, then
/// rows with cells joined by tabs.
pub struct SheetText {
    pub text: String,
    /// True when a row did not fit the budget — the text is partial.
    pub truncated: bool,
    pub sheets: usize,
}

/// Reads every sheet of `path` into text, stopping once `max_chars` is hit.
pub fn read_spreadsheet(path: &Path, max_chars: usize) -> Result<SheetText, String> {
    let mut workbook = calamine::open_workbook_auto(path).map_err(|e| e.to_string())?;
    let names = workbook.sheet_names();
    let mut text = String::new();
    let mut truncated = false;
    let sheets = names.len();
    for name in &names {
        if truncated {
            break;
        }
        let range = workbook.worksheet_range(name).map_err(|e| e.to_string())?;
        let header = format!("## sheet: {name}\n");
        if text.chars().count() + header.chars().count() > max_chars {
            truncated = true;
            break;
        }
        text.push_str(&header);
        for row in range.rows() {
            let mut line: String = row
                .iter()
                .map(cell_text)
                .collect::<Vec<String>>()
                .join("\t");
            line.push('\n');
            if text.chars().count() + line.chars().count() > max_chars {
                truncated = true;
                break;
            }
            text.push_str(&line);
        }
    }
    Ok(SheetText {
        text,
        truncated,
        sheets,
    })
}

/// One cell → flat text. Tabs/newlines inside a value would break the TSV
/// row structure, so they collapse to spaces.
fn cell_text(cell: &calamine::Data) -> String {
    use calamine::Data;
    let raw = match cell {
        Data::Int(i) => i.to_string(),
        Data::Float(f) => f.to_string(),
        Data::String(s) => s.clone(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(dt) => dt.to_string(),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(_) => "#ERR!".to_owned(),
        Data::Empty => String::new(),
    };
    if raw.contains(['\t', '\n', '\r']) {
        raw.chars()
            .map(|c| match c {
                '\t' | '\n' | '\r' => ' ',
                other => other,
            })
            .collect()
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal but real xlsx: build one with calamine's writer? calamine is
    /// read-only — instead the fixture is a committed file (tests/fixtures).
    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/qa.xlsx")
    }

    #[test]
    fn extracts_xlsx_cells_as_tsv() {
        let sheet = read_spreadsheet(&fixture(), 10_000).expect("fixture reads");
        assert_eq!(sheet.sheets, 2, "fixture has two sheets");
        assert!(
            sheet.text.lines().count() >= 8,
            "two headers + data rows: {}",
            sheet.text
        );
        assert!(!sheet.truncated, "small fixture fits the budget");
        assert!(sheet.text.contains("## sheet: 缴费"), "sheet header");
        assert!(sheet.text.contains("险种"), "cell text");
        assert!(sheet.text.contains("养老保险"), "data row");
        // Row shape: cells joined by tabs, rows by newlines.
        let data_line = sheet
            .text
            .lines()
            .find(|l| l.contains("养老保险"))
            .expect("data line");
        assert!(data_line.contains('\t'), "cells are tab-separated");
    }

    #[test]
    fn truncates_at_budget_on_a_row_boundary() {
        let sheet = read_spreadsheet(&fixture(), 40).expect("fixture reads");
        assert!(sheet.truncated, "budget forces truncation");
        assert!(sheet.text.chars().count() <= 40, "never exceeds the budget");
        assert!(!sheet.text.ends_with('\t'), "cut lands between rows");
    }

    #[test]
    fn corrupt_file_errors_for_descriptor_fallback() {
        let dir = std::env::temp_dir().join(format!("kodo-office-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let bad = dir.join("corrupt.xlsx");
        std::fs::write(&bad, b"this is not a zip archive").expect("write");
        assert!(read_spreadsheet(&bad, 1_000).is_err());
    }

    #[test]
    fn is_spreadsheet_matches_extensions() {
        assert!(is_spreadsheet("/tmp/a.xlsx"));
        assert!(is_spreadsheet("/tmp/a.XLS"));
        assert!(is_spreadsheet("b.ods"));
        assert!(!is_spreadsheet("/tmp/a.docx"));
        assert!(!is_spreadsheet("/tmp/noext"));
    }
}
