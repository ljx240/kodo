//! Scripted Work runtime contracts. These results are not live model quality.
use kodo_agent::{AgentMode, Permission, Provider, RunRequest, SinkEvent, Step};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;

#[derive(Deserialize)]
struct Case {
    id: String,
    split: String,
    source: String,
    path: String,
    body: Option<String>,
    contains: String,
    #[serde(default)]
    existing: Option<String>,
    #[serde(default)]
    corrupt: bool,
    accept: bool,
}

pub(super) fn run(report: &mut super::EvalReport) {
    let cases: Vec<Case> = serde_json::from_str(include_str!("../work-fixtures/cases.json"))
        .expect("Work fixture manifest");
    assert_eq!(cases.len(), 30);
    let root = std::env::temp_dir().join(format!("kodo-work-contracts-{}", std::process::id()));
    fs::create_dir_all(&root).expect("fixture root");
    let mut splits = [(0, 0, 0), (0, 0, 0)];
    for case in cases {
        let outcome = (|| -> Result<bool, String> {
            let project = root.join(&case.id);
            fs::create_dir_all(&project).map_err(|e| e.to_string())?;
            fs::write(project.join("source.txt"), &case.source).map_err(|e| e.to_string())?;
            if let Some(existing) = &case.existing {
                kodo_agent::tools::write_project_file(&project, &case.path, existing)?;
            }
            if case.corrupt {
                fs::write(project.join(&case.path), "broken office archive")
                    .map_err(|e| e.to_string())?;
            }
            let mut turns = vec![
                json!({"text": json!({"tool_calls": [{"name":"read_file","args":{"path":"source.txt"}}]}).to_string()}),
            ];
            if let Some(body) = &case.body {
                turns.push(json!({"text": json!({"tool_calls": [{"name":"write_file","args":{"path":case.path,"content":body}}]}).to_string()}));
                turns.push(json!({"text": json!({"tool_calls": [{"name":"read_file","args":{"path":case.path}}]}).to_string()}));
            }
            for _ in 0..16 {
                turns.push(json!({"text":"已完成请求，结果已保存。"}));
            }
            let script = root.join(format!("{}.jsonl", case.id));
            fs::write(
                &script,
                turns
                    .iter()
                    .map(Value::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n",
            )
            .map_err(|e| e.to_string())?;
            let request = RunRequest {
                project: Some(project.clone()),
                message: format!("请根据 source.txt 的资料生成文件，保存到 `{}`", case.path),
                conversation_history: Vec::new(),
                pinned_context: Vec::new(),
                pinned_force_inline: Vec::new(),
                provider: Some(Provider::new(
                    "fake",
                    "synthetic",
                    script.to_string_lossy(),
                    "fixture",
                )),
                permission: Permission::Full,
                mode: AgentMode::Work,
                fallback_to_local: false,
                max_output_tokens: 1024,
                extended_thinking: false,
                local_trace: false,
                session_id: None,
                user_skills_dir: None,
                mcp_servers: Vec::new(),
                agent_instructions: None,
                personas: Vec::new(),
                budget_family: None,
                memory: None,
                knowledge_context: None,
                allow_verify_commands: false,
                decision: None,
            };
            let mut ready = false;
            let mut sink = |event| {
                if let SinkEvent::Finished {
                    step: Step::AgentMessage { delivery, .. },
                    ..
                } = event
                {
                    ready = delivery == "ready";
                }
                true
            };
            let result = kodo_agent::run(&request, &|| true, &|_, _| true, &mut sink);
            let intact = fs::read_to_string(project.join("source.txt"))
                .map_err(|e| e.to_string())?
                == case.source;
            let oracle = if case.accept {
                let full = kodo_agent::tools::resolve_in_project(&project, &case.path)?;
                let content = if kodo_agent::office::is_docx(&case.path) {
                    kodo_agent::office::read_docx(&full, 100_000)?.text
                } else {
                    fs::read_to_string(full).map_err(|e| e.to_string())?
                };
                ready && content.contains(&case.contains)
            } else {
                !ready
            };
            Ok(intact && oracle && (result.is_ok() || !case.accept))
        })();
        let passed = outcome.as_ref().is_ok_and(|v| *v);
        report.check(
            &format!("work_{}", case.id),
            passed,
            &format!("scripted Work contract failed: {outcome:?}"),
        );
        let index = usize::from(case.split == "holdout");
        let split = &mut splits[index];
        split.0 += 1;
        split.1 += usize::from(passed);
        split.2 += usize::from(!case.accept && !passed);
    }
    for (label, (count, passed, false_completion)) in
        ["development", "holdout"].into_iter().zip(splits)
    {
        println!("Work scripted {label}: {passed}/{count} contracts; false_completion_rejections_failed={false_completion}; live quality NOT RUN");
    }
    fs::remove_dir_all(root).expect("remove owned fixture root");
}
