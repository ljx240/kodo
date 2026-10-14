use std::collections::BTreeSet;

use crate::error::{KnowledgeError, KnowledgeResult};
pub const MAX_TEXT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_IMPORT_BYTES: usize = MAX_TEXT_BYTES;
pub(crate) const MAX_IMPORT_CHARS: usize = 128 * 1024;
pub(crate) const MAX_SOURCE_CHARS: usize = 4 * 1024;
pub(crate) const MAX_CHUNK_CHARS: usize = 2_048;
pub(crate) const MAX_CHUNKS_PER_DOCUMENT: usize = 1_024;
pub(crate) const MAX_QUERY_BYTES: usize = 2_048;
pub(crate) const MAX_QUERY_TERMS: usize = 64;

#[derive(Debug, Clone)]
pub(crate) struct TextChunk {
    pub heading: String,
    pub line_start: usize,
    pub line_end: usize,
    pub body: String,
    pub search_terms: String,
}

pub(crate) fn validate_text_body(body: &str) -> KnowledgeResult<()> {
    if body.trim().is_empty()
        || body.len() > MAX_IMPORT_BYTES
        || body.chars().count() > MAX_IMPORT_CHARS
    {
        return Err(KnowledgeError::InvalidInput);
    }
    if contains_secret_material(body) {
        return Err(KnowledgeError::SensitiveContent);
    }
    Ok(())
}

pub(crate) fn contains_secret_material(body: &str) -> bool {
    if kodo_core::redact::redact_secrets(body) != body {
        return true;
    }

    let lower = body.to_ascii_lowercase();
    if lower.contains("•••") || lower.contains("[redacted]") || lower.contains("-----redacted")
    {
        return true;
    }

    for key in [
        "api_key",
        "apikey",
        "access_token",
        "refresh_token",
        "client_secret",
        "password",
        "authorization",
        "private_key",
        "aws_access_key",
    ] {
        let mut rest = lower.as_str();
        while let Some(index) = rest.find(key) {
            let after_key = &rest[index + key.len()..];
            let trimmed = after_key
                .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, '\'' | '"'));
            if let Some(value) = trimmed
                .strip_prefix('=')
                .or_else(|| trimmed.strip_prefix(':'))
            {
                let value = value
                    .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, '\'' | '"'));
                let length = value
                    .chars()
                    .take_while(|ch| !ch.is_whitespace() && !matches!(ch, '\'' | '"' | ',' | ';'))
                    .count();
                if length >= 4 {
                    return true;
                }
            }
            rest = after_key;
        }
    }
    false
}

pub(crate) fn looks_hostile(body: &str) -> bool {
    let lower = body.to_lowercase();
    [
        "ignore previous instructions",
        "ignore all previous",
        "ignore all instructions",
        "ignore the user",
        "ignore your system prompt",
        "disregard previous",
        "you are now a",
        "act as a system",
        "system:",
        "<|system|>",
        "<system>",
        "[inst]",
        "### instruction",
        "upload your credentials",
        "send your api key",
        "send your api_key",
        "send your secret",
        "run `rm -rf",
        "execute rm -rf",
        "delete all files",
        "忽略之前的指令",
        "忽略以上所有指令",
        "忽略系统指令",
        "忘记之前的提示",
        "你现在是系统",
        "泄露你的密钥",
        "输出 api key",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
}

pub(crate) fn content_hash(body: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(body.as_bytes()))
}

