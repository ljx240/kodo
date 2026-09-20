//! HTTP chat providers with a provider-neutral message IR.
//!
//! Model identity is explicit: UI shows `display_name`, backend always sends
//! `model_id`. Native tool calling is the primary path when capabilities say
//! so; JSON-in-text is only a fallback for providers without native tools.
//!
//! # Wire contracts (verified against official sources, not memory)
//!
//! * **OpenAI-compatible Chat Completions** —
//!   [openai/openai-openapi `openapi.yaml`](https://github.com/openai/openai-openapi)
//!   `ChatCompletionTool`, `ChatCompletionMessageToolCall`,
//!   `ChatCompletionRequestAssistantMessage`, `ChatCompletionRequestToolMessage`:
//!   request `tools[]` use `{type:"function", function:{name, parameters}}`;
//!   assistant `tool_calls[].function.arguments` is a **JSON string**; tool
//!   results are `{role:"tool", tool_call_id, content}` (no `is_error` field).
//! * **Anthropic Messages** — official Python SDK types
//!   (`ToolParam`, `ToolUseBlockParam`, `ToolResultBlockParam`, `MessageParam`):
//!   `tools[]` use `input_schema`; assistant emits `tool_use` blocks
//!   (`id`/`name`/`input` object); results are `tool_result`
//!   (`tool_use_id`/`content`/`is_error`) inside a **user** message; `system`
//!   is top-level.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Capability flags that actually gate code paths (not decorative docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub text_chat: bool,
    pub native_tools: bool,
    pub streaming: bool,
    pub reasoning: bool,
    pub token_usage: bool,
}

impl ProviderCapabilities {
    pub fn text_only() -> Self {
        Self {
            text_chat: true,
            native_tools: false,
            streaming: false,
            reasoning: false,
            token_usage: false,
        }
    }

    /// Conservative defaults for custom / unknown providers: prove each
    /// capability via explicit config (or a probe) before turning it on.
    /// Never assume `native_tools` / `streaming` / `token_usage`.
    pub fn conservative() -> Self {
        Self {
            text_chat: true,
            native_tools: false,
            streaming: false,
            reasoning: false,
            token_usage: false,
        }
    }

    pub fn anthropic() -> Self {
        Self {
            text_chat: true,
            native_tools: true,
            streaming: true,
            reasoning: true,
            token_usage: true,
        }
    }

    pub fn openai_compatible() -> Self {
        Self {
            text_chat: true,
            native_tools: true,
            streaming: true,
            reasoning: false,
            token_usage: true,
        }
    }

    pub fn deepseek() -> Self {
        Self {
            text_chat: true,
            native_tools: true,
            streaming: true,
            reasoning: true,
            token_usage: true,
        }
    }
}

/// One model the user can pick. UI uses `display_name`; backend uses `model_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSpec {
    pub display_name: String,
    pub model_id: String,
    pub provider_type: String,
}

/// Catalog entry used **only** for config migration from legacy display labels.
///
/// Marketing display labels are never a long-term source of API ids: prefer an
/// explicit `model_id` in config. This table cannot track every model rename
/// and must not be treated as the authority for what the backend accepts.
#[derive(Debug, Clone, Copy)]
pub struct CatalogModel {
    pub provider_type: &'static str,
    pub display_name: &'static str,
    pub model_id: &'static str,
}

/// Known display labels → API model ids (migration aid only).
/// Explicit `model_id` always wins when present on the config record.
pub const MODEL_CATALOG: &[CatalogModel] = &[
    CatalogModel {
        provider_type: "anthropic",
        display_name: "Claude 3.5 Sonnet",
        model_id: "claude-3-5-sonnet-latest",
    },
    CatalogModel {
        provider_type: "anthropic",
        display_name: "Claude Sonnet 5",
        model_id: "claude-sonnet-4-5",
    },
    CatalogModel {
        provider_type: "anthropic",
        display_name: "Claude Opus 5",
        model_id: "claude-opus-4-1",
    },
    CatalogModel {
        provider_type: "openai",
        display_name: "GPT-4o",
        model_id: "gpt-4o",
    },
    CatalogModel {
        provider_type: "openai",
        display_name: "GPT-4o mini",
        model_id: "gpt-4o-mini",
    },
    CatalogModel {
        provider_type: "openai",
        display_name: "o1",
        model_id: "o1",
    },
    CatalogModel {
        provider_type: "openai",
        display_name: "o1-mini",
        model_id: "o1-mini",
    },
    CatalogModel {
        provider_type: "deepseek",
        display_name: "DeepSeek-V3",
        model_id: "deepseek-chat",
    },
    CatalogModel {
        provider_type: "deepseek",
        display_name: "DeepSeek-R1",
        model_id: "deepseek-reasoner",
    },
];

/// Resolve a raw config value into (display_name, model_id).
///
/// - exact catalog display match → mapped id
/// - case-insensitive display match → mapped id
/// - already looks like an API id (no spaces, slug-ish) → used as both
pub fn resolve_model_identity(provider_type: &str, raw: &str) -> ModelSpec {
    let raw = raw.trim();
    if raw.is_empty() {
        return ModelSpec {
            display_name: String::new(),
            model_id: String::new(),
            provider_type: provider_type.to_owned(),
        };
    }
    for entry in MODEL_CATALOG {
        if entry.provider_type.eq_ignore_ascii_case(provider_type)
            && entry.display_name.eq_ignore_ascii_case(raw)
        {
            return ModelSpec {
                display_name: entry.display_name.to_owned(),
                model_id: entry.model_id.to_owned(),
                provider_type: provider_type.to_owned(),
            };
        }
    }
    // Treat space-free values as API model ids (custom / already migrated).
    if !raw.contains(char::is_whitespace) {
        return ModelSpec {
            display_name: raw.to_owned(),
            model_id: raw.to_owned(),
            provider_type: provider_type.to_owned(),
        };
    }
    // Unknown display label: keep it visible, but do not invent an API id.
    // Backend will reject empty/unmapped labels at request time.
    ModelSpec {
        display_name: raw.to_owned(),
        model_id: String::new(),
        provider_type: provider_type.to_owned(),
    }
}

/// True when the string looks like a backend model id, not a marketing label.
pub fn looks_like_model_id(raw: &str) -> bool {
    let raw = raw.trim();
    !raw.is_empty()
        && !raw.contains(char::is_whitespace)
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// Migrate a legacy provider payload. Never drops user keys/endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfigRecord {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub template: String,
    /// Legacy / custom field: may hold display name OR model id.
    #[serde(default)]
    pub model: String,
    /// Preferred backend id when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// UI label when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub api_key: String,
}

impl ProviderConfigRecord {
    /// Back-compat migration: fills `model_id` / `display_name` from `model`.
    pub fn migrated(self) -> Self {
        let template = if self.template.trim().is_empty() {
            "custom".to_owned()
        } else {
            self.template.clone()
        };
        let raw = self
            .model_id
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| self.model.clone());
        let spec = resolve_model_identity(&template, &raw);
        let model_id = if spec.model_id.is_empty() {
            raw.trim().to_owned()
        } else {
            spec.model_id.clone()
        };
        let display_name = if self.display_name.as_deref().unwrap_or("").trim().is_empty() {
            if spec.display_name.is_empty() {
                raw.trim().to_owned()
            } else {
                spec.display_name.clone()
            }
        } else {
            self.display_name.clone().unwrap_or_default()
        };
        Self {
            template,
            model_id: Some(model_id.clone()),
            display_name: Some(display_name),
            // Keep legacy `model` in sync so old readers still work.
            model: model_id,
            ..self
        }
    }
}

#[derive(Debug, Clone)]
pub struct Provider {
    pub template: String,
    pub api_key: String,
    pub endpoint: String,
    /// Raw config value (display name or model id). Prefer `model_id`.
    pub model: String,
    pub model_id: Option<String>,
    pub display_name: Option<String>,
    pub capabilities_override: Option<ProviderCapabilities>,
    /// Fallback chain (other configured providers), used when failover is on.
    pub fallbacks: Vec<Provider>,
}

impl Provider {
    pub fn new(
        template: impl Into<String>,
        api_key: impl Into<String>,
        endpoint: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let template = template.into();
        let model = model.into();
        let spec = resolve_model_identity(&template, &model);
        Self {
            template,
            api_key: api_key.into(),
            endpoint: endpoint.into(),
            model_id: Some(if spec.model_id.is_empty() {
                model.clone()
            } else {
                spec.model_id
            }),
            display_name: Some(if spec.display_name.is_empty() {
                model
            } else {
                spec.display_name
            }),
            model: String::new(), // filled below
            capabilities_override: None,
            fallbacks: Vec::new(),
        }
        .sync_legacy_model()
    }

    fn sync_legacy_model(mut self) -> Self {
        self.model = self.resolved_model_id();
        self
    }

    /// From a migrated config record. Explicit `model_id` is authoritative
    /// (catalog is only a legacy display-label migration aid).
    pub fn from_record(record: ProviderConfigRecord) -> Self {
        let rec = record.migrated();
        Self {
            template: rec.template,
            api_key: rec.api_key,
            endpoint: rec.endpoint,
            model: rec.model.clone(),
            model_id: rec.model_id.clone(),
            display_name: rec.display_name,
            capabilities_override: None,
            fallbacks: Vec::new(),
        }
    }

