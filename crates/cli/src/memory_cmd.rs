//! `kodo memory <verb>` — the CLI surface for the Memory Engine.
//!
//! One shared on-disk store at `<state_dir>/memory/v1/memory.sqlite`,
//! same path the desktop uses, so a write from the GUI shows up here and
//! vice versa. Errors are reported on stderr; the exit code is non-zero
//! for storage failures and 0 otherwise (including "not found" cases,
//! which print a helpful note).

use std::path::{Path, PathBuf};

use kodo_core::settings;
use kodo_memory::{
    doctor, model::EmbeddingBackend, model::MemoryFilter, model::MemoryKind, model::MemoryPrivacy,
    model::MemoryQuery, model::MemoryStatus, model_store, retrieval, scope, SqliteMemory,
};

use crate::args::{EmbedVerb, MemoryCmd, MemoryVerb};

pub const USAGE: &str = "\
用法: kodo memory <verb> [选项]

管理 Memory Engine 索引：浏览、查询、停用、修复。GUI 与 CLI 共享同一份本地数据库。

子命令:
  status                          摘要：表行数、待处理 outbox、DB 大小
  list [--kind K] [--status S]    列出当前项目作用域下的记忆（默认含历史）
        [--limit N] [--project P]
  search <query> [--limit N]      关键词检索（BM25 + 精确符号）
  inspect <id>                    显示一条记忆的完整元数据与证据
  disable <id>                    将记忆标记为已停用
  doctor [--apply]                完整性检查；--apply 同时执行 FTS 重建与 outbox 重放
  reindex [--apply]               重建 FTS 索引（默认 dry-run，仅报告）
  embed <status|warmup|off>       管理本地 embedding 缓存（需 --features download）
        [--backend <name>]
        [--from <dir>]              离线 warmup：从本地目录复制 model.onnx + tokenizer.json
  promote <id> [--confirm]        把 procedural 记忆提升为 skill 文件
        [--force]                   默认 dry-run；--confirm 才会写盘
  benchmark                       性能基准（目前仅占位 — 见 P2-3）

通用选项:
  --project <目录>                目标项目根（默认当前目录）
  -h, --help                      显示本帮助
";

pub fn run(cmd: MemoryCmd) -> i32 {
    if cmd.verb.is_none() {
        print!("{USAGE}");
        return 0;
    }
    let project = match resolve_project(cmd.project.as_deref()) {
        Ok(p) => p,
        Err(code) => return code,
    };
    let sqlite = match open_sqlite() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("打开 Memory 存储失败：{e}");
            return 1;
        }
    };
    let store = sqlite.store();
    let sqlite_ref = &sqlite;

    match cmd.verb.unwrap() {
        MemoryVerb::Status => run_status(store),
        MemoryVerb::List => run_list(store, &project, cmd.limit, cmd.kind, cmd.status),
        MemoryVerb::Search => match cmd.value {
            Some(q) => run_search(sqlite_ref, &project, &q, cmd.limit),
            None => {
                eprintln!("search 需要查询字符串");
                2
            }
        },
        MemoryVerb::Inspect => match cmd.value {
            Some(id) => run_inspect(sqlite_ref, &id),
            None => {
                eprintln!("inspect 需要 id");
                2
            }
        },
        MemoryVerb::Disable => match cmd.value {
            Some(id) => run_disable(sqlite_ref, &id),
            None => {
                eprintln!("disable 需要 id");
                2
            }
        },
        MemoryVerb::Doctor => run_doctor(sqlite_ref, cmd.apply),
        MemoryVerb::Reindex => run_doctor(sqlite_ref, cmd.apply),
        MemoryVerb::Embed => run_embed(
            cmd.embed_verb,
            cmd.embed_backend.as_deref(),
            cmd.embed_from.as_deref(),
        ),
        MemoryVerb::Promote => match cmd.value.as_deref() {
            Some(id) => run_promote(store, id, cmd.promote_confirm, cmd.promote_force),
            None => {
                eprintln!("promote 需要 <id>");
                2
            }
        },
        MemoryVerb::Benchmark => {
            print_benchmark_banner();
            0
        }
    }
}

fn resolve_project(input: Option<&Path>) -> Result<PathBuf, i32> {
    let raw = input
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    match std::fs::canonicalize(&raw) {
        Ok(p) if p.is_dir() => Ok(p),
        _ => {
            eprintln!("项目目录无效：{}", raw.display());
            Err(2)
        }
    }
}

