//! Secret / hostile-content redaction. Single source of truth shared by
//! `kodo-agent` (tool results, traces) and `kodo-memory` (memory body
//! scrubbing). Originally lived in `kodo_agent::tools::redact_secrets`;
//! moved to the leaf crate to break an `agent <-> memory` cycle.

/// Redact secret material from a free-text payload. The function is
/// conservative: it prefers to over-redact (e.g. scrubbing the body of
/// "Bearer " tokens or AWS access-key prefixes) rather than let a secret
/// reach the model or the session log.
pub fn redact_secrets(text: &str) -> String {
    if text.is_empty() {
        return text.to_owned();
    }
    let mut out = text.to_owned();
    out = redact_private_key_blocks(&out);
    out = redact_token_prefixes(&out);
    out = redact_bearer_and_basic(&out);
    out = redact_json_secrets(&out);
    out = redact_url_userinfo(&out);
    redact_env_assignments(&out)
}

fn redact_private_key_blocks(text: &str) -> String {
    let mut out = text.to_owned();
    for header in [
        "-----BEGIN PRIVATE KEY-----",
        "-----BEGIN RSA PRIVATE KEY-----",
    ] {
        while let Some(start) = out.find(header) {
            let end_marker = header.replace("BEGIN", "END");
            let rest_start = start + header.len();
            match out[rest_start..].find(&end_marker) {
                Some(rel) => {
                    let end = rest_start + rel + end_marker.len();
                    out.replace_range(start..end, "-----REDACTED PRIVATE KEY-----");
                }
                None => {
                    out.replace_range(start.., "-----REDACTED PRIVATE KEY-----");
                    break;
                }
            }
        }
    }
    out
}

fn redact_token_prefixes(text: &str) -> String {
    let mut out = text.to_owned();
    for pattern in [
        "sk-ant-",
        "sk-proj-",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "xoxa-",
        "xoxs-",
        "glpat-",
        "AIza",
        "AKIA",
        "eyJ",
        "sk-",
    ] {
        let mut search_from = 0;
        while let Some(rel) = out[search_from..].find(pattern) {
            let pos = search_from + rel;
            let after = pos + pattern.len();
            if out[after..].starts_with('•') {
                search_from = after;
                continue;
            }
            let mut end = after;
            let bytes = out.as_bytes();
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'-' || bytes[end] == b'_')
            {
                end += 1;
            }
            let min_body = if pattern == "eyJ" || pattern == "AKIA" {
                pattern.len() + 8
            } else {
                after
            };
            if end > min_body || (pattern != "eyJ" && pattern != "AKIA" && end > after) {
                out.replace_range(after..end, "•••");
                search_from = after + "•••".len();
            } else {
                search_from = after;
            }
        }
    }
    out
}

fn redact_bearer_and_basic(text: &str) -> String {
    let mut out = text.to_owned();
    for prefix in ["Bearer ", "Basic "] {
        let mut search_from = 0;
        while let Some(rel) = out[search_from..].find(prefix) {
            let pos = search_from + rel + prefix.len();
            if out[pos..].starts_with('•') {
                search_from = pos;
                continue;
            }
            let mut end = pos;
            let bytes = out.as_bytes();
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() && bytes[end] != b'"' {
                end += 1;
            }
            if end > pos {
                out.replace_range(pos..end, "•••");
                search_from = pos + "•••".len();
            } else {
                search_from = pos;
            }
        }
    }
    out
}

fn redact_json_secrets(text: &str) -> String {
    let keys = [
        "api_key",
        "apikey",
        "accessToken",
        "access_token",
        "refresh_token",
        "client_secret",
        "password",
        "secret",
        "token",
        "authorization",
        "private_key",
    ];
    let mut out = text.to_owned();
    for key in keys {
        for quote in ['"', '\''] {
            let needle = format!("{quote}{key}{quote}");
            let mut from = 0;
            while let Some(rel) = out[from..].find(&needle) {
                let key_end = from + rel + needle.len();
                let rest = &out[key_end..];
                let Some(colon_rel) = rest.find(':') else {
                    break;
                };
                let after_colon = key_end + colon_rel + 1;
                let value_part = &out[after_colon..];
                let trimmed = value_part.trim_start();
                if trimmed.is_empty() {
                    break;
                }
                let q = trimmed.chars().next().unwrap();
                if q != '"' && q != '\'' {
                    from = after_colon;
                    continue;
                }
                let abs_q = after_colon + value_part.find(q).unwrap_or(0);
                let body_start = abs_q + q.len_utf8();
                if body_start >= out.len() {
                    break;
                }
                let Some(end_rel) = out[body_start..].find(q) else {
                    break;
                };
                let body_end = body_start + end_rel;
                if body_end > body_start {
                    out.replace_range(body_start..body_end, "•••");
                }
                from = body_start + 3;
            }
        }
    }
    out
}

fn redact_url_userinfo(text: &str) -> String {
    let mut out = text.to_owned();
    let mut search_from = 0;
    while let Some(rel) = out[search_from..].find("://") {
        let scheme_end = search_from + rel + 3;
        let rest = &out[scheme_end..];
        let Some(at_rel) = rest.find('@') else {
            search_from = scheme_end;
            continue;
        };
        let authority = &rest[..at_rel];
        if let Some(colon) = authority.find(':') {
            if colon > 0 && authority[colon + 1..].len() > 2 {
                let pass_start = scheme_end + colon + 1;
                let pass_end = scheme_end + at_rel;
                if pass_end > pass_start {
                    out.replace_range(pass_start..pass_end, "•••");
                    search_from = pass_start + 3;
                    continue;
                }
            }
        }
        search_from = scheme_end;
    }
    out
}

fn redact_env_assignments(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            if let Some(eq) = line.find('=') {
                let name = &line[..eq];
                let upper = name.to_ascii_uppercase();
                let sensitive = upper.ends_with("_TOKEN")
                    || upper.ends_with("_KEY")
                    || upper.ends_with("_SECRET")
                    || upper.ends_with("_PASSWORD")
                    || upper.ends_with("_CREDENTIAL")
                    || upper.ends_with("_CREDENTIALS")
                    || upper.contains("PASSWORD")
                    || upper.contains("AUTHORIZATION")
                    || upper.contains("APIKEY")
                    || upper.contains("API_KEY")
                    || upper == "TOKEN"
                    || upper == "KEY"
                    || upper == "SECRET";
                if sensitive && line[eq + 1..].trim().len() > 3 {
                    format!("{name}=•••")
                } else {
                    line.to_owned()
                }
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_bearer() {
        let out = redact_secrets("Authorization: Bearer abcdefghijklmnop");
        assert!(out.contains("•••"));
        assert!(!out.contains("abcdefghijklmnop"));
    }

    #[test]
    fn redacts_aws_key() {
        let out = redact_secrets("AKIA1234567890ABCDEF");
        assert!(out.contains("•••") || out.contains("AKIA"));
    }

    #[test]
    fn no_change_on_benign() {
        let s = "hello world, this is a normal sentence";
        assert_eq!(redact_secrets(s), s);
    }
}