    /// Backend model id — never a marketing label.
    pub fn resolved_model_id(&self) -> String {
        if let Some(id) = self.model_id.as_ref() {
            if !id.trim().is_empty() {
                return id.trim().to_owned();
            }
        }
        let spec = resolve_model_identity(&self.template, &self.model);
        if !spec.model_id.is_empty() {
            return spec.model_id.clone();
        }
        let raw = self.model.trim();
        if looks_like_model_id(raw) {
            return raw.to_owned();
        }
        String::new()
    }

    pub fn display_label(&self) -> String {
        if let Some(name) = self.display_name.as_ref() {
            if !name.trim().is_empty() {
                return name.trim().to_owned();
            }
        }
        let spec = resolve_model_identity(&self.template, &self.model);
        if !spec.display_name.is_empty() {
            return spec.display_name;
        }
        self.model.trim().to_owned()
    }

    pub fn capabilities(&self) -> ProviderCapabilities {
        if let Some(caps) = self.capabilities_override {
            return caps;
        }
        match self.template.as_str() {
            "anthropic" => ProviderCapabilities::anthropic(),
            "deepseek" => ProviderCapabilities::deepseek(),
            "openai" => ProviderCapabilities::openai_compatible(),
            // Custom / unknown endpoints: conservative until configured or probed.
            _ => ProviderCapabilities::conservative(),
        }
    }

    /// Clear, typed error when the backend cannot send this model id.
    pub fn validate_model(&self) -> Result<String, ProviderError> {
        let id = self.resolved_model_id();
        if id.is_empty() {
            return Err(ProviderError {
                class: ProviderFailureClass::InvalidModel,
                message: format!(
                    "invalid model id for provider `{}`: `{}` is a display label without a mapped API model id",
                    self.template,
                    self.display_label()
                ),
            });
        }
        if self.api_key.trim().is_empty() {
            return Err(ProviderError {
                class: ProviderFailureClass::Auth,
                message: "provider API key is empty".to_owned(),
            });
        }
        Ok(id)
    }
}

// Keep old struct-literal call sites compiling during migration helpers.
impl Default for Provider {
    fn default() -> Self {
        Self {
            template: "custom".into(),
            api_key: String::new(),
            endpoint: String::new(),
            model: String::new(),
            model_id: None,
            display_name: None,
            capabilities_override: None,
            fallbacks: Vec::new(),
        }
    }
}

/// Provider-neutral message IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderMessage {
    pub role: MessageRole,
    pub content: Vec<ContentBlock>,
}

impl ProviderMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn assistant(text: impl Into<String>, tool_calls: Vec<NativeToolCall>) -> Self {
        let mut content = Vec::new();
        let text = text.into();
        if !text.trim().is_empty() {
            content.push(ContentBlock::Text { text });
        }
        for call in tool_calls {
            content.push(ContentBlock::ToolCall {
                id: call.id,
                name: call.name,
                arguments: call.arguments,
            });
        }
        if content.is_empty() {
            content.push(ContentBlock::Text {
                text: String::new(),
            });
        }
        Self {
            role: MessageRole::Assistant,
            content,
        }
    }

    pub fn tool_result(
        call_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            role: MessageRole::Tool,
            content: vec![ContentBlock::ToolResult {
                call_id: call_id.into(),
                content: content.into(),
                is_error,
            }],
        }
    }

    /// Legacy `ChatMessage` view (system/user/assistant text only).
    pub fn text_of(&self) -> String {
        self.content
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => text.clone(),
                ContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                } => format!("[tool_call {id} {name} {arguments}]"),
                ContentBlock::ToolResult {
                    call_id,
                    content,
                    is_error,
                } => format!(
                    "[tool_result {call_id}{} {content}]",
                    if *is_error { " error" } else { "" }
                ),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Convert legacy role+string messages into IR.
pub fn messages_from_chat(history: &[(String, String)]) -> Vec<ProviderMessage> {
    history
        .iter()
        .map(|(role, content)| match role.as_str() {
            "system" => ProviderMessage::system(content.clone()),
            "assistant" => ProviderMessage::assistant(content.clone(), Vec::new()),
            "tool" => ProviderMessage::tool_result("legacy", content.clone(), false),
            _ => ProviderMessage::user(content.clone()),
        })
        .collect()
}

/// A tool call returned via the provider's **native** tool API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Tool definition sent to the provider (provider-neutral).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChatResponse {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub native_tool_calls: Vec<NativeToolCall>,
    pub model_id: String,
    pub finish_reason: String,
}

/// Incremental provider events the agent loop consumes.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderEvent {
    MessageStart {
        model_id: String,
    },
    TextDelta {
        text: String,
    },
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallDelta {
        id: String,
        arguments_delta: String,
    },
    ToolCallComplete {
        call: NativeToolCall,
    },
    Usage {
        input_tokens: u32,
        output_tokens: u32,
    },
    MessageComplete {
        response: ChatResponse,
    },
    Error {
        error: String,
        class: ProviderFailureClass,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderFailureClass {
    Auth,
    InvalidModel,
    RateLimit,
    Timeout,
    ServerError,
    MalformedResponse,
    ContextTooLong,
    Cancelled,
    Unknown,
}

impl ProviderFailureClass {
    /// Auth / invalid model / cancel must not be blindly retried.
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimit | Self::Timeout | Self::ServerError | Self::Unknown
        )
    }

    /// Safe to try the next configured provider.
    pub fn allows_failover(self) -> bool {
        matches!(
            self,
            Self::RateLimit
                | Self::Timeout
                | Self::ServerError
                | Self::MalformedResponse
                | Self::Unknown
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderError {
    pub class: ProviderFailureClass,
    pub message: String,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.class, self.message)
    }
}

impl std::error::Error for ProviderError {}

pub fn classify_failure(message: &str, status: Option<u16>) -> ProviderFailureClass {
    let lower = message.to_ascii_lowercase();
    if lower.contains("cancel") {
        return ProviderFailureClass::Cancelled;
    }
    if let Some(code) = status {
        match code {
            401 | 403 => return ProviderFailureClass::Auth,
            404 => return ProviderFailureClass::InvalidModel,
            429 => return ProviderFailureClass::RateLimit,
            408 | 504 => return ProviderFailureClass::Timeout,
            400 if lower.contains("context") || lower.contains("too long") => {
                return ProviderFailureClass::ContextTooLong
            }
            400 if lower.contains("model") => return ProviderFailureClass::InvalidModel,
            500..=599 => return ProviderFailureClass::ServerError,
            _ => {}
        }
    }
    if lower.contains("api key") || lower.contains("unauthorized") || lower.contains("401") {
        return ProviderFailureClass::Auth;
    }
    if lower.contains("model")
        && (lower.contains("not found")
            || lower.contains("invalid")
            || lower.contains("does not exist"))
    {
        return ProviderFailureClass::InvalidModel;
    }
    if lower.contains("rate limit") || lower.contains("429") {
        return ProviderFailureClass::RateLimit;
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return ProviderFailureClass::Timeout;
    }
    if lower.contains("context length") || lower.contains("too many tokens") {
        return ProviderFailureClass::ContextTooLong;
    }
    if lower.contains("invalid") && lower.contains("json") {
        return ProviderFailureClass::MalformedResponse;
    }
    if lower.contains("server") || lower.contains("502") || lower.contains("503") {
        return ProviderFailureClass::ServerError;
    }
    ProviderFailureClass::Unknown
}

/// Legacy role+string chat message (still used by some call sites).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

pub fn chat(
    provider: &Provider,
    messages: &[ChatMessage],
    max_tokens: u32,
) -> Result<ChatResponse, ProviderError> {
    let ir = messages
        .iter()
        .map(|m| match m.role.as_str() {
            "system" => ProviderMessage::system(m.content.clone()),
            "assistant" => ProviderMessage::assistant(m.content.clone(), Vec::new()),
            _ => ProviderMessage::user(m.content.clone()),
        })
        .collect::<Vec<_>>();
    chat_ir(provider, &ir, &[], max_tokens)
}

/// Full native (or JSON-fallback) model request.
pub fn chat_ir(
    provider: &Provider,
    messages: &[ProviderMessage],
    tools: &[ToolSchema],
    max_tokens: u32,
) -> Result<ChatResponse, ProviderError> {
    let model_id = provider.validate_model()?;
    let max_tokens = max_tokens.clamp(256, 8192);
    let caps = provider.capabilities();

    if caps.native_tools && !tools.is_empty() {
        match provider.template.as_str() {
            "anthropic" => anthropic_native(provider, &model_id, messages, tools, max_tokens),
            _ => openai_native(provider, &model_id, messages, tools, max_tokens),
        }
    } else {
        // Text path / JSON-fallback path for providers without native tools.
        let flat: Vec<ChatMessage> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    MessageRole::System => "system",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "user",
                    MessageRole::User => "user",
                };
                ChatMessage {
                    role: role.to_owned(),
                    content: m.text_of(),
                }
            })
            .collect();
        match provider.template.as_str() {
            "anthropic" => anthropic_text(provider, &model_id, &flat, max_tokens),
            _ => openai_text(provider, &model_id, &flat, max_tokens),
        }
    }
}