fn open_sqlite() -> Result<SqliteMemory, kodo_memory::MemoryError> {
    let state_dir = settings::state_dir()
        .ok_or_else(|| kodo_memory::MemoryError::StorageUnavailable("HOME not set".into()))?;
    let path = state_dir.join("memory").join("v1").join("memory.sqlite");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    SqliteMemory::open(&path)
}

fn run_status(store: &kodo_memory::store::Store) -> i32 {
    match store.row_counts() {
        Ok(counts) => {
            for (table, n) in counts {
                println!("{table:<28} {n}");
            }
            match store.outbox_count_pending() {
                Ok(pending) => println!("outbox pending              {pending}"),
                Err(e) => eprintln!("outbox count failed: {e}"),
            }
            0
        }
        Err(e) => {
            eprintln!("status failed: {e}");
            1
        }
    }
}

fn run_list(
    store: &kodo_memory::store::Store,
    project: &Path,
    limit: Option<usize>,
    kind: Option<String>,
    status: Option<String>,
) -> i32 {
    let project_scope = scope::scope_for(project, None, None);
    let mut filter = MemoryFilter {
        scope: Some(project_scope),
        limit,
        include_history: true,
        ..Default::default()
    };
    if let Some(k) = kind {
        match MemoryKind::parse(&k) {
            Some(parsed) => filter.kind = Some(parsed),
            None => {
                eprintln!("未知 kind：{k}");
                return 2;
            }
        }
    }
    if let Some(s) = status {
        match MemoryStatus::parse(&s) {
            Some(parsed) => filter.status = Some(parsed),
            None => {
                eprintln!("未知 status：{s}");
                return 2;
            }
        }
    }
    match store.list(&filter) {
        Ok(entries) => {
            if entries.is_empty() {
                println!("(no entries)");
                return 0;
            }
            for e in entries {
                println!(
                    "{id:<14} {kind:<12} {status:<10} {authority:<8} {title}",
                    id = e.id,
                    kind = e.kind.as_str(),
                    status = e.status.as_str(),
                    authority = e.authority.as_str(),
                    title = truncate(&e.title, 60),
                );
            }
            0
        }
        Err(e) => {
            eprintln!("list failed: {e}");
            1
        }
    }
}

fn run_search(sqlite: &SqliteMemory, project: &Path, query: &str, limit: Option<usize>) -> i32 {
    let project_scope = scope::scope_for(project, None, None);
    let q = MemoryQuery {
        scope: project_scope,
        text: query.to_owned(),
        related_paths: Vec::new(),
        related_symbols: Vec::new(),
        budget_tokens: 2000,
        require_current_evidence: false,
    };
    match retrieval::recall(sqlite.store(), &q) {
        Ok(report) => {
            if report.pack.snippets.is_empty() {
                println!("(no hits)");
                return 0;
            }
            let limit = limit.unwrap_or(report.pack.snippets.len());
            for s in report.pack.snippets.iter().take(limit) {
                println!(
                    "{id:<14} {kind:<12} {validity:<18} {title}",
                    id = s.id,
                    kind = s.kind.as_str(),
                    validity = s.validity.as_str(),
                    title = truncate(&s.title, 60),
                );
            }
            0
        }
        Err(e) => {
            eprintln!("search failed: {e}");
            1
        }
    }
}

fn run_inspect(sqlite: &SqliteMemory, id: &str) -> i32 {
    match sqlite.store().get_entry(id) {
        Ok(Some(e)) => {
            println!("id            {}", e.id);
            println!("kind          {}", e.kind.as_str());
            println!("status        {}", e.status.as_str());
            println!("authority     {}", e.authority.as_str());
            println!("source_trust  {}", e.source_trust.as_str());
            println!("title         {}", e.title);
            println!("abstract      {}", e.abstract_text);
            println!(
                "scope         project={:?} repo={:?} branch={:?}",
                e.scope.project_id, e.scope.repository_id, e.scope.branch
            );
            println!("confidence    {}", e.confidence);
            println!("use_count     {}", e.use_count);
            println!("version       {}", e.version);
            if !e.source_uris.is_empty() {
                println!("source_uris");
                for u in &e.source_uris {
                    println!("  - {u}");
                }
            }
            println!("body");
            println!("{}", format_body(&e.body));
            0
        }
        Ok(None) => {
            println!("(not found)");
            0
        }
        Err(e) => {
            eprintln!("inspect failed: {e}");
            1
        }
    }
}