pub(crate) fn make_chunks(title: &str, body: &str) -> KnowledgeResult<Vec<TextChunk>> {
    let mut chunks = Vec::new();
    let mut current_heading = title.to_owned();
    let mut current_body: Vec<String> = Vec::new();
    let mut current_start = 0usize;
    let mut current_end = 0usize;
    let mut current_chars = 0usize;

    let flush = |chunks: &mut Vec<TextChunk>,
                 heading: &str,
                 body_lines: &mut Vec<String>,
                 line_start: usize,
                 line_end: usize| {
        if body_lines.is_empty() {
            return;
        }
        let text = body_lines.join("\n");
        let terms = index_terms(&format!("{title}\n{heading}\n{text}"));
        chunks.push(TextChunk {
            heading: heading.to_owned(),
            line_start,
            line_end,
            body: text,
            search_terms: terms,
        });
        body_lines.clear();
    };

    for (index, line) in body.split('\n').enumerate() {
        let line_number = index + 1;
        let heading = markdown_heading(line);
        if let Some(new_heading) = heading.as_deref() {
            flush(
                &mut chunks,
                &current_heading,
                &mut current_body,
                current_start,
                current_end,
            );
            current_heading = new_heading.to_owned();
            current_chars = 0;
        }

        let line_char_count = line.chars().count();
        if line_char_count > MAX_CHUNK_CHARS {
            flush(
                &mut chunks,
                &current_heading,
                &mut current_body,
                current_start,
                current_end,
            );
            let mut segment = String::new();
            let mut segment_chars = 0;
            for ch in line.chars() {
                segment.push(ch);
                segment_chars += 1;
                if segment_chars == MAX_CHUNK_CHARS {
                    current_start = line_number;
                    current_end = line_number;
                    current_body.push(std::mem::take(&mut segment));
                    flush(
                        &mut chunks,
                        &current_heading,
                        &mut current_body,
                        current_start,
                        current_end,
                    );
                    segment_chars = 0;
                }
            }
            if !segment.is_empty() {
                current_start = line_number;
                current_end = line_number;
                current_body.push(segment);
                flush(
                    &mut chunks,
                    &current_heading,
                    &mut current_body,
                    current_start,
                    current_end,
                );
            }
            continue;
        }

        let additional_chars = line_char_count + usize::from(!current_body.is_empty());
        if !current_body.is_empty()
            && (current_body.len() >= 80 || current_chars + additional_chars > MAX_CHUNK_CHARS)
        {
            flush(
                &mut chunks,
                &current_heading,
                &mut current_body,
                current_start,
                current_end,
            );
            current_chars = 0;
        }

        if current_body.is_empty() {
            current_start = line_number;
            current_chars = line_char_count;
        } else {
            current_chars += additional_chars;
        }
        current_end = line_number;
        current_body.push(line.to_owned());
    }

    flush(
        &mut chunks,
        &current_heading,
        &mut current_body,
        current_start,
        current_end,
    );
    if chunks.is_empty() || chunks.len() > MAX_CHUNKS_PER_DOCUMENT {
        return Err(KnowledgeError::InvalidInput);
    }
    Ok(chunks)
}

pub(crate) fn query_terms(text: &str) -> KnowledgeResult<Vec<String>> {
    if text.trim().is_empty() || text.len() > MAX_QUERY_BYTES {
        return Err(KnowledgeError::InvalidInput);
    }
    let terms = tokenize(text, true);
    if terms.is_empty() || terms.len() > MAX_QUERY_TERMS {
        return Err(KnowledgeError::InvalidInput);
    }
    Ok(terms)
}

fn index_terms(text: &str) -> String {
    tokenize(text, false)
        .into_iter()
        .collect::<Vec<_>>()
        .join(" ")
}

fn tokenize(text: &str, query: bool) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut terms = BTreeSet::new();
    let mut index = 0;
    while index < chars.len() {
        if is_cjk(chars[index]) {
            let start = index;
            while index < chars.len() && is_cjk(chars[index]) {
                index += 1;
            }
            let run = &chars[start..index];
            if run.len() == 1 {
                terms.insert(run[0].to_string());
            } else {
                let whole: String = run.iter().collect();
                if whole.chars().count() <= 16 {
                    terms.insert(whole);
                }
                if !query {
                    for ch in run {
                        terms.insert(ch.to_string());
                    }
                }
                for width in 2..=3.min(run.len()) {
                    for window in run.windows(width) {
                        terms.insert(window.iter().collect());
                    }
                }
            }
        } else if chars[index].is_alphanumeric() || matches!(chars[index], '_' | '-') {
            let start = index;
            while index < chars.len()
                && (chars[index].is_alphanumeric() || matches!(chars[index], '_' | '-'))
            {
                index += 1;
            }
            let token: String = chars[start..index]
                .iter()
                .flat_map(|ch| ch.to_lowercase())
                .collect();
            if token.chars().count() <= 128 {
                terms.insert(token.clone());
            }
            for part in token.split(['_', '-']).filter(|part| !part.is_empty()) {
                terms.insert(part.to_owned());
            }
        } else {
            index += 1;
        }
        // Query preprocessing is bounded independently from document
        // indexing. Indexing must cover every term in a bounded chunk so a
        // relevant identifier or phrase near its end remains retrievable.
        if query && terms.len() > MAX_QUERY_TERMS * 4 {
            break;
        }
    }
    if query {
        terms.into_iter().take(MAX_QUERY_TERMS + 1).collect()
    } else {
        terms.into_iter().collect()
    }
}

fn markdown_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let hashes = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let heading = rest.trim().trim_end_matches('#').trim();
    (!heading.is_empty()).then(|| heading.to_owned())
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3400..=0x4DBF |
        0x4E00..=0x9FFF |
        0xF900..=0xFAFF |
        0x20000..=0x2FA1F)
}
