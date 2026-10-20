//! The CLI's consent channel for the decision component.
//!
//! The CLI shows the same preview the desktop shows — the real bytes, the
//! destination, the model — and waits for an explicit answer. A machine
//! that has no terminal is never asked: it refuses, so a piped run cannot
//! silently send data.

use std::io::{self, BufRead, IsTerminal, Write};

use kodo_shell::decision::{DecisionConsent, DecisionConsentAnswer, DecisionPreview};

/// The CLI's consent host.
pub struct CliDecisionConsent;

impl DecisionConsent for CliDecisionConsent {
    fn request(&self, preview: &DecisionPreview) -> DecisionConsentAnswer {
        if !io::stdin().is_terminal() {
            // No terminal, no human, no request. The reason is printed so
            // the run's log says why nothing was sent.
            let mut stderr = io::stderr();
            let _ = writeln!(
                stderr,
                "· 非交互输入，已跳过 Jev 决策请求（{}）",
                preview.scenario.label()
            );
            let _ = stderr.flush();
            return DecisionConsentAnswer::Skip;
        }
        let mut stderr = io::stderr();
        let _ = write!(stderr, "{}", prompt_block(preview));
        let _ = stderr.flush();
        let mut line = String::new();
        let mut input = io::stdin().lock();
        match input.read_line(&mut line) {
            // EOF or a read error: refuse. Never guess.
            Ok(0) | Err(_) => DecisionConsentAnswer::Skip,
            Ok(_) => match line.trim() {
                "y" | "Y" => DecisionConsentAnswer::Allow,
                _ => DecisionConsentAnswer::Skip,
            },
        }
    }
}

/// The preview block, as it should appear on stderr.
pub fn prompt_block(preview: &DecisionPreview) -> String {
    format!(
        "────────────────────────────────────────\n\
         需要批准 · Jev 决策辅助（{purpose}）\n\
         \x20 接收方：{origin}{path}\n\
         \x20 模型：{model}\n\
         \x20 模式：{mode} · 策略：{policy}\n\
         \x20 请求大小：{bytes} 字节 · 有效期：{ttl} 秒\n\
         \x20 完整发送内容：\n{body}\n\
         \x20 问题与答案空间：\n{questions}\n\
         \x20 [y] 允许本次发送   [n] 跳过 Jev\n\
         > ",
        purpose = preview.purpose,
        origin = preview.origin,
        path = preview.path,
        model = preview.model,
        mode = preview.mode.label(),
        policy = preview.policy_version,
        bytes = preview.request_bytes,
        ttl = preview.expires_in_seconds,
        body = preview.request_body,
        questions = preview.questions,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kodo_agent::decision::{DecisionMode, DecisionScenario};

    fn preview() -> DecisionPreview {
        DecisionPreview {
            request_id: "jev-1".to_owned(),
            scenario: DecisionScenario::TaskRoute,
            mode: DecisionMode::Observe,
            origin: "https://api.typesafe.ai".to_owned(),
            path: "/v1/systemone".to_owned(),
            model: "jev-1.13.0".to_owned(),
            policy_version: "kodo-jev-v1".to_owned(),
            purpose: "判断任务类别".to_owned(),
            request_body: "{\"model\":\"jev-1.13.0\"}".to_owned(),
            questions: "{\"task_type\":{}}".to_owned(),
            request_bytes: 22,
            state_bytes: 10,
            expires_in_seconds: 300,
        }
    }

    #[test]
    fn the_block_shows_the_real_content() {
        let block = prompt_block(&preview());
        assert!(
            block.contains("https://api.typesafe.ai/v1/systemone"),
            "{block}"
        );
        assert!(block.contains("jev-1.13.0"), "{block}");
        assert!(block.contains("{\"model\":\"jev-1.13.0\"}"), "{block}");
        assert!(block.contains("task_type"), "{block}");
        assert!(block.contains("允许本次发送"), "{block}");
    }
}