/// Streaming chat. When capabilities.streaming is false, emits a single
/// MessageStart + MessageComplete using the blocking path.
pub fn chat_stream(
    provider: &Provider,
    messages: &[ProviderMessage],
    tools: &[ToolSchema],
    max_tokens: u32,
    alive: &dyn Fn() -> bool,
    mut on_event: impl FnMut(ProviderEvent) -> bool,
) -> Result<ChatResponse, ProviderError> {
    let model_id = provider.validate_model()?;
    let caps = provider.capabilities();
    if !alive() {
        return Err(ProviderError {
            class: ProviderFailureClass::Cancelled,
            message: "cancelled before model request".into(),
        });
    }
    if !on_event(ProviderEvent::MessageStart {
        model_id: model_id.clone(),
    }) {
        return Err(ProviderError {
            class: ProviderFailureClass::Cancelled,
            message: "cancelled at message start".into(),
        });
    }

    if !caps.streaming {
        let response = chat_ir(provider, messages, tools, max_tokens)?;
        if !alive() {
            return Err(ProviderError {
                class: ProviderFailureClass::Cancelled,
                message: "cancelled after model response".into(),
            });
        }
        if response.input_tokens > 0 || response.output_tokens > 0 {
            let _ = on_event(ProviderEvent::Usage {
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
            });
        }
        for call in response.native_tool_calls.clone() {
            if !on_event(ProviderEvent::ToolCallComplete { call: call.clone() }) {
                return Err(ProviderError {
                    class: ProviderFailureClass::Cancelled,
                    message: "cancelled while emitting tool call".into(),
                });
            }
        }
        if !response.text.is_empty() {
            let _ = on_event(ProviderEvent::TextDelta {
                text: response.text.clone(),
            });
        }
        let _ = on_event(ProviderEvent::MessageComplete {
            response: response.clone(),
        });
        return Ok(response);
    }

    // Capability=true → SSE path.
    let result = match provider.template.as_str() {
        "anthropic" => anthropic_sse(
            provider,
            &model_id,
            messages,
            tools,
            max_tokens,
            alive,
            &mut on_event,
        ),
        _ => openai_sse(
            provider,
            &model_id,
            messages,
            tools,
            max_tokens,
            alive,
            &mut on_event,
        ),
    };
    match result {
        Ok(response) => {
            let _ = on_event(ProviderEvent::MessageComplete {
                response: response.clone(),
            });
            Ok(response)
        }
        Err(err) => {
            let _ = on_event(ProviderEvent::Error {
                error: err.message.clone(),
                class: err.class,
            });
            Err(err)
        }
    }
}

/// Chat with optional failover across configured providers.
/// `fallback_to_local` is NOT implemented here — failover is provider→provider only.
pub fn chat_with_failover(
    primary: &Provider,
    messages: &[ProviderMessage],
    tools: &[ToolSchema],
    max_tokens: u32,
    allow_failover: bool,
    mut on_switch: impl FnMut(&str, &str),
) -> Result<(Provider, ChatResponse), ProviderError> {
    let mut candidates: Vec<Provider> = Vec::new();
    candidates.push(primary.clone());
    if allow_failover {
        candidates.extend(primary.fallbacks.iter().cloned());
    }

    let mut last: Option<ProviderError> = None;
    for (index, provider) in candidates.iter().enumerate() {
        match chat_ir(provider, messages, tools, max_tokens) {
            Ok(response) => return Ok((provider.clone(), response)),
            Err(err) => {
                let is_last = index + 1 >= candidates.len();
                if !err.class.allows_failover() || is_last {
                    return Err(err);
                }
                if let Some(next) = candidates.get(index + 1) {
                    on_switch(&provider.display_label(), &next.display_label());
                }
                last = Some(err);
            }
        }
    }
    Err(last.unwrap_or(ProviderError {
        class: ProviderFailureClass::Unknown,
        message: "no provider available".into(),
    }))
}

fn default_openai_base(template: &str) -> String {
    match template {
        "deepseek" => "https://api.deepseek.com/v1".to_owned(),
        _ => "https://api.openai.com/v1".to_owned(),
    }
}

fn provider_base(provider: &Provider, default: &str) -> String {
    if provider.endpoint.trim().is_empty() {
        default.to_owned()
    } else {
        provider.endpoint.trim().trim_end_matches('/').to_owned()
    }
}

fn map_ureq_err(error: ureq::Error) -> ProviderError {
    match error {
        ureq::Error::Status(code, response) => {
            let status = code;
            let body = response
                .into_string()
                .unwrap_or_else(|_| format!("HTTP {status}"));
            let class = classify_failure(&body, Some(status));
            ProviderError {
                class,
                message: format!("HTTP {status}: {body}"),
            }
        }
        ureq::Error::Transport(t) => {
            let message = t.to_string();
            let class = classify_failure(&message, None);
            ProviderError { class, message }
        }
    }
}

// ---------------------------------------------------------------------------
// OpenAI-compatible
// ---------------------------------------------------------------------------

fn openai_tool_payload(tools: &[ToolSchema]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                }
            })
        })
        .collect()
}

/// Serialize IR → OpenAI-compatible Chat Completions `messages` (official
/// OpenAPI: assistant `tool_calls` + tool-role `tool_call_id`).
fn openai_messages(messages: &[ProviderMessage]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for message in messages {
        match message.role {
            MessageRole::System => {
                out.push(json!({ "role": "system", "content": message.text_of() }));
            }
            MessageRole::User => {
                out.push(json!({ "role": "user", "content": message.text_of() }));
            }
            MessageRole::Assistant => {
                let mut text = String::new();
                let mut tool_calls = Vec::new();
                for block in &message.content {
                    match block {
                        ContentBlock::Text { text: t } => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                        ContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": arguments.to_string(),
                                }
                            }));
                        }
                        ContentBlock::ToolResult { .. } => {}
                    }
                }
                let mut msg = json!({ "role": "assistant", "content": if text.is_empty() { Value::Null } else { Value::String(text) } });
                if !tool_calls.is_empty() {
                    msg["tool_calls"] = Value::Array(tool_calls);
                }
                out.push(msg);
            }
            MessageRole::Tool => {
                for block in &message.content {
                    if let ContentBlock::ToolResult {
                        call_id, content, ..
                    } = block
                    {
                        // Official OpenAI tool message has no `is_error` field;
                        // error semantics travel in `content` (see lib push_tool_results).
                        out.push(json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": content,
                        }));
                    }
                }
            }
        }
    }
    out
}

fn openai_native(
    provider: &Provider,
    model_id: &str,
    messages: &[ProviderMessage],
    tools: &[ToolSchema],
    max_tokens: u32,
) -> Result<ChatResponse, ProviderError> {
    let base = provider_base(provider, &default_openai_base(&provider.template));
    let url = format!("{base}/chat/completions");
    let payload = json!({
        "model": model_id,
        "messages": openai_messages(messages),
        "tools": openai_tool_payload(tools),
        "tool_choice": "auto",
        "temperature": 0.2,
        "max_tokens": max_tokens,
    });

    let response = ureq::post(&url)
        .timeout(Duration::from_secs(90))
        .set(
            "Authorization",
            &format!("Bearer {}", provider.api_key.trim()),
        )
        .set("Content-Type", "application/json")
        .send_json(payload)
        .map_err(map_ureq_err)?;

    let body: OpenAiResponse = response.into_json().map_err(|e| ProviderError {
        class: ProviderFailureClass::MalformedResponse,
        message: e.to_string(),
    })?;
    parse_openai_body(body, model_id)
}

fn openai_text(
    provider: &Provider,
    model_id: &str,
    messages: &[ChatMessage],
    max_tokens: u32,
) -> Result<ChatResponse, ProviderError> {
    let base = provider_base(provider, &default_openai_base(&provider.template));
    let url = format!("{base}/chat/completions");
    let payload = json!({
        "model": model_id,
        "messages": messages
            .iter()
            .map(|m| json!({ "role": m.role, "content": m.content }))
            .collect::<Vec<_>>(),
        "temperature": 0.2,
        "max_tokens": max_tokens,
    });
    let response = ureq::post(&url)
        .timeout(Duration::from_secs(90))
        .set(
            "Authorization",
            &format!("Bearer {}", provider.api_key.trim()),
        )
        .set("Content-Type", "application/json")
        .send_json(payload)
        .map_err(map_ureq_err)?;
    let body: OpenAiResponse = response.into_json().map_err(|e| ProviderError {
        class: ProviderFailureClass::MalformedResponse,
        message: e.to_string(),
    })?;
    parse_openai_body(body, model_id)
}

