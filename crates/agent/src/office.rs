//! Spreadsheet (xlsx / xls / ods) text extraction for pinned attachments.
//!
//! Attachment bodies must reach the model on every arm (DESIGN.md §11), and
//! spreadsheets are binary — `ContextManager::read_range` would refuse them.
//! This module turns workbook cells into TSV text capped at the caller's
//! character budget, stopping on row boundaries so a cut never lands inside a
//! cell. Extraction failure is a `Err` the caller degrades to a path
//! descriptor; sending never fails because of an attachment.

use calamine::Reader;
use std::io::{Read, Write};
use std::path::Path;

pub fn is_docx(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("docx"))
}

pub struct DocumentText {
    pub text: String,
    pub truncated: bool,
}

/// Read document.xml directly; never extract archive paths or resolve entities.
pub fn read_docx(path: &Path, max_chars: usize) -> Result<DocumentText, String> {
    use quick_xml::events::Event;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
        return Err("DOCX exceeds 64 MiB".into());
    }
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    if archive.len() > 4096 {
        return Err("too many DOCX archive entries".into());
    }
    let entry = archive
        .by_name("word/document.xml")
        .map_err(|e| e.to_string())?;
    const XML_LIMIT: u64 = 8 * 1024 * 1024;
    if entry.size() > XML_LIMIT {
        return Err("DOCX XML exceeds 8 MiB".into());
    }
    let mut xml = String::new();
    entry
        .take(XML_LIMIT + 1)
        .read_to_string(&mut xml)
        .map_err(|e| e.to_string())?;
    if xml.len() as u64 > XML_LIMIT {
        return Err("DOCX XML limit exceeded".into());
    }
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut text = String::new();
    let mut chars = 0;
    let mut truncated = false;
    let mut in_text = false;
    let mut document = false;
    let mut depth = 0usize;
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        let piece = match event {
            Event::Start(tag) => {
                depth += 1;
                if tag.local_name().as_ref() == "document" {
                    document = true;
                }
                if tag.local_name().as_ref() == "t" {
                    in_text = true;
                }
                String::new()
            }
            Event::End(tag) => {
                depth = depth.checked_sub(1).ok_or("unbalanced DOCX XML")?;
                match tag.local_name().as_ref() {
                    "t" => {
                        in_text = false;
                        String::new()
                    }
                    "p" | "tr" => "\n".into(),
                    "tc" => "\t".into(),
                    _ => String::new(),
                }
            }
            Event::Empty(tag) => match tag.local_name().as_ref() {
                "tab" => "\t".into(),
                "br" | "cr" => "\n".into(),
                _ => String::new(),
            },
            Event::Text(value) if in_text => value.xml10_content().into_owned(),
            Event::CData(value) if in_text => value.xml10_content().into_owned(),
            Event::GeneralRef(value) if in_text => {
                quick_xml::escape::unescape(&format!("&{};", value.as_ref()))
                    .map_err(|e| e.to_string())?
                    .into_owned()
            }
            Event::DocType(_) => return Err("DOCX entities/DOCTYPE are not supported".into()),
            Event::Eof => break,
            _ => String::new(),
        };
        let length = piece.chars().count();
        let remaining = max_chars.saturating_sub(chars);
        if length > remaining {
            truncated = true;
        }
        text.extend(piece.chars().take(remaining));
        chars += length.min(remaining);
    }
    if depth != 0 {
        return Err("incomplete DOCX XML".into());
    }
    if !document {
        return Err("DOCX document element is missing".into());
    }
    Ok(DocumentText { text, truncated })
}

