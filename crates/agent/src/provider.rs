//! HTTP chat providers. Keys never leave this process except toward the user's
//! own configured endpoint.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Provider {
    pub template: String,
    pub api_key: String,
    pub endpoint: String,
    pub model: String,
}

/// Feature flags a provider exposes. Fields always exist; unsupported
/// capabilities are simply `false` so the agent loop can branch safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub text_chat: bool,
    /// Native OpenAI/Anthropic tool-calling API (reserved for full wiring).
    pub native_tools: bool,
    pub streaming: bool,
    pub reasoning: bool,
}

impl ProviderCapabilities {
    pub fn text_only() -> Self {
        Self {
            text_chat: true,
            native_tools: false,
            streaming: false,
            reasoning: false,
        }
    }
}

impl Provider {
    /// Capability set for this provider template / model family.
    pub fn capabilities(&self) -> ProviderCapabilities {
        match self.template.as_str() {
            "anthropic" => ProviderCapabilities {
                text_chat: true,
                native_tools: true,
                streaming: false,
                reasoning: true,
            },
            // OpenAI-compatible endpoints vary; reserve native_tools as false
            // until per-endpoint detection lands. JSON fallback still works.
            _ => ProviderCapabilities {
                text_chat: true,
                native_tools: false,
                streaming: false,
                reasoning: false,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// A tool call returned via the provider's **native** tool API.
/// `arguments` is only valid at this boundary — the agent converts it to
/// typed [`crate::protocol::ToolCall`] immediately.
#[derive(Debug, Clone)]
pub struct NativeToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Empty when the model only returned text (JSON fallback lives in `text`).
    pub native_tool_calls: Vec<NativeToolCall>,
}

pub fn chat(
    provider: &Provider,
    messages: &[ChatMessage],
    max_tokens: u32,
) -> Result<ChatResponse, String> {
    let max_tokens = max_tokens.clamp(256, 8192);
    match provider.template.as_str() {
        "anthropic" => anthropic(provider, messages, max_tokens),
        // Offline deterministic provider for benchmarks / CI. `endpoint` is a
        // local JSONL path — one scripted ChatResponse line per model turn.
        "fake" => fake_chat(provider, messages),
        _ => openai_compatible(provider, messages, max_tokens),
    }
}

/// Scripted, fully offline replies. Index = number of assistant messages already
/// in history, so the same script + same loop always produce the same turns.
fn fake_chat(provider: &Provider, messages: &[ChatMessage]) -> Result<ChatResponse, String> {
    let path = provider.endpoint.trim();
    if path.is_empty() {
        return Err("fake provider requires endpoint=<local script.jsonl path>".to_owned());
    }
    let raw = std::fs::read_to_string(path).map_err(|e| format!("fake script read {path}: {e}"))?;
    let lines: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if lines.is_empty() {
        return Err(format!("fake script is empty: {path}"));
    }
    let turn = messages.iter().filter(|m| m.role == "assistant").count();
    let line = lines.get(turn).ok_or_else(|| {
        format!(
            "fake script exhausted at turn {turn} ({} lines)",
            lines.len()
        )
    })?;
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|e| format!("fake script line {turn}: {e}"))?;
    let text = value
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    let native_tool_calls = value
        .get("native_tool_calls")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .enumerate()
                .map(|(i, c)| NativeToolCall {
                    id: c
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("fake_{i}")),
                    name: c
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned(),
                    arguments: c
                        .get("arguments")
                        .cloned()
                        .unwrap_or(serde_json::Value::Object(Default::default())),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ChatResponse {
        text,
        input_tokens: 16,
        output_tokens: 16,
        native_tool_calls,
    })
}

fn openai_compatible(
    provider: &Provider,
    messages: &[ChatMessage],
    max_tokens: u32,
) -> Result<ChatResponse, String> {
    let base = if provider.endpoint.trim().is_empty() {
        default_openai_base(&provider.template)
    } else {
        provider.endpoint.trim().trim_end_matches('/').to_owned()
    };
    let url = format!("{base}/chat/completions");

    let payload = OpenAiRequest {
        model: provider.model.clone(),
        messages: messages
            .iter()
            .map(|m| OpenAiMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect(),
        temperature: 0.2,
        max_tokens,
    };

    let response = ureq::post(&url)
        .timeout(std::time::Duration::from_secs(60))
        .set(
            "Authorization",
            &format!("Bearer {}", provider.api_key.trim()),
        )
        .set("Content-Type", "application/json")
        .send_json(serde_json::to_value(&payload).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let body: OpenAiResponse = response.into_json().map_err(|e| e.to_string())?;
    let choice = body.choices.into_iter().next();
    let text = choice
        .as_ref()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default();
    let native_tool_calls = choice
        .as_ref()
        .and_then(|c| c.message.tool_calls.as_ref())
        .map(|calls| {
            calls
                .iter()
                .map(|call| NativeToolCall {
                    id: call.id.clone(),
                    name: call.function.name.clone(),
                    arguments: parse_openai_arguments(&call.function.arguments),
                })
                .collect()
        })
        .unwrap_or_default();
    let usage = body.usage.unwrap_or(OpenAiUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
    });
    Ok(ChatResponse {
        text,
        input_tokens: usage.prompt_tokens,
        output_tokens: usage.completion_tokens,
        native_tool_calls,
    })
}

/// OpenAI ships arguments as a JSON string; tolerate empty/invalid payloads.
fn parse_openai_arguments(raw: &str) -> serde_json::Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return serde_json::Value::Object(Default::default());
    }
    serde_json::from_str(trimmed).unwrap_or(serde_json::Value::String(trimmed.to_owned()))
}

fn anthropic(
    provider: &Provider,
    messages: &[ChatMessage],
    max_tokens: u32,
) -> Result<ChatResponse, String> {
    let base = if provider.endpoint.trim().is_empty() {
        "https://api.anthropic.com".to_owned()
    } else {
        provider.endpoint.trim().trim_end_matches('/').to_owned()
    };
    let url = format!("{base}/v1/messages");

    let system = messages
        .iter()
        .find(|m| m.role == "system")
        .map(|m| m.content.clone())
        .unwrap_or_default();
    let rest: Vec<AnthropicMessage> = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| AnthropicMessage {
            role: if m.role == "assistant" {
                "assistant"
            } else {
                "user"
            }
            .to_owned(),
            content: m.content.clone(),
        })
        .collect();