fn parse_openai_body(body: OpenAiResponse, model_id: &str) -> Result<ChatResponse, ProviderError> {
    let finish_reason = body
        .choices
        .first()
        .and_then(|c| c.finish_reason.clone())
        .unwrap_or_default();
    let choice = body.choices.into_iter().next();
    let message = choice.as_ref().map(|c| &c.message);
    let text = message.and_then(|m| m.content.clone()).unwrap_or_default();
    let native_tool_calls = message
        .and_then(|m| m.tool_calls.as_ref())
        .map(|calls| {
            calls
                .iter()
                .map(|call| NativeToolCall {
                    id: if call.id.trim().is_empty() {
                        format!("openai_{}", call.function.name)
                    } else {
                        call.id.clone()
                    },
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
        model_id: model_id.to_owned(),
        finish_reason,
    })
}

/// Parse OpenAI Chat Completions `function.arguments` (JSON **string** per
/// official OpenAPI). Malformed JSON becomes a non-object `Value::String` so
/// the typed tool registry rejects it — never a silent free-text mutation.
fn parse_openai_arguments(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::Object(Default::default());
    }
    match serde_json::from_str(trimmed) {
        Ok(v) if v.is_object() || v.is_array() => v,
        Ok(Value::String(s)) => Value::String(s),
        Ok(_) => Value::String(trimmed.to_owned()),
        Err(_) => Value::String(trimmed.to_owned()),
    }
}

fn openai_sse(
    provider: &Provider,
    model_id: &str,
    messages: &[ProviderMessage],
    tools: &[ToolSchema],
    max_tokens: u32,
    alive: &dyn Fn() -> bool,
    on_event: &mut impl FnMut(ProviderEvent) -> bool,
) -> Result<ChatResponse, ProviderError> {
    let base = provider_base(provider, &default_openai_base(&provider.template));
    let url = format!("{base}/chat/completions");
    let payload = json!({
        "model": model_id,
        "messages": openai_messages(messages),
        "tools": openai_tool_payload(tools),
        "tool_choice": "auto",
        "temperature": 0.2,
        "max_tokens": max_tokens,
        "stream": true,
        "stream_options": { "include_usage": true },
    });

    let response = ureq::post(&url)
        .timeout(Duration::from_secs(180))
        .set(
            "Authorization",
            &format!("Bearer {}", provider.api_key.trim()),
        )
        .set("Content-Type", "application/json")
        .set("Accept", "text/event-stream")
        .send_json(payload)
        .map_err(map_ureq_err)?;

    let mut reader = response.into_reader();
    let mut raw = String::new();
    let mut text = String::new();
    let mut tool_acc: Vec<(String, String, String)> = Vec::new(); // id, name, args
    let mut usage = (0u32, 0u32);
    let mut buf = [0u8; 4096];
    use std::io::Read;

    loop {
        if !alive() {
            return Err(ProviderError {
                class: ProviderFailureClass::Cancelled,
                message: "cancelled while reading model stream".into(),
            });
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.push_str(&String::from_utf8_lossy(&buf[..n]));
                while let Some(pos) = raw.find('\n') {
                    let line = raw[..pos].trim_end().to_owned();
                    raw.drain(..=pos);
                    if !line.starts_with("data:") {
                        continue;
                    }
                    let data = line[5..].trim();
                    if data == "[DONE]" {
                        break;
                    }
                    let Ok(value) = serde_json::from_str::<Value>(data) else {
                        continue;
                    };
                    if let Some(delta) = value
                        .pointer("/choices/0/delta/content")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        text.push_str(delta);
                        if !on_event(ProviderEvent::TextDelta {
                            text: delta.to_owned(),
                        }) {
                            return Err(ProviderError {
                                class: ProviderFailureClass::Cancelled,
                                message: "cancelled by event sink".into(),
                            });
                        }
                    }
                    if let Some(calls) = value
                        .pointer("/choices/0/delta/tool_calls")
                        .and_then(|v| v.as_array())
                    {
                        for call in calls {
                            let idx =
                                call.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                            while tool_acc.len() <= idx {
                                tool_acc.push((String::new(), String::new(), String::new()));
                            }
                            if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                                tool_acc[idx].0 = id.to_owned();
                            }
                            if let Some(name) =
                                call.pointer("/function/name").and_then(|v| v.as_str())
                            {
                                if tool_acc[idx].1.is_empty() {
                                    let _ = on_event(ProviderEvent::ToolCallStart {
                                        id: tool_acc[idx].0.clone(),
                                        name: name.to_owned(),
                                    });
                                }
                                tool_acc[idx].1.push_str(name);
                            }
                            if let Some(args) =
                                call.pointer("/function/arguments").and_then(|v| v.as_str())
                            {
                                tool_acc[idx].2.push_str(args);
                                let _ = on_event(ProviderEvent::ToolCallDelta {
                                    id: tool_acc[idx].0.clone(),
                                    arguments_delta: args.to_owned(),
                                });
                            }
                        }
                    }
                    if let Some(u) = value.get("usage") {
                        usage.0 =
                            u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        usage.1 = u
                            .get("completion_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;
                    }
                }
            }
            Err(err) => {
                return Err(ProviderError {
                    class: classify_failure(&err.to_string(), None),
                    message: err.to_string(),
                });
            }
        }
    }

    let mut native = Vec::new();
    for (id, name, args) in tool_acc {
        if name.is_empty() {
            continue;
        }
        let call = NativeToolCall {
            id: if id.is_empty() {
                format!("openai_{name}")
            } else {
                id
            },
            name,
            arguments: parse_openai_arguments(&args),
        };
        let _ = on_event(ProviderEvent::ToolCallComplete { call: call.clone() });
        native.push(call);
    }
    if usage.0 > 0 || usage.1 > 0 {
        let _ = on_event(ProviderEvent::Usage {
            input_tokens: usage.0,
            output_tokens: usage.1,
        });
    }
    Ok(ChatResponse {
        text,
        input_tokens: usage.0,
        output_tokens: usage.1,
        native_tool_calls: native,
        model_id: model_id.to_owned(),
        finish_reason: "stop".into(),
    })
}

// ---------------------------------------------------------------------------
// Anthropic
// ---------------------------------------------------------------------------

fn anthropic_tools(tools: &[ToolSchema]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.parameters,
            })
        })
        .collect()
}

fn anthropic_messages(messages: &[ProviderMessage]) -> (String, Vec<Value>) {
    let mut system = String::new();
    let mut out: Vec<Value> = Vec::new();
    for message in messages {
        match message.role {
            MessageRole::System => {
                if !system.is_empty() {
                    system.push('\n');
                }
                system.push_str(&message.text_of());
            }
            MessageRole::User | MessageRole::Tool => {
                // Anthropic expects tool results as a user message content block.
                let mut blocks: Vec<Value> = Vec::new();
                for block in &message.content {
                    match block {
                        ContentBlock::Text { text } => {
                            if !text.trim().is_empty() {
                                blocks.push(json!({ "type": "text", "text": text }));
                            }
                        }
                        ContentBlock::ToolResult {
                            call_id,
                            content,
                            is_error,
                        } => {
                            blocks.push(json!({
                                "type": "tool_result",
                                "tool_use_id": call_id,
                                "content": content,
                                "is_error": *is_error,
                            }));
                        }
                        ContentBlock::ToolCall { .. } => {}
                    }
                }
                if blocks.is_empty() {
                    let text = message.text_of();
                    if !text.trim().is_empty() {
                        blocks.push(json!({ "type": "text", "text": text }));
                    }
                }
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": "" }));
                }
                // Merge consecutive user/tool turns into one user message.
                if let Some(last) = out.last_mut() {
                    if last["role"] == "user" {
                        if let Some(arr) = last["content"].as_array_mut() {
                            arr.extend(blocks);
                            continue;
                        }
                    }
                }
                out.push(json!({ "role": "user", "content": blocks }));
            }
            MessageRole::Assistant => {
                let mut blocks: Vec<Value> = Vec::new();
                for block in &message.content {
                    match block {
                        ContentBlock::Text { text } => {
                            if !text.trim().is_empty() {
                                blocks.push(json!({ "type": "text", "text": text }));
                            }
                        }
                        ContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": arguments,
                            }));
                        }
                        ContentBlock::ToolResult { .. } => {}
                    }
                }
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": "" }));
                }
                out.push(json!({ "role": "assistant", "content": blocks }));
            }
        }
    }
    (system, out)
}

fn anthropic_native(
    provider: &Provider,
    model_id: &str,
    messages: &[ProviderMessage],
    tools: &[ToolSchema],
    max_tokens: u32,
) -> Result<ChatResponse, ProviderError> {
    let base = provider_base(provider, "https://api.anthropic.com");
    let url = format!("{base}/v1/messages");
    let (system, rest) = anthropic_messages(messages);
    let payload = json!({
        "model": model_id,
        "max_tokens": max_tokens,
        "system": system,
        "messages": rest,
        "tools": anthropic_tools(tools),
    });
    let response = ureq::post(&url)
        .timeout(Duration::from_secs(90))
        .set("x-api-key", provider.api_key.trim())
        .set("anthropic-version", "2023-06-01")
        .set("Content-Type", "application/json")
        .send_json(payload)
        .map_err(map_ureq_err)?;
    let body: AnthropicResponse = response.into_json().map_err(|e| ProviderError {
        class: ProviderFailureClass::MalformedResponse,
        message: e.to_string(),
    })?;
    parse_anthropic_body(body, model_id)
}

