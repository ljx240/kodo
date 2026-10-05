//! Free public-web search via DuckDuckGo's HTML endpoint.
//!
//! The point of this module is "tool capability without a paid API key": the
//! model can look something up before answering. DDG's HTML endpoint is the
//! only free search backend that's stable enough to parse; the JS endpoint
//! requires a token and the lite endpoint rotates user-agents. We hand-roll
//! the HTML scan instead of pulling in `scraper`/`html5ever` to stay at zero
//! new dependencies.
//!
//! Surfacing a "搜索失败" note in human summaries is acceptable; surfacing a
//! crash on a CAPTCHAnot test would not be. The `anomaly-modal` check on the
//! raw HTML is the only place we hard-fail — everything else degrades to
//! "no results".

use std::time::Duration;

use ureq::{Agent, AgentBuilder, Proxy};

/// One parsed DuckDuckGo HTML result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Run a search and return up to `max_results` hits. Errors are user-readable
/// Chinese strings suitable for the agent's note line.
pub fn search(query: &str, max_results: u32) -> Result<Vec<SearchHit>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let cap = max_results.clamp(1, 20);
    let url = build_url(trimmed);
    let html = fetch(&url)?;
    if html.contains("anomaly-modal") {
        return Err("搜索触发频率限制，请稍后再试".into());
    }
    if html.contains("result__no_results") {
        return Ok(Vec::new());
    }
    parse_results(&html, cap)
}

/// Build the DDG HTML endpoint URL with the query percent-encoded (spaces
/// become `+` per the endpoint's expectation).
fn build_url(query: &str) -> String {
    let encoded = percent_encode(query);
    format!("https://html.duckduckgo.com/html/?q={encoded}")
}

/// Hand-rolled percent-encoder for the query component. Reserved characters
/// in RFC 3986 query strings are encoded; everything else (UTF-8, `-_.~`)
/// passes through unchanged. We keep it conservative so DDG's parser doesn't
/// surprise us with `&`/`=`/`#` in the query being interpreted as URL syntax.
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b'\''
            | b'('
            | b')'
            | b'!'
            | b'*' => out.push(byte as char),
            b' ' => out.push('+'),
            _ => {
                // Two ASCII hex digits — the byte is one octet.
                out.push('%');
                out.push(hex_digit(byte >> 4));
                out.push(hex_digit(byte & 0x0f));
            }
        }
    }
    out
}

fn hex_digit(nibble: u8) -> char {
    match nibble & 0x0f {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + nibble - 10) as char,
        _ => '0',
    }
}

/// Fetch with conservative timeouts and optional proxy. We mirror the same
/// env-var set the shell uses for `run_command` (`HTTP_PROXY` / `HTTPS_PROXY`)
/// so a user with a corporate proxy gets DDG through it without extra config.
fn fetch(url: &str) -> Result<String, String> {
    let mut builder = AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(15))
        .timeout_write(Duration::from_secs(10));
    builder = match proxy_from_env() {
        Ok(Some(proxy)) => builder.proxy(proxy),
        Ok(None) => builder,
        Err(error) => {
            // A bad proxy URL should not poison the whole tool; the user just
            // has to unset it.
            eprintln!("websearch: ignoring bad proxy env: {error}");
            builder
        }
    };
    let agent: Agent = builder.build();
    agent
        .get(url)
        .set("User-Agent", "Mozilla/5.0 (Kodo) AppleWebKit/537.36")
        .set("Accept-Language", "en-US,en;q=0.9,zh-CN;q=0.8")
        .call()
        .map_err(|error| format!("请求失败：{error}"))?
        .into_string()
        .map_err(|error| format!("响应解析失败：{error}"))
}

fn proxy_from_env() -> Result<Option<Proxy>, String> {
    for key in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            return Proxy::new(value)
                .map(Some)
                .map_err(|error| format!("{key}: {error}"));
        }
    }
    Ok(None)
}

/// Walk the HTML and pull out (title, url, snippet) tuples. The DDG HTML
/// result block looks roughly like:
///
/// ```html
/// <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2F...">Title</a>
/// <a class="result__snippet" href="...">snippet text…</a>
/// ```
///
/// Real URLs are inside the `uddg=` query param; we strip the DDG redirect
/// wrapper because the model only cares about the canonical destination.
fn parse_results(html: &str, max: u32) -> Result<Vec<SearchHit>, String> {
    let mut hits = Vec::new();
    let mut cursor = 0usize;
    while let Some((href, open_end, close)) = find_result_a(html, cursor) {
        let title = extract_anchor_text(&html[open_end..close]);
        let snippet =
            find_next_snippet(&html[close..]).unwrap_or_default();
        hits.push(SearchHit {
            title: collapse_ws(&title),
            url: canonicalize_url(&href),
            snippet: collapse_ws(&snippet),
        });
        cursor = close + "</a>".len();
        if hits.len() as u32 >= max {
            break;
        }
    }
    Ok(hits)
}

