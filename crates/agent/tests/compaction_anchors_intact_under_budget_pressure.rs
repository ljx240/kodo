//! P1: `compact_history` must preserve the leading system block and
//! the trailing tool-group-aligned tail under a 12-round middle block.
//! The post-check `verify_anchors` is the runtime guard, so the test
//! drives the full pass and asserts both the byte-identical system
//! block / tail and the `anchors_ok` flag.
//!
//! Note: this test exercises `kodo_agent::compaction`. The plan placed
//! it under `crates/memory/tests/`, but the memory crate cannot
//! depend on kodo-agent (that would be a cycle — agent already
//! depends on memory), so the test lives in the agent crate's
//! integration tests. The behaviour it pins is the same.

use kodo_agent::compaction::{compact_history, verify_anchors, CompactConfig};
use kodo_agent::provider::{self, ContentBlock, MessageRole, Provider, ProviderMessage};
use std::path::Path;

fn write_fake_summary_script(dir: &Path, body: &str) -> std::path::PathBuf {
    let script = dir.join("compaction_anchors_script.jsonl");
    std::fs::write(
        &script,
        format!("{}\n", serde_json::json!({ "text": body })),
    )
    .expect("write fake script");
    script
}

fn build_history(n_middle: usize, n_tail: usize) -> Vec<ProviderMessage> {
    let mut history = vec![
        ProviderMessage::system("sys-rules"),
        ProviderMessage::system("sys-policies"),
    ];
    for i in 0..n_middle {
        // Twelve user / tool / assistant triples would be exactly 36
        // messages, so the cut must land inside the middle without
        // breaking any tool call group. The simplest shape that
        // exercises the boundary finder is a plain user message per
        // round; the boundary logic only looks at tool-result
        // pairing, so this still validates the keep_tail/keep system
        // invariants.
        history.push(ProviderMessage::user(format!("middle user #{i}")));
        let calls = vec![provider::NativeToolCall {
            id: format!("c{i}"),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": format!("src/lib{i}.rs")}),
        }];
        history.push(ProviderMessage::assistant("", calls));
        history.push(ProviderMessage::tool_result(
            format!("c{i}"),
            format!("body of lib{i}.rs"),
            false,
        ));
    }
    for i in 0..n_tail {
        history.push(ProviderMessage::user(format!("tail user #{i}")));
        history.push(ProviderMessage::assistant(
            format!("tail assistant #{i}"),
            Vec::new(),
        ));
    }
    history
}

#[test]
fn anchors_intact_after_full_compaction_pass() {
    let dir = std::env::temp_dir().join(format!("kodo-compact-anchors-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = write_fake_summary_script(&dir, "Concise summary.");
    let provider = Provider::new("fake", "fake-key", script.to_string_lossy(), "fake-model");
    let cfg = CompactConfig::default();

    // 12 middle triples + 6 tail messages + 2 system messages.
    let mut history = build_history(12, 3);
    // 3 tail messages = 3*2 = 6 keep_tail messages.
    let pre_system_end = 2;
    let pre_keep_tail = 6;

    let report = compact_history(&mut history, &cfg, &provider, &|| true)
        .expect("compact_history must succeed");

    // The report's anchors_ok is the runtime's own check; the explicit
    // call below is the post-check the agent loop runs at lib.rs:2590.
    assert!(report.anchors_ok, "anchors_ok must be true on success");

    // Leading system block preserved verbatim.
    assert_eq!(history[0].role, MessageRole::System);
    assert_eq!(history[1].role, MessageRole::System);
    if let (ContentBlock::Text { text: a }, ContentBlock::Text { text: b }) =
        (&history[0].content[0], &history[1].content[0])
    {
        assert_eq!(a, "sys-rules");
        assert_eq!(b, "sys-policies");
    } else {
        panic!("system block should be plain text");
    }

    // Trailing tool-group-aligned tail preserved verbatim and in order.
    let tail_offset = history.len() - pre_keep_tail;
    for (i, expected) in (0..3)
        .flat_map(|i| {
            [
                (i, format!("tail user #{i}")),
                (i, format!("tail assistant #{i}")),
            ]
        })
        .enumerate()
    {
        let msg = &history[tail_offset + i];
        let ContentBlock::Text { text } = &msg.content[0] else {
            panic!("tail[{i}] should be plain text");
        };
        assert_eq!(text, &expected.1, "tail[{i}] mismatch");
    }

    // The post-check function agrees.
    assert!(verify_anchors(&history, pre_system_end, pre_keep_tail));

    let _ = std::fs::remove_dir_all(&dir);
}