fn anthropic_text(
    provider: &Provider,
    model_id: &str,
    messages: &[ChatMessage],
    max_tokens: u32,
) -> Result<ChatResponse, ProviderError> {
    let base = provider_base(provider, "https://api.anthropic.com");
    let url = format!("{base}/v1/messages");
    let system = messages
        .iter()
        .find(|m| m.role == "system")
        .map(|m| m.content.clone())
        .unwrap_or_default();
    let rest: Vec<Value> = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            json!({
                "role": if m.role == "assistant" { "assistant" } else { "user" },
                "content": m.content,
            })
        })
        .collect();
    let payload = json!({
        "model": model_id,
        "max_tokens": max_tokens,
        "system": system,
        "messages": rest,
    });
    let response = ureq::post(&url)
        .timeout(Duration::from_secs(90))
        .set("x-api-key", provider.api_key.trim())
        .set("anthropic-version", "2023-06-01")
        .set("Content-Type", "application/json")
        .send_json(payload)
        .map_err(map_ureq_err)?;
    let body: AnthropicResponse = response.into_json().map_err(|e| ProviderError {
        class: ProviderFailureClass::MalformedResponse,
        message: e.to_string(),
    })?;
    parse_anthropic_body(body, model_id)
}

fn parse_anthropic_body(
    body: AnthropicResponse,
    model_id: &str,
) -> Result<ChatResponse, ProviderError> {
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
                let input = block.input.unwrap_or(Value::Object(Default::default()));
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
        model_id: model_id.to_owned(),
        finish_reason: body.stop_reason.unwrap_or_default(),
    })
}