/// Plain text/Markdown headings and pipe tables become a minimal OOXML document.
pub fn write_docx(path: &Path, content: &str) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("DOCX content is empty".into());
    }
    if content.len() > 2 * 1024 * 1024
        || content
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err("DOCX content is too large or contains invalid XML characters".into());
    }
    let paragraph = |line: &str| {
        let heading = line.chars().take_while(|c| *c == '#').count().min(6);
        let (style, body) = if heading > 0 && line.as_bytes().get(heading) == Some(&b' ') {
            (
                format!("<w:pPr><w:pStyle w:val=\"Heading{heading}\"/></w:pPr>"),
                line[heading + 1..].trim(),
            )
        } else {
            (String::new(), line)
        };
        format!(
            "<w:p>{style}<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
            quick_xml::escape::escape(body)
        )
    };
    let mut body = String::new();
    let mut table = false;
    for line in content.lines() {
        if line.trim().starts_with('|') && line.trim().ends_with('|') {
            let cells: Vec<&str> = line
                .trim()
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect();
            if cells
                .iter()
                .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
            {
                continue;
            }
            if !table {
                body.push_str("<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/></w:tblPr>");
                table = true;
            }
            body.push_str("<w:tr>");
            for cell in cells {
                body.push_str(&format!("<w:tc>{}</w:tc>", paragraph(cell)));
            }
            body.push_str("</w:tr>");
        } else {
            if table {
                body.push_str("</w:tbl>");
                table = false;
            }
            body.push_str(&paragraph(line));
        }
    }
    if table {
        body.push_str("</w:tbl>");
    }
    let document = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr/></w:body></w:document>");
    let styles = format!("<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{}</w:styles>", (1..=6).map(|i| format!("<w:style w:type=\"paragraph\" w:styleId=\"Heading{i}\"><w:name w:val=\"heading {i}\"/><w:rPr><w:b/><w:sz w:val=\"{}\"/></w:rPr></w:style>", 34 - i * 2)).collect::<String>());
    let temporary = path.with_extension(format!(
        "docx-tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    ));
    let result = (|| {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let entries = [
            ("[Content_Types].xml", "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/></Types>"),
            ("_rels/.rels", "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>"),
            ("word/_rels/document.xml.rels", "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>"),
            ("word/document.xml", document.as_str()), ("word/styles.xml", styles.as_str()),
        ];
        for (name, text) in entries {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .map_err(|e| e.to_string())?;
            zip.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        }
        zip.finish()
            .map_err(|e| e.to_string())?
            .sync_all()
            .map_err(|e| e.to_string())?;
        if read_docx(&temporary, 2 * 1024 * 1024)?
            .text
            .trim()
            .is_empty()
        {
            return Err("DOCX readback is empty".into());
        }
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

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
    pub non_empty_cells: usize,
}

/// Reads every sheet of `path` into text, stopping once `max_chars` is hit.
pub fn read_spreadsheet(path: &Path, max_chars: usize) -> Result<SheetText, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
        return Err("spreadsheet exceeds 64 MiB".into());
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xlsx") || e.eq_ignore_ascii_case("ods"))
    {
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        if archive.len() > 4096 {
            return Err("too many spreadsheet archive entries".into());
        }
        let mut total = 0u64;
        for index in 0..archive.len() {
            let entry = archive.by_index(index).map_err(|e| e.to_string())?;
            total = total.saturating_add(entry.size());
            if entry.size() > 8 * 1024 * 1024 || total > 64 * 1024 * 1024 {
                return Err("spreadsheet decompression limit exceeded".into());
            }
        }
    }
    let mut workbook = calamine::open_workbook_auto(path).map_err(|e| e.to_string())?;
    let names = workbook.sheet_names();
    let mut text = String::new();
    let mut truncated = false;
    let sheets = names.len();
    let mut non_empty_cells = 0;
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
            non_empty_cells += row
                .iter()
                .filter(|c| !cell_text(c).trim().is_empty())
                .count();
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
        non_empty_cells,
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
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qa.xlsx")
    }

    #[test]
    fn docx_roundtrip_chinese_entities_headings_and_table() {
        let dir = std::env::temp_dir().join(format!("kodo-docx-roundtrip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("report.docx");
        write_docx(
            &path,
            "# 项目报告\n中文 & <边界>\n| 项目 | 金额 |\n| --- | --- |\n| 收入 | 120 |\n",
        )
        .unwrap();
        let read = read_docx(&path, 1000).unwrap();
        assert!(read.text.contains("项目报告") && read.text.contains("中文 & <边界>"));
        assert!(read.text.contains("收入") && read.text.contains("120"));
        assert!(!read.truncated);
        let clipped = read_docx(&path, 5).unwrap();
        assert_eq!(clipped.text.chars().count(), 5);
        assert!(clipped.truncated);
        assert!(write_docx(&path, " ").is_err());
        assert!(read_docx(&path, 1000).unwrap().text.contains("120"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn docx_rejects_invalid_xml_and_external_entities() {
        let path =
            std::env::temp_dir().join(format!("kodo-docx-invalid-{}.docx", std::process::id()));
        for xml in ["<w:document><w:p>", "<!DOCTYPE x [<!ENTITY x SYSTEM 'file:///unread-secret'>]><document><t>&x;</t></document>", "<wrong/>"] {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(xml.as_bytes()).unwrap();
            zip.finish().unwrap();
            assert!(read_docx(&path, 1000).is_err());
        }
        std::fs::remove_file(path).unwrap();
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