    let payload = AnthropicRequest {
        model: provider.model.clone(),
        max_tokens,
        system,
        messages: rest,
    };

    let response = ureq::post(&url)
        .timeout(std::time::Duration::from_secs(60))
        .set("x-api-key", provider.api_key.trim())
        .set("anthropic-version", "2023-06-01")
        .set("Content-Type", "application/json")
        .send_json(serde_json::to_value(&payload).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let body: AnthropicResponse = response.into_json().map_err(|e| e.to_string())?;
    let mut text_parts: Vec<String> = Vec::new();
    let mut native_tool_calls: Vec<NativeToolCall> = Vec::new();
    let mut tool_index = 0usize;
    for block in body.content {
        match block.kind.as_str() {
            "text" => text_parts.push(block.text.unwrap_or_default()),
            "tool_use" => {
                let id = block
                    .id
                    .unwrap_or_else(|| format!("anthropic_{tool_index}"));
                let name = block.name.unwrap_or_default();
                let input = block
                    .input
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                native_tool_calls.push(NativeToolCall {
                    id,
                    name,
                    arguments: input,
                });
                tool_index += 1;
            }
            _ => {}
        }
    }
    Ok(ChatResponse {
        text: text_parts.join("\n"),
        input_tokens: body.usage.as_ref().map(|u| u.input_tokens).unwrap_or(0),
        output_tokens: body.usage.as_ref().map(|u| u.output_tokens).unwrap_or(0),
        native_tool_calls,
    })
}

fn default_openai_base(template: &str) -> String {
    match template {
        "deepseek" => "https://api.deepseek.com/v1".to_owned(),
        _ => "https://api.openai.com/v1".to_owned(),
    }
}

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiChoiceMessage,
}

#[derive(Deserialize)]
struct OpenAiChoiceMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Deserialize)]
struct OpenAiToolCall {
    #[serde(default)]
    id: String,
    function: OpenAiToolFunction,
}

#[derive(Deserialize)]
struct OpenAiToolFunction {
    name: String,
    #[serde(default)]
    arguments: String,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<AnthropicMessage>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicBlock>,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicBlock {
    #[serde(default, rename = "type")]
    kind: String,
    text: Option<String>,
    id: Option<String>,
    name: Option<String>,
    input: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_always_expose_all_four_flags() {
        let anthropic = Provider {
            template: "anthropic".into(),
            api_key: "k".into(),
            endpoint: String::new(),
            model: "claude".into(),
        };
        let caps = anthropic.capabilities();
        assert!(caps.text_chat);
        assert!(caps.native_tools);
        assert!(!caps.streaming);
        assert!(caps.reasoning);

        let openai = Provider {
            template: "openai".into(),
            api_key: "k".into(),
            endpoint: String::new(),
            model: "gpt".into(),
        };
        let caps = openai.capabilities();
        assert!(caps.text_chat);
        assert!(!caps.native_tools);
        assert!(!caps.streaming);
        assert!(!caps.reasoning);
    }

    #[test]
    fn openai_arguments_string_parses_to_object() {
        let value = parse_openai_arguments(r#"{"path":"a.txt"}"#);
        assert_eq!(value["path"], "a.txt");
        let empty = parse_openai_arguments("");
        assert!(empty.as_object().unwrap().is_empty());
        let broken = parse_openai_arguments("not-json");
        assert_eq!(broken, serde_json::Value::String("not-json".into()));
    }
}