fn run_disable(sqlite: &SqliteMemory, id: &str) -> i32 {
    match sqlite.store().disable(id) {
        Ok(()) => {
            println!("disabled {id}");
            0
        }
        Err(e) => {
            eprintln!("disable failed: {e}");
            1
        }
    }
}

fn run_doctor(sqlite: &SqliteMemory, apply: bool) -> i32 {
    let path = sqlite.path();
    match doctor::diagnose(sqlite.store(), path, apply, MemoryPrivacy::High) {
        Ok(report) => {
            println!("ok: {}", report.ok);
            println!("integrity: {:?}", report.integrity);
            println!("tombstones: {}", report.tombstones);
            println!("db_size_bytes: {}", report.db_size_bytes);
            println!("outbox_pending: {}", report.outbox_pending);
            if apply {
                println!("outbox_replayed: {}", report.outbox_replayed);
            }
            println!("row_counts:");
            for (t, n) in report.row_counts {
                println!("  {t:<28} {n}");
            }
            if !report.notes.is_empty() {
                println!("notes:");
                for n in report.notes {
                    println!("  - {n}");
                }
            }
            0
        }
        Err(e) => {
            eprintln!("doctor failed: {e}");
            1
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// Hand-rolled JSON-ish pretty printer that avoids the `serde_json`
/// dep. Bodies are stored as `serde_json::Value` but we only need a
/// readable dump for `inspect`, not a faithful round-trip.
fn format_body(v: &serde_json::Value) -> String {
    format_value(v, 0)
}

fn format_value(v: &serde_json::Value, indent: usize) -> String {
    let pad = " ".repeat(indent);
    match v {
        serde_json::Value::Null => format!("{pad}null"),
        serde_json::Value::Bool(b) => format!("{pad}{b}"),
        serde_json::Value::Number(n) => format!("{pad}{n}"),
        serde_json::Value::String(s) => format!("{pad}{s:?}"),
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                return format!("{pad}[]");
            }
            let mut out = format!("{pad}[\n");
            for (i, item) in arr.iter().enumerate() {
                out.push_str(&format_value(item, indent + 2));
                if i + 1 < arr.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&format!("{pad}]"));
            out
        }
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                return format!("{pad}{{}}");
            }
            let mut out = format!("{pad}{{\n");
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for (i, k) in keys.iter().enumerate() {
                let val = &map[*k];
                out.push_str(&format!("{pad}  {k:?}: "));
                out.push_str(&format_value(val, indent + 2).trim_start());
                if i + 1 < keys.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&format!("{pad}}}"));
            out
        }
    }
}

fn run_embed(verb: Option<EmbedVerb>, backend: Option<&str>, from: Option<&Path>) -> i32 {
    let Some(verb) = verb else {
        eprintln!("embed 需要一个子命令：status | warmup | off");
        return 2;
    };
    let backend = match backend.unwrap_or("local_bge_small_en") {
        raw => match EmbeddingBackend::parse(raw) {
            Some(b) => b,
            None => {
                eprintln!("未知 backend：{raw}（可选：off | local_bge_small_en | local_bge_small_zh | local_minilm_l6 | local_hash）");
                return 2;
            }
        },
    };
    match verb {
        EmbedVerb::Status => match model_store::status(backend) {
            Ok(row) => {
                let cache = model_store::cache_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "<unavailable>".to_owned());
                let total = model_store::cache_size_bytes().unwrap_or(0);
                println!("backend:        {}", backend.as_str());
                println!("model_id:       {}", row.model_id);
                println!("dim:            {}", row.dim);
                println!("ready:          {}", row.is_ready());
                println!(
                    "warmed_at:      {}",
                    row.warmed_at
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "—".to_owned())
                );
                println!(
                    "onnx_sha256:    {}",
                    row.onnx_sha256.as_deref().unwrap_or("—")
                );
                println!(
                    "tokenizer_sha:  {}",
                    row.tokenizer_sha256.as_deref().unwrap_or("—")
                );
                println!(
                    "bytes:          {} (cache total {total})",
                    row.bytes_on_disk
                );
                println!("cache_dir:      {cache}");
                0
            }
            Err(e) => {
                eprintln!("读取 embedding 状态失败：{e}");
                1
            }
        },
        EmbedVerb::Warmup => match model_store::warmup(backend, from) {
            Ok(row) => {
                println!(
                    "warmup ok: backend={} bytes={}",
                    row.backend.as_str(),
                    row.bytes_on_disk
                );
                0
            }
            Err(e) => {
                eprintln!("warmup 失败：{e}");
                1
            }
        },
        EmbedVerb::Off => match model_store::clear(backend) {
            Ok(()) => {
                println!("cleared cache for backend={}", backend.as_str());
                0
            }
            Err(e) => {
                eprintln!("清理失败：{e}");
                1
            }
        },
    }
}

