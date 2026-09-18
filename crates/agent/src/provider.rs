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

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

pub fn chat(provider: &Provider, messages: &[ChatMessage], max_tokens: u32) -> Result<ChatResponse, String> {
    let max_tokens = max_tokens.clamp(256, 8192);
    match provider.template.as_str() {
        "anthropic" => anthropic(provider, messages, max_tokens),
        _ => openai_compatible(provider, messages, max_tokens),
    }
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
            .map(|m| OpenAiMessage { role: m.role.clone(), content: m.content.clone() })
            .collect(),
        temperature: 0.2,
        max_tokens,
    };

    let response = ureq::post(&url)
        .timeout(std::time::Duration::from_secs(60))
        .set("Authorization", &format!("Bearer {}", provider.api_key.trim()))
        .set("Content-Type", "application/json")
        .send_json(serde_json::to_value(&payload).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let body: OpenAiResponse = response.into_json().map_err(|e| e.to_string())?;
    let text = body
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message.content)
        .unwrap_or_default();
    let usage = body.usage.unwrap_or(OpenAiUsage { prompt_tokens: 0, completion_tokens: 0 });
    Ok(ChatResponse {
        text,
        input_tokens: usage.prompt_tokens,
        output_tokens: usage.completion_tokens,
    })
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
            role: if m.role == "assistant" { "assistant" } else { "user" }.to_owned(),
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
    let text = body
        .content
        .into_iter()
        .filter(|block| block.kind == "text")
        .map(|block| block.text.unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ChatResponse {
        text,
        input_tokens: body.usage.as_ref().map(|u| u.input_tokens).unwrap_or(0),
        output_tokens: body.usage.as_ref().map(|u| u.output_tokens).unwrap_or(0),
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
    #[serde(default)]
    kind: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}