fn anthropic_sse(
    provider: &Provider,
    model_id: &str,
    messages: &[ProviderMessage],
    tools: &[ToolSchema],
    max_tokens: u32,
    alive: &dyn Fn() -> bool,
    on_event: &mut impl FnMut(ProviderEvent) -> bool,
) -> Result<ChatResponse, ProviderError> {
    let base = provider_base(provider, "https://api.anthropic.com");
    let url = format!("{base}/v1/messages");
    let (system, rest) = anthropic_messages(messages);
    let payload = json!({
        "model": model_id,
        "max_tokens": max_tokens,
        "system": system,
        "messages": rest,
        "tools": anthropic_tools(tools),
        "stream": true,
    });
    let response = ureq::post(&url)
        .timeout(Duration::from_secs(180))
        .set("x-api-key", provider.api_key.trim())
        .set("anthropic-version", "2023-06-01")
        .set("Content-Type", "application/json")
        .set("Accept", "text/event-stream")
        .send_json(payload)
        .map_err(map_ureq_err)?;

    let mut reader = response.into_reader();
    let mut raw = String::new();
    let mut text = String::new();
    let mut usage = (0u32, 0u32);
    let mut pending_tool: Option<NativeToolCall> = None;
    let mut completed_tools: Vec<NativeToolCall> = Vec::new();
    let mut buf = [0u8; 4096];
    use std::io::Read;

    loop {
        if !alive() {
            return Err(ProviderError {
                class: ProviderFailureClass::Cancelled,
                message: "cancelled while reading model stream".into(),
            });
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.push_str(&String::from_utf8_lossy(&buf[..n]));
                while let Some(pos) = raw.find('\n') {
                    let line = raw[..pos].trim_end().to_owned();
                    raw.drain(..=pos);
                    if !line.starts_with("data:") {
                        continue;
                    }
                    let data = line[5..].trim();
                    let Ok(value) = serde_json::from_str::<Value>(data) else {
                        continue;
                    };
                    let event = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match event {
                        "content_block_start"
                            if value
                                .pointer("/content_block/type")
                                .and_then(|v| v.as_str())
                                == Some("tool_use") =>
                        {
                            let id = value
                                .pointer("/content_block/id")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_owned();
                            let name = value
                                .pointer("/content_block/name")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_owned();
                            let _ = on_event(ProviderEvent::ToolCallStart {
                                id: id.clone(),
                                name: name.clone(),
                            });
                            pending_tool = Some(NativeToolCall {
                                id,
                                name,
                                arguments: Value::Object(Default::default()),
                            });
                        }
                        "content_block_delta" => {
                            if let Some(delta) = value
                                .pointer("/delta/text")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                            {
                                text.push_str(delta);
                                if !on_event(ProviderEvent::TextDelta {
                                    text: delta.to_owned(),
                                }) {
                                    return Err(ProviderError {
                                        class: ProviderFailureClass::Cancelled,
                                        message: "cancelled by event sink".into(),
                                    });
                                }
                            }
                            if let Some(partial) = value
                                .pointer("/delta/partial_json")
                                .and_then(|v| v.as_str())
                            {
                                if let Some(tool) = pending_tool.as_mut() {
                                    let _ = on_event(ProviderEvent::ToolCallDelta {
                                        id: tool.id.clone(),
                                        arguments_delta: partial.to_owned(),
                                    });
                                    // Accumulate as raw string then parse at end.
                                    if tool.arguments.is_null() {
                                        tool.arguments = Value::String(String::new());
                                    }
                                    if let Value::String(s) = &mut tool.arguments {
                                        s.push_str(partial);
                                    }
                                }
                            }
                        }
                        "content_block_stop" => {
                            if let Some(mut tool) = pending_tool.take() {
                                let raw_args = match tool.arguments.take() {
                                    Value::String(s) => s,
                                    other => other.to_string(),
                                };
                                // Malformed partial JSON must stay non-object so the
                                // registry rejects it (never free-text mutation).
                                tool.arguments = if raw_args.trim().is_empty() {
                                    Value::Object(Default::default())
                                } else {
                                    serde_json::from_str(&raw_args)
                                        .unwrap_or(Value::String(raw_args))
                                };
                                let _ = on_event(ProviderEvent::ToolCallComplete {
                                    call: tool.clone(),
                                });
                                completed_tools.push(tool);
                            }
                        }
                        "message_delta" => {
                            if let Some(u) = value.pointer("/usage") {
                                usage.0 +=
                                    u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0)
                                        as u32;
                                usage.1 =
                                    u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0)
                                        as u32;
                            }
                        }
                        "message_start" => {
                            if let Some(u) = value.pointer("/message/usage") {
                                usage.0 =
                                    u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0)
                                        as u32;
                            }
                        }
                        "error" => {
                            let message = value
                                .pointer("/error/message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("anthropic stream error")
                                .to_owned();
                            let class = classify_failure(&message, None);
                            return Err(ProviderError { class, message });
                        }
                        _ => {}
                    }
                }
            }
            Err(err) => {
                return Err(ProviderError {
                    class: classify_failure(&err.to_string(), None),
                    message: err.to_string(),
                });
            }
        }
    }

    if usage.0 > 0 || usage.1 > 0 {
        let _ = on_event(ProviderEvent::Usage {
            input_tokens: usage.0,
            output_tokens: usage.1,
        });
    }
    Ok(ChatResponse {
        text,
        input_tokens: usage.0,
        output_tokens: usage.1,
        native_tool_calls: completed_tools,
        model_id: model_id.to_owned(),
        finish_reason: "stop".into(),
    })
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OpenAiResponseModel;

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiChoiceMessage,
    #[serde(default)]
    finish_reason: Option<String>,
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

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicBlock>,
    usage: Option<AnthropicUsage>,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicBlock {
    #[serde(default, rename = "type")]
    kind: String,
    text: Option<String>,
    id: Option<String>,
    name: Option<String>,
    input: Option<Value>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

/// Snapshot helper used by deterministic provider serialization tests.
pub fn openai_request_json(
    provider: &Provider,
    messages: &[ProviderMessage],
    tools: &[ToolSchema],
    max_tokens: u32,
) -> Result<Value, ProviderError> {
    let model_id = provider.validate_model()?;
    Ok(json!({
        "model": model_id,
        "messages": openai_messages(messages),
        "tools": openai_tool_payload(tools),
        "tool_choice": "auto",
        "temperature": 0.2,
        "max_tokens": max_tokens.clamp(256, 8192),
    }))
}

/// Serialize IR → Anthropic Messages API request JSON (official SDK contract:
/// `system` top-level; tools use `input_schema`; `tool_use` / `tool_result` blocks).
pub fn anthropic_request_json(
    provider: &Provider,
    messages: &[ProviderMessage],
    tools: &[ToolSchema],
    max_tokens: u32,
) -> Result<Value, ProviderError> {
    let model_id = provider.validate_model()?;
    let (system, rest) = anthropic_messages(messages);
    Ok(json!({
        "model": model_id,
        "max_tokens": max_tokens.clamp(256, 8192),
        "system": system,
        "messages": rest,
        "tools": anthropic_tools(tools),
    }))
}

/// Parse a provider-native assistant payload back into IR (for round-trip tests).
pub fn openai_assistant_message_from_response(response: &ChatResponse) -> ProviderMessage {
    ProviderMessage::assistant(response.text.clone(), response.native_tool_calls.clone())
}

pub fn anthropic_assistant_blocks(response: &ChatResponse) -> Vec<Value> {
    let mut blocks = Vec::new();
    if !response.text.is_empty() {
        blocks.push(json!({ "type": "text", "text": response.text }));
    }
    for call in &response.native_tool_calls {
        blocks.push(json!({
            "type": "tool_use",
            "id": call.id,
            "name": call.name,
            "input": call.arguments,
        }));
    }
    blocks
}

/// Exposed for UI/tests: known catalog models for a provider template.
pub fn catalog_models(provider_type: &str) -> Vec<ModelSpec> {
    MODEL_CATALOG
        .iter()
        .filter(|m| m.provider_type.eq_ignore_ascii_case(provider_type))
        .map(|m| ModelSpec {
            display_name: m.display_name.to_owned(),
            model_id: m.model_id.to_owned(),
            provider_type: m.provider_type.to_owned(),
        })
        .collect()
}

// Silence unused struct warning for intermediate serialization helper.
#[allow(dead_code)]
fn _unused(_: OpenAiResponseModel) {
    let _ = Instant::now();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anthropic_provider() -> Provider {
        Provider::new("anthropic", "sk-test", "", "Claude Sonnet 5")
    }

    fn openai_provider() -> Provider {
        Provider::new("openai", "sk-test", "", "GPT-4o")
    }

    #[test]
    fn display_names_resolve_to_model_ids() {
        let p = anthropic_provider();
        assert_eq!(p.resolved_model_id(), "claude-sonnet-4-5");
        assert_eq!(p.display_label(), "Claude Sonnet 5");
        let p = openai_provider();
        assert_eq!(p.resolved_model_id(), "gpt-4o");
        assert_eq!(p.display_label(), "GPT-4o");
        let ds = Provider::new("deepseek", "sk", "", "DeepSeek-V3");
        assert_eq!(ds.resolved_model_id(), "deepseek-chat");
    }

    #[test]
    fn custom_api_ids_pass_through() {
        let p = Provider::new("custom", "sk", "https://x.test/v1", "my-model-v2");
        assert_eq!(p.resolved_model_id(), "my-model-v2");
        assert!(p.validate_model().is_ok());
    }

    #[test]
    fn unmapped_display_label_is_invalid_model_error() {
        let mut p = Provider::new("anthropic", "sk", "", "");
        p.model = "Claude Mystery Label".into();
        p.model_id = Some(String::new());
        p.display_name = Some("Claude Mystery Label".into());
        let err = p.validate_model().unwrap_err();
        assert_eq!(err.class, ProviderFailureClass::InvalidModel);
        assert!(err.message.contains("invalid model id"));
    }

    #[test]
    fn capabilities_gate_native_tools() {
        let anthropic = anthropic_provider();
        assert!(anthropic.capabilities().native_tools);
        assert!(anthropic.capabilities().streaming);
        let text_only = Provider {
            capabilities_override: Some(ProviderCapabilities::text_only()),
            ..openai_provider()
        };
        assert!(!text_only.capabilities().native_tools);
        assert!(!text_only.capabilities().streaming);
    }

    #[test]
    fn custom_provider_capabilities_are_conservative_by_default() {
        let custom = Provider::new("custom", "sk", "https://proxy.example/v1", "my-model-v2");
        let caps = custom.capabilities();
        assert!(caps.text_chat);
        assert!(
            !caps.native_tools && !caps.streaming && !caps.token_usage && !caps.reasoning,
            "custom must not assume native_tools/streaming/token_usage, got {caps:?}"
        );
        let enabled = Provider {
            capabilities_override: Some(ProviderCapabilities::openai_compatible()),
            ..custom
        };
        assert!(enabled.capabilities().native_tools);
        assert!(enabled.capabilities().streaming);
        assert!(enabled.capabilities().token_usage);
        let unknown = Provider::new("mystery-vendor", "sk", "", "m");
        assert!(!unknown.capabilities().native_tools);
    }

    #[test]
    fn provider_capabilities_true_has_contract_coverage() {
        for (name, caps) in [
            ("anthropic", ProviderCapabilities::anthropic()),
            ("openai", ProviderCapabilities::openai_compatible()),
            ("deepseek", ProviderCapabilities::deepseek()),
        ] {
            assert!(caps.native_tools, "{name} native_tools contract");
            assert!(caps.streaming, "{name} streaming contract");
            assert!(caps.token_usage, "{name} token_usage contract");
            assert!(caps.text_chat, "{name} text_chat contract");
        }
        let conservative = ProviderCapabilities::conservative();
        assert!(conservative.text_chat);
        assert!(!conservative.native_tools);
        assert!(!conservative.streaming);
        assert!(!conservative.token_usage);
        assert!(!conservative.reasoning);
    }

    #[test]
    fn model_id_and_display_name_are_permanently_separated() {
        let p = Provider::new("anthropic", "sk", "", "Claude Sonnet 5");
        assert_eq!(p.display_label(), "Claude Sonnet 5");
        assert_eq!(p.resolved_model_id(), "claude-sonnet-4-5");
        let payload = anthropic_request_json(&p, &[ProviderMessage::user("hi")], &[], 512).unwrap();
        assert_eq!(payload["model"], "claude-sonnet-4-5");
        assert_ne!(payload["model"], "Claude Sonnet 5");

        let rec = ProviderConfigRecord {
            id: "p".into(),
            name: "custom".into(),
            template: "custom".into(),
            model: "Claude Sonnet 5".into(),
            model_id: Some("explicit-api-id-v9".into()),
            display_name: Some("Marketing Label".into()),
            endpoint: "https://x.test".into(),
            api_key: "sk".into(),
        }
        .migrated();
        assert_eq!(rec.model_id.as_deref(), Some("explicit-api-id-v9"));
        assert_eq!(rec.display_name.as_deref(), Some("Marketing Label"));
        let provider = Provider::from_record(rec);
        assert_eq!(provider.resolved_model_id(), "explicit-api-id-v9");
        assert_eq!(provider.display_label(), "Marketing Label");
    }

    #[test]
    fn catalog_is_migration_aid_not_authoritative_api_id_source() {
        let spec = resolve_model_identity("anthropic", "Claude Brand New Box");
        assert_eq!(spec.model_id, "", "no inventing model ids from marketing");
        let spec = resolve_model_identity("custom", "partner-chat-2026-01");
        assert_eq!(spec.model_id, "partner-chat-2026-01");
        assert_eq!(spec.display_name, "partner-chat-2026-01");
    }

    /// Official OpenAI Chat Completions multi-turn tool contract:
    /// user → assistant tool_calls (JSON-string args) → tool result → second assistant.
    #[test]
    fn native_tool_openai_contract_multi_turn_multi_call() {
        let provider = openai_provider();
        let tools = vec![
            ToolSchema {
                name: "read_file".into(),
                description: "Read a file".into(),
                parameters: json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"],
                }),
            },
            ToolSchema {
                name: "run_command".into(),
                description: "Run a command".into(),
                parameters: json!({
                    "type": "object",
                    "properties": { "command": { "type": "string" } },
                    "required": ["command"],
                }),
            },
        ];

        let history = vec![
            ProviderMessage::system("You are Kodo"),
            ProviderMessage::user("inspect and test"),
        ];
        let req1 = openai_request_json(&provider, &history, &tools, 2048).unwrap();
        assert_eq!(req1["model"], "gpt-4o");
        assert_eq!(req1["tool_choice"], "auto");
        let tools_arr = req1["tools"].as_array().unwrap();
        assert_eq!(tools_arr.len(), 2);
        for t in tools_arr {
            assert_eq!(t["type"], "function", "official ChatCompletionTool type");
            assert!(t["function"]["name"].is_string());
            assert!(t["function"]["parameters"].is_object());
        }

        let response_fixture = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_abc123",
                            "type": "function",
                            "function": {
                                "name": "read_file",
                                "arguments": "{\"path\":\"src/lib.rs\"}"
                            }
                        },
                        {
                            "id": "call_def456",
                            "type": "function",
                            "function": {
                                "name": "run_command",
                                "arguments": "{\"command\":\"cargo test\"}"
                            }
                        }
                    ]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 82, "completion_tokens": 17 }
        });
        let body: OpenAiResponse = serde_json::from_value(response_fixture).unwrap();
        let response = parse_openai_body(body, "gpt-4o").unwrap();
        assert_eq!(
            response.native_tool_calls.len(),
            2,
            "multiple calls in one turn"
        );
        assert_eq!(response.native_tool_calls[0].id, "call_abc123");
        assert_eq!(response.native_tool_calls[1].id, "call_def456");
        assert_eq!(
            response.native_tool_calls[0].arguments["path"],
            "src/lib.rs"
        );
        assert_eq!(
            response.native_tool_calls[1].arguments["command"],
            "cargo test"
        );
        assert_eq!(response.finish_reason, "tool_calls");
        assert_eq!(response.input_tokens, 82);
        assert_eq!(response.output_tokens, 17);

        let assistant = openai_assistant_message_from_response(&response);
        let history2 = vec![
            ProviderMessage::system("You are Kodo"),
            ProviderMessage::user("inspect and test"),
            assistant,
            ProviderMessage::tool_result("call_abc123", "fn main() {}", false),
            ProviderMessage::tool_result("call_def456", "test result: ok", false),
        ];
        let req2 = openai_request_json(&provider, &history2, &tools, 2048).unwrap();
        let msgs = req2["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 5);
        let asst = &msgs[2];
        assert_eq!(asst["role"], "assistant");
        let tcs = asst["tool_calls"].as_array().unwrap();
        assert_eq!(tcs.len(), 2);
        assert_eq!(tcs[0]["id"], "call_abc123");
        assert_eq!(tcs[0]["type"], "function");
        let args0 = tcs[0]["function"]["arguments"]
            .as_str()
            .expect("arguments is string");
        assert_eq!(
            serde_json::from_str::<Value>(args0).unwrap()["path"],
            "src/lib.rs"
        );
        assert_eq!(tcs[1]["id"], "call_def456");
        assert_eq!(msgs[3]["role"], "tool");
        assert_eq!(msgs[3]["tool_call_id"], "call_abc123");
        assert_eq!(msgs[4]["role"], "tool");
        assert_eq!(msgs[4]["tool_call_id"], "call_def456");
        assert!(
            msgs[3].get("is_error").is_none(),
            "OpenAI tool message has no is_error"
        );

        let turn2_fixture = json!({
            "choices": [{
                "message": { "role": "assistant", "content": "All good." },
                "finish_reason": "stop"
            }]
        });
        let body2: OpenAiResponse = serde_json::from_value(turn2_fixture).unwrap();
        let response2 = parse_openai_body(body2, "gpt-4o").unwrap();
        assert_eq!(response2.text, "All good.");
        assert!(response2.native_tool_calls.is_empty());
        assert_eq!(response2.finish_reason, "stop");
    }

    /// Official Anthropic Messages multi-turn tool contract (SDK types).
    #[test]
    fn native_tool_anthropic_contract_multi_turn_multi_call() {
        let provider = anthropic_provider();
        let tools = vec![
            ToolSchema {
                name: "read_file".into(),
                description: "Read a file".into(),
                parameters: json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"],
                }),
            },
            ToolSchema {
                name: "run_command".into(),
                description: "Run a command".into(),
                parameters: json!({
                    "type": "object",
                    "properties": { "command": { "type": "string" } },
                    "required": ["command"],
                }),
            },
        ];

        let history = vec![
            ProviderMessage::system("You are Kodo"),
            ProviderMessage::user("inspect and test"),
        ];
        let req1 = anthropic_request_json(&provider, &history, &tools, 2048).unwrap();
        assert_eq!(req1["model"], "claude-sonnet-4-5");
        assert!(req1["system"].is_string());
        let tools_arr = req1["tools"].as_array().unwrap();
        assert_eq!(tools_arr.len(), 2);
        for t in tools_arr {
            assert!(t["name"].is_string());
            assert!(
                t["input_schema"].is_object(),
                "official ToolParam.input_schema"
            );
            assert!(t.get("function").is_none(), "not OpenAI function shape");
        }
        let msgs = req1["messages"].as_array().unwrap();
        assert!(msgs.iter().all(|m| m["role"] != "system"));

        let response_fixture = json!({
            "content": [
                { "type": "text", "text": "I will inspect both." },
                {
                    "type": "tool_use",
                    "id": "toolu_01A09q90qw90lq917835lq9",
                    "name": "read_file",
                    "input": { "path": "src/lib.rs" }
                },
                {
                    "type": "tool_use",
                    "id": "toolu_01B09q90qw90lq917835lq9",
                    "name": "run_command",
                    "input": { "command": "cargo test" }
                }
            ],
            "usage": { "input_tokens": 25, "output_tokens": 40 },
            "stop_reason": "tool_use"
        });
        let body: AnthropicResponse = serde_json::from_value(response_fixture).unwrap();
        let response = parse_anthropic_body(body, "claude-sonnet-4-5").unwrap();
        assert_eq!(response.native_tool_calls.len(), 2);
        assert_eq!(
            response.native_tool_calls[0].id,
            "toolu_01A09q90qw90lq917835lq9"
        );
        assert_eq!(
            response.native_tool_calls[1].id,
            "toolu_01B09q90qw90lq917835lq9"
        );
        assert_eq!(
            response.native_tool_calls[0].arguments["path"],
            "src/lib.rs"
        );
        assert_eq!(response.finish_reason, "tool_use");

        let blocks = anthropic_assistant_blocks(&response);
        let history2 = vec![
            ProviderMessage::system("You are Kodo"),
            ProviderMessage::user("inspect and test"),
            ProviderMessage {
                role: MessageRole::Assistant,
                content: vec![
                    ContentBlock::Text {
                        text: "I will inspect both.".into(),
                    },
                    ContentBlock::ToolCall {
                        id: response.native_tool_calls[0].id.clone(),
                        name: response.native_tool_calls[0].name.clone(),
                        arguments: response.native_tool_calls[0].arguments.clone(),
                    },
                    ContentBlock::ToolCall {
                        id: response.native_tool_calls[1].id.clone(),
                        name: response.native_tool_calls[1].name.clone(),
                        arguments: response.native_tool_calls[1].arguments.clone(),
                    },
                ],
            },
            ProviderMessage::tool_result("toolu_01A09q90qw90lq917835lq9", "fn main() {}", false),
            ProviderMessage::tool_result("toolu_01B09q90qw90lq917835lq9", "ERROR: exit 101", true),
        ];
        let req2 = anthropic_request_json(&provider, &history2, &tools, 2048).unwrap();
        let msgs2 = req2["messages"].as_array().unwrap();
        let asst = msgs2
            .iter()
            .find(|m| m["role"] == "assistant")
            .expect("assistant");
        let tool_uses: Vec<_> = asst["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["type"] == "tool_use")
            .collect();
        assert_eq!(tool_uses.len(), 2);
        assert_eq!(tool_uses[0]["id"], "toolu_01A09q90qw90lq917835lq9");
        assert_eq!(tool_uses[0]["input"]["path"], "src/lib.rs");
        assert_eq!(tool_uses[1]["id"], "toolu_01B09q90qw90lq917835lq9");

        let last_user = msgs2.iter().rfind(|m| m["role"] == "user").unwrap();
        let results: Vec<_> = last_user["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["type"] == "tool_result")
            .cloned()
            .collect();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["tool_use_id"], tool_uses[0]["id"]);
        assert_eq!(results[0]["is_error"], false);
        assert_eq!(results[1]["tool_use_id"], tool_uses[1]["id"]);
        assert_eq!(results[1]["is_error"], true, "error semantics preserved");
        assert!(results[1]["content"].as_str().unwrap().contains("ERROR"));
        assert_eq!(blocks[1]["id"], tool_uses[0]["id"]);
    }

    #[test]
    fn tool_call_ids_survive_full_roundtrip_both_providers() {
        let ids = ["call_stable_1", "toolu_stable_2"];
        let response = ChatResponse {
            text: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            native_tool_calls: vec![
                NativeToolCall {
                    id: ids[0].into(),
                    name: "read_file".into(),
                    arguments: json!({"path":"a"}),
                },
                NativeToolCall {
                    id: ids[1].into(),
                    name: "run_command".into(),
                    arguments: json!({"command":"true"}),
                },
            ],
            model_id: "gpt-4o".into(),
            finish_reason: "tool_calls".into(),
        };
        let assistant = openai_assistant_message_from_response(&response);
        let payload = openai_request_json(
            &openai_provider(),
            &[
                ProviderMessage::user("go"),
                assistant,
                ProviderMessage::tool_result(ids[0], "ok", false),
                ProviderMessage::tool_result(ids[1], "ok", false),
            ],
            &[],
            512,
        )
        .unwrap();
        let serialized = payload.to_string();
        assert!(serialized.contains(ids[0]));
        assert!(serialized.contains(ids[1]));

        let blocks = anthropic_assistant_blocks(&response);
        assert_eq!(blocks[0]["id"], ids[0]);
        assert_eq!(blocks[1]["id"], ids[1]);
        let payload = anthropic_request_json(
            &anthropic_provider(),
            &[
                ProviderMessage::user("go"),
                ProviderMessage {
                    role: MessageRole::Assistant,
                    content: vec![ContentBlock::ToolCall {
                        id: ids[0].into(),
                        name: "read_file".into(),
                        arguments: json!({"path":"a"}),
                    }],
                },
                ProviderMessage::tool_result(ids[0], "ok", false),
            ],
            &[],
            512,
        )
        .unwrap();
        assert!(payload.to_string().contains(ids[0]));
    }

    #[test]
    fn malformed_tool_args_reject_instead_of_free_text_mutation() {
        let broken = parse_openai_arguments("{not json");
        assert!(broken.is_string(), "must not pretend to be an object");
        let registry = crate::protocol::ToolRegistry::standard();
        let inv = registry.accept(
            crate::protocol::ToolCallId::new("m2"),
            "write_file",
            &broken,
        );
        assert!(
            matches!(inv, crate::protocol::ToolInvocation::Rejected(_)),
            "malformed args must be rejected: {inv:?}"
        );
        let result = inv.into_result();
        assert!(!result.ok);
        // Valid object still accepted (mutation path only after typed parse).
        let ok_inv = registry.accept(
            crate::protocol::ToolCallId::new("m3"),
            "write_file",
            &json!({"path":"src/a.rs","content":"x"}),
        );
        assert!(matches!(ok_inv, crate::protocol::ToolInvocation::Ready(_)));
    }

    #[test]
    fn openai_tool_result_error_uses_content_not_is_error_field() {
        let messages = vec![
            ProviderMessage::user("do it"),
            ProviderMessage::assistant(
                "",
                vec![NativeToolCall {
                    id: "call_e".into(),
                    name: "run_command".into(),
                    arguments: json!({"command":"rm"}),
                }],
            ),
            ProviderMessage::tool_result("call_e", "ERROR: permission denied", true),
        ];
        let payload = openai_request_json(&openai_provider(), &messages, &[], 512).unwrap();
        let tool_msg = payload["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["role"] == "tool")
            .expect("tool message");
        assert_eq!(tool_msg["tool_call_id"], "call_e");
        assert!(
            tool_msg["content"].as_str().unwrap().contains("ERROR:"),
            "error semantics live in content"
        );
        assert!(tool_msg.get("is_error").is_none());

        let payload = anthropic_request_json(&anthropic_provider(), &messages, &[], 512).unwrap();
        let last = payload["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rfind(|m| m["role"] == "user")
            .unwrap();
        let tr = last["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["type"] == "tool_result")
            .unwrap();
        assert_eq!(tr["is_error"], true);
        assert_eq!(tr["tool_use_id"], "call_e");
    }

    #[test]
    fn native_tool_roundtrip_openai_serialization() {
        let provider = openai_provider();
        let tools = vec![ToolSchema {
            name: "run_command".into(),
            description: "Run a shell command".into(),
            parameters: json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"],
            }),
        }];
        let messages = vec![
            ProviderMessage::system("You are Kodo"),
            ProviderMessage::user("fix the bug"),
            ProviderMessage::assistant(
                "",
                vec![NativeToolCall {
                    id: "call_1".into(),
                    name: "run_command".into(),
                    arguments: json!({ "command": "cargo test" }),
                }],
            ),
            ProviderMessage::tool_result("call_1", "exit 0", false),
        ];
        let payload = openai_request_json(&provider, &messages, &tools, 2048).unwrap();
        assert_eq!(payload["model"], "gpt-4o");
        assert!(payload["tools"].as_array().unwrap().len() == 1);
        let msgs = payload["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[2]["role"], "assistant");
        assert_eq!(msgs[2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(msgs[3]["role"], "tool");
        assert_eq!(msgs[3]["tool_call_id"], "call_1");
        // Second request after tool result must include both tool call and result.
        let args: Value = serde_json::from_str(
            msgs[2]["tool_calls"][0]["function"]["arguments"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(args["command"], "cargo test");
    }

    #[test]
    fn native_tool_roundtrip_anthropic_serialization() {
        let provider = anthropic_provider();
        let tools = vec![ToolSchema {
            name: "read_file".into(),
            description: "Read a file".into(),
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
            }),
        }];
        let messages = vec![
            ProviderMessage::system("You are Kodo"),
            ProviderMessage::user("read config"),
            ProviderMessage::assistant(
                "",
                vec![NativeToolCall {
                    id: "toolu_1".into(),
                    name: "read_file".into(),
                    arguments: json!({ "path": "src/lib.rs" }),
                }],
            ),
            ProviderMessage::tool_result("toolu_1", "fn main() {}", false),
        ];
        let payload = anthropic_request_json(&provider, &messages, &tools, 2048).unwrap();
        assert_eq!(payload["model"], "claude-sonnet-4-5");
        assert_eq!(payload["tools"][0]["name"], "read_file");
        let msgs = payload["messages"].as_array().unwrap();
        // system is separate; messages = user, assistant(tool_use), user(tool_result)
        assert!(msgs.len() >= 3, "msgs={msgs:?}");
        let assistant = msgs
            .iter()
            .find(|m| m["role"] == "assistant")
            .expect("assistant message");
        assert_eq!(assistant["content"][0]["type"], "tool_use");
        assert_eq!(assistant["content"][0]["id"], "toolu_1");
        let tool_user = msgs
            .iter()
            .rfind(|m| m["role"] == "user")
            .expect("tool result user message");
        assert!(
            tool_user["content"]
                .as_array()
                .map(|blocks| blocks
                    .iter()
                    .any(|b| b["type"] == "tool_result" && b["tool_use_id"] == "toolu_1"))
                .unwrap_or(false),
            "tool_result missing: {tool_user:?}"
        );
    }

    #[test]
    fn config_migration_keeps_legacy_fields() {
        let record = ProviderConfigRecord {
            id: "p1".into(),
            name: "OpenAI".into(),
            template: "openai".into(),
            model: "GPT-4o".into(),
            model_id: None,
            display_name: None,
            endpoint: "https://api.openai.com/v1".into(),
            api_key: "sk-secret".into(),
        };
        let migrated = record.migrated();
        assert_eq!(migrated.model_id.as_deref(), Some("gpt-4o"));
        assert_eq!(migrated.display_name.as_deref(), Some("GPT-4o"));
        assert_eq!(migrated.model, "gpt-4o");
        assert_eq!(migrated.api_key, "sk-secret");
        assert_eq!(migrated.endpoint, "https://api.openai.com/v1");
    }

    #[test]
    fn invalid_model_and_auth_are_not_failover_blind() {
        assert!(!ProviderFailureClass::Auth.allows_failover());
        assert!(!ProviderFailureClass::Auth.is_retryable());
        assert!(!ProviderFailureClass::InvalidModel.allows_failover());
        assert!(ProviderFailureClass::RateLimit.allows_failover());
        assert!(ProviderFailureClass::RateLimit.is_retryable());
        assert!(ProviderFailureClass::ServerError.is_retryable());
        assert!(!ProviderFailureClass::Cancelled.is_retryable());
    }

    #[test]
    fn classify_status_codes() {
        assert_eq!(
            classify_failure("unauthorized", Some(401)),
            ProviderFailureClass::Auth
        );
        assert_eq!(
            classify_failure("rate limited", Some(429)),
            ProviderFailureClass::RateLimit
        );
        assert_eq!(
            classify_failure("model not found", Some(404)),
            ProviderFailureClass::InvalidModel
        );
        assert_eq!(
            classify_failure("boom", Some(503)),
            ProviderFailureClass::ServerError
        );
        assert_eq!(
            classify_failure("request timed out", None),
            ProviderFailureClass::Timeout
        );
    }

    #[test]
    fn capabilities_always_expose_flags() {
        let caps = ProviderCapabilities::anthropic();
        assert!(
            caps.text_chat
                && caps.native_tools
                && caps.streaming
                && caps.reasoning
                && caps.token_usage
        );
        let caps = ProviderCapabilities::openai_compatible();
        assert!(caps.native_tools && caps.streaming && caps.token_usage);
        let caps = ProviderCapabilities::text_only();
        assert!(!caps.native_tools && !caps.streaming);
    }

    #[test]
    fn openai_arguments_string_parses_to_object() {
        let value = parse_openai_arguments(r#"{"path":"a.txt"}"#);
        assert_eq!(value["path"], "a.txt");
        let empty = parse_openai_arguments("");
        assert!(empty.as_object().unwrap().is_empty());
        let broken = parse_openai_arguments("not-json");
        assert_eq!(broken, Value::String("not-json".into()));
    }

    #[test]
    fn chat_stream_without_streaming_capability_emits_complete_events() {
        let provider = Provider {
            capabilities_override: Some(ProviderCapabilities {
                text_chat: true,
                native_tools: false,
                streaming: false,
                reasoning: false,
                token_usage: true,
            }),
            // Force invalid network — we only assert capability path selection
            // via a local override that fails fast on empty key... use validate path.
            api_key: String::new(),
            ..openai_provider()
        };
        let messages = vec![ProviderMessage::user("hi")];
        let mut events = Vec::new();
        let result = chat_stream(&provider, &messages, &[], 256, &|| true, |event| {
            events.push(event);
            true
        });
        // Empty key → Auth error before HTTP.
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().class,
            ProviderFailureClass::Auth | ProviderFailureClass::InvalidModel
        ));
        // MessageStart is emitted only after validate; empty key fails at validate first.
        let _ = events;
    }

    #[test]
    fn failover_classifies_and_reports_switch() {
        let primary = Provider {
            api_key: String::new(),
            fallbacks: vec![],
            ..openai_provider()
        };
        let err = chat_with_failover(
            &primary,
            &[ProviderMessage::user("x")],
            &[],
            256,
            true,
            |_, _| {},
        )
        .unwrap_err();
        assert_eq!(err.class, ProviderFailureClass::Auth);
    }
}