/// `kodo memory promote <id> [--confirm] [--force]`
///
/// Dry-run by default: prints the would-be target path + a 20-line
/// preview of the body. Refuses to write unless `--confirm` is passed.
/// Refuses to overwrite an existing file unless `--force` is also
/// passed. Only `MemoryKind::Procedural` entries are eligible — other
/// kinds are not skills and the CLI refuses up front.
fn run_promote(store: &kodo_memory::store::Store, id: &str, confirm: bool, force: bool) -> i32 {
    let entry = match store.get_entry(id) {
        Ok(Some(e)) => e,
        Ok(None) => {
            eprintln!("未找到记忆 {id}");
            return 1;
        }
        Err(e) => {
            eprintln!("读取失败：{e}");
            return 1;
        }
    };
    if entry.kind != MemoryKind::Procedural {
        eprintln!(
            "promote 仅支持 procedural 记忆；当前条目 kind = {}",
            entry.kind.as_str()
        );
        return 2;
    }
    let Some(state_dir) = settings::state_dir() else {
        eprintln!("无法定位 state_dir；HOME 未设置？");
        return 1;
    };
    let dir = state_dir.join("skills");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("创建 skills 目录失败 {}: {e}", dir.display());
        return 1;
    }
    let path = dir.join(format!("{id}.md"));
    let body_md = format_body_md(&entry);

    if !confirm {
        // Dry-run path: print target + first 20 lines, do not write.
        println!("target: {}", path.display());
        println!("---- preview (dry-run; no file written) ----");
        for (i, line) in body_md.lines().take(20).enumerate() {
            println!("{:>3} | {line}", i + 1);
        }
        if body_md.lines().count() > 20 {
            println!("... ({} lines total)", body_md.lines().count());
        }
        println!("---- re-run with --confirm to write ----");
        return 0;
    }

    if path.exists() && !force {
        eprintln!("{} 已存在；用 --force 覆盖，或换一个 id。", path.display());
        return 2;
    }

    let provenance = format!(
        "<!-- promoted from memory entry {} at {} -->\n",
        id,
        now_iso8601()
    );
    if let Err(e) = std::fs::write(&path, format!("{provenance}{body_md}")) {
        eprintln!("写入失败：{e}");
        return 1;
    }
    println!("wrote {}", path.display());
    0
}

/// Print the `kodo memory benchmark` banner. The benchmark harness is a
/// future P2-3 deliverable (counterfactual replay); for now the CLI
/// prints an honest "not implemented" message instead of fake numbers.
fn print_benchmark_banner() {
    println!(
        "kodo memory benchmark\n\
         ─────────────────────\n\
         Benchmark harness not yet implemented — see docs/memory/EVALUATION.md\n\
         for the methodology and the data we plan to collect.\n\
         \n\
         What you CAN do today:\n\
           kodo memory doctor --apply      # integrity + FTS rebuild + outbox replay\n\
           kodo memory embed warmup        # download a local embedding model\n\
           kodo memory list --kind procedural  # browse candidate skills\n\
         \n\
         This stub exists so the CLI surface is consistent across the P2 verbs.\n\
         The benchmark itself ships when P2-3 (counterfactual replay) lands."
    );
}

fn format_body_md(entry: &kodo_memory::MemoryEntry) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", entry.title));
    if !entry.abstract_text.is_empty() {
        out.push_str(&format!("_{}_\n\n", entry.abstract_text));
    }
    if let Some(md) = entry.body.as_str() {
        out.push_str(md);
    } else {
        // Non-string body: pretty-print as a fenced JSON block so the
        // skill file still has something the agent can read.
        out.push_str("```json\n");
        out.push_str(&format_value(&entry.body, 0));
        out.push_str("\n```\n");
    }
    if !entry.conditions.is_empty() {
        out.push_str("\n## Conditions\n");
        for c in &entry.conditions {
            out.push_str(&format!("- {c}\n"));
        }
    }
    out
}