/// Find the next `<a class="result__a" …>…</a>` and return
/// `(href_value, byte_after_open_tag_gt, byte_at_close_tag_lt)`.
fn find_result_a(html: &str, from: usize) -> Option<(String, usize, usize)> {
    let needle = "<a class=\"result__a\"";
    let needle_alt = "<a class='result__a'";
    let pos = match html[from..].find(needle) {
        Some(p) => from + p,
        None => html[from..].find(needle_alt).map(|p| from + p)?,
    };
    // The opening tag spans `pos .. open_end`. We need to pull `href="…"`
    // out of it before scanning for the closing `</a>`.
    let open_end_rel = html[pos..].find('>')?;
    let open_end = pos + open_end_rel;
    let href = extract_attr(&html[pos..open_end], "href").unwrap_or_default();
    let close = html[open_end..].find("</a>")? + open_end;
    Some((href, open_end + 1, close))
}

/// Pull `<a …>TEXT</a>` text out of a slice that starts immediately after the
/// opening `<a …>` tag. Stops at the first `</a>`.
fn extract_anchor_text(slice: &str) -> String {
    let end = slice.find("</a>").unwrap_or(slice.len());
    slice[..end].to_string()
}

/// Pull the value of an HTML attribute out of a tag opening like
/// `<a class="…" href="…">`. Handles double- and single-quoted values, and
/// unquoted values up to the next whitespace. Returns `None` if the attribute
/// is missing.
fn extract_attr(tag_open: &str, name: &str) -> Option<String> {
    let needle_dq = format!("{name}=\"");
    let needle_sq = format!("{name}='");
    if let Some(start) = tag_open.find(&needle_dq) {
        let after = start + needle_dq.len();
        let rest = &tag_open[after..];
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    if let Some(start) = tag_open.find(&needle_sq) {
        let after = start + needle_sq.len();
        let rest = &tag_open[after..];
        let end = rest.find('\'')?;
        return Some(rest[..end].to_string());
    }
    None
}

/// Find the visible text of the next `<a class="result__snippet">…</a>` in
/// `slice`, returning the cleaned text. Returns an empty string when none
/// exists so the result row still gets a title/url.
fn find_next_snippet(slice: &str) -> Option<String> {
    let needle = "<a class=\"result__snippet\"";
    let pos = slice.find(needle)?;
    let open_end_rel = slice[pos..].find('>')?;
    let open_end = pos + open_end_rel + 1;
    let close_rel = slice[open_end..].find("</a>")?;
    Some(slice[open_end..open_end + close_rel].to_string())
}

/// Collapse runs of whitespace into single spaces (DDG results carry
/// newlines from inline `<span>`s inside the title/snippet).
fn collapse_ws(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_space = false;
    for ch in input.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// Strip the DDG redirect wrapper and return the canonical URL. The wrapper
/// is `//duckduckgo.com/l/?uddg=<encoded>`; we keep the encoded value as-is
/// because the model will display it to the user (no point double-decoding).
fn canonicalize_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("//duckduckgo.com/l/?uddg=") {
        let end = rest.find('&').unwrap_or(rest.len());
        return rest[..end].to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("https://duckduckgo.com/l/?uddg=") {
        let end = rest.find('&').unwrap_or(rest.len());
        return rest[..end].to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("http://duckduckgo.com/l/?uddg=") {
        let end = rest.find('&').unwrap_or(rest.len());
        return rest[..end].to_string();
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encode_preserves_unreserved_and_encodes_spaces() {
        assert_eq!(percent_encode("hello world"), "hello+world");
        assert_eq!(percent_encode("kodo-agent"), "kodo-agent");
        // Hex digits are lowercase per the Rust convention; DDG accepts both.
        assert_eq!(percent_encode("a&b=c#d"), "a%26b%3dc%23d");
        // UTF-8 bytes are encoded per octet (hex digits lowercase).
        assert_eq!(percent_encode("搜索"), "%e6%90%9c%e7%b4%a2");
    }

    #[test]
    fn canonicalize_url_strips_ddg_redirect_wrapper() {
        assert_eq!(
            canonicalize_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpath&kl=us"),
            "https%3A%2F%2Fexample.com%2Fpath"
        );
        assert_eq!(canonicalize_url("https://example.com/"), "https://example.com/");
    }

    #[test]
    fn parse_results_extracts_three_rows_from_fixture() {
        let html = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/ddg_sample.html"),
        )
        .expect("ddg_sample.html fixture must exist");
        let hits = parse_results(&html, 8).expect("parser must not error");
        assert_eq!(hits.len(), 3, "fixture carries three result rows");
        let first = &hits[0];
        assert_eq!(first.url, "https%3A%2F%2Ftauri.app%2F");
        assert!(
            first.title.contains("Tauri"),
            "first title must include the brand, got {:?}",
            first.title
        );
        assert!(!first.snippet.is_empty());
        // The "result__no_results" marker is in the fixture; the parser only
        // honours it from the live HTML path, but we still want to assert that
        // the fixture file carries it (regression guard for the live code).
        assert!(html.contains("result__no_results"));
    }

    #[test]
    fn parse_results_respects_max_results_cap() {
        let html = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/ddg_sample.html"),
        )
        .expect("ddg_sample.html fixture must exist");
        assert_eq!(parse_results(&html, 1).unwrap().len(), 1);
        assert_eq!(parse_results(&html, 2).unwrap().len(), 2);
        // The cap is clamped at 1 — a 0/negative request still gets the first
        // row, matching the `clamp(1, 20)` guard in `search()`.
        assert_eq!(parse_results(&html, 0).unwrap().len(), 1);
    }
}