/// Lightweight ISO-8601 timestamp. We use the system clock directly so
/// the CLI does not pull in a date/time crate; precision is seconds,
/// which is enough for a provenance header.
fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // YYYY-MM-DDTHH:MM:SSZ — derived from epoch seconds via the
    // civil-from-days algorithm (Howard Hinnant). UTC only; the
    // provenance header is a machine-readable stamp, not a date.
    let z = (secs / 86_400) as i64;
    let s = (secs % 86_400) as u64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let s = s % 60;
    let (y, mo, d) = civil_from_days(z);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    // https://howardhinnant.github.io/date_algorithms.html#civil_from_days
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kodo_memory::{
        Authority, MemoryEntry, MemoryKind, MemoryScope, MemoryStatus, SourceTrust, SqliteMemory,
        Store,
    };

    fn fresh_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::Builder::new()
            .prefix("kodo-cli-promote-")
            .tempdir()
            .unwrap();
        let path = dir.path().join("memory.sqlite");
        let mem = SqliteMemory::open(&path).unwrap();
        (dir, mem.store().clone())
    }

    fn procedural_entry(id: &str) -> MemoryEntry {
        let now = kodo_memory::store::now_secs();
        MemoryEntry {
            id: id.to_owned(),
            kind: MemoryKind::Procedural,
            status: MemoryStatus::Active,
            authority: Authority::Historical,
            scope: MemoryScope::default(),
            title: format!("promote-test {id}"),
            abstract_text: "abstract".to_owned(),
            body: serde_json::json!("body markdown line 1\nline 2"),
            content_hash: format!("h-{id}"),
            source_trust: SourceTrust::ToolVerified,
            confidence: 0.7,
            verified: true,
            conditions: vec!["when foo".to_owned()],
            source_uris: vec![],
            valid_from: None,
            valid_until: None,
            last_verified_at: None,
            last_used_at: None,
            use_count: 0,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            version: 1,
            candidate_skill: true,
        }
    }

    fn episodic_entry(id: &str) -> MemoryEntry {
        let mut e = procedural_entry(id);
        e.kind = MemoryKind::Episodic;
        e
    }

    #[test]
    fn promote_dry_run_does_not_write_file() {
        let (_dir, store) = fresh_store();
        let entry = procedural_entry("dry");
        store.upsert_entry(&entry).unwrap();
        let rc = run_promote(&store, "dry", false, false);
        assert_eq!(rc, 0);
        let skills = kodo_core::settings::state_dir()
            .map(|d| d.join("skills"))
            .unwrap_or_default();
        // The file is the user_skills_dir under state_dir. The test does
        // not assert the path content (state_dir may not be writable in
        // CI sandboxes); the dry-run path *must not* write regardless.
        if skills.exists() {
            assert!(
                !skills.join("dry.md").exists(),
                "dry-run must not write the skill file"
            );
        }
    }

    #[test]
    fn promote_refuses_non_procedural_entry() {
        let (_dir, store) = fresh_store();
        let entry = episodic_entry("wrong-kind");
        store.upsert_entry(&entry).unwrap();
        let rc = run_promote(&store, "wrong-kind", true, true);
        assert_eq!(rc, 2, "non-procedural must return 2");
    }

    #[test]
    fn now_iso8601_format_is_stable() {
        // The exact value depends on the wall clock; what we pin is the
        // shape: 4-digit year, 2-digit month/day, Z suffix.
        let s = now_iso8601();
        assert_eq!(s.len(), 20, "got: {s}");
        assert!(s.ends_with('Z'), "got: {s}");
        assert!(s.chars().nth(4) == Some('-'));
        assert!(s.chars().nth(7) == Some('-'));
        assert!(s.chars().nth(10) == Some('T'));
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        // 1970-01-01 is day 0.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2000-01-01 is day 10957.
        assert_eq!(civil_from_days(10957), (2000, 1, 1));
        // 2026-10-13 is day 20739 (56 years incl. 14 leap days = 20453;
        // +286 days into 2026 = 20739).
        assert_eq!(civil_from_days(20739), (2026, 10, 13));
    }
}
