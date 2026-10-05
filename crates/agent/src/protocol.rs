//! Typed tool protocol between the model and the agent loop.
//!
//! The agent loop only sees [`ToolInvocation`] values. Wire formats (strict
//! JSON fallback, native provider tool calls, XML-ish `function=`/`parameter=`
//! text tags, deprecated Markdown fences) are converted here so business
//! logic never walks raw `Value` maps.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable correlation id for one tool invocation and its result.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolCallId(pub String);

impl ToolCallId {
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        if raw.trim().is_empty() {
            return Self("tc_empty".to_owned());
        }
        Self(raw)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ToolCallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Canonical tool names known to the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolName {
    Search,
    ReadFile,
    WriteFile,
    RunCommand,
    ApplyPatch,
    ReplaceRange,
    CreateFile,
    DeleteFile,
    ListFiles,
    FindSymbol,
    FindReferences,
    ReadRange,
    WebSearch,
    /// Phase 0 second-pass — model-initiated ask-the-user round-trip.
    /// `question` is required; `options` is an optional list of quick-pick
    /// chips. The runtime emits a `SinkEvent::AskUser` and returns a
    /// placeholder ToolResult so the agent loop keeps going even when no
    /// UI is wired up.
    AskUser,
}

impl ToolName {
    pub fn label(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::RunCommand => "run_command",
            Self::ApplyPatch => "apply_patch",
            Self::ReplaceRange => "replace_range",
            Self::CreateFile => "create_file",
            Self::DeleteFile => "delete_file",
            Self::ListFiles => "list_files",
            Self::FindSymbol => "find_symbol",
            Self::FindReferences => "find_references",
            Self::ReadRange => "read_range",
            Self::WebSearch => "websearch",
            Self::AskUser => "ask_user",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "search" => Some(Self::Search),
            "read_file" | "read" => Some(Self::ReadFile),
            "write_file" | "write" => Some(Self::WriteFile),
            "run_command" | "bash" | "command" => Some(Self::RunCommand),
            "apply_patch" | "patch" => Some(Self::ApplyPatch),
            "replace_range" | "replace" => Some(Self::ReplaceRange),
            "create_file" | "create" => Some(Self::CreateFile),
            "delete_file" | "delete" => Some(Self::DeleteFile),
            "list_files" => Some(Self::ListFiles),
            "find_symbol" => Some(Self::FindSymbol),
            "find_references" => Some(Self::FindReferences),
            "read_range" => Some(Self::ReadRange),
            "websearch" | "search_web" | "web_search" => Some(Self::WebSearch),
            "ask_user" | "ask" | "ask-user" => Some(Self::AskUser),
            _ => None,
        }
    }

    /// True for file-mutating tools that require permission approval.
    pub fn is_mutation(self) -> bool {
        matches!(
            self,
            Self::WriteFile
                | Self::ApplyPatch
                | Self::ReplaceRange
                | Self::CreateFile
                | Self::DeleteFile
        )
    }
}

/// Typed arguments — never a free-form map past the protocol boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolArgs {
    Search {
        query: String,
    },
    ReadFile {
        path: String,
    },
    WriteFile {
        path: String,
        content: String,
    },
    RunCommand {
        command: String,
    },
    ApplyPatch {
        path: String,
        old: String,
        new: String,
        start_line: Option<usize>,
    },
    ReplaceRange {
        path: String,
        start_line: usize,
        end_line: usize,
        new_text: String,
    },
    CreateFile {
        path: String,
        content: String,
    },
    DeleteFile {
        path: String,
    },
    ListFiles {
        prefix: Option<String>,
    },
    FindSymbol {
        name: String,
    },
    FindReferences {
        name: String,
    },
    ReadRange {
        path: String,
        start_line: usize,
        end_line: usize,
    },
    WebSearch {
        query: String,
        max_results: Option<u32>,
    },
    /// Phase 0 second-pass — model-initiated ask-the-user round-trip.
    /// `question` is required; `options` is an optional quick-pick list.
    AskUser {
        question: String,
        options: Vec<String>,
    },
}

impl ToolArgs {
    /// File path this call targets, when it has one.
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::WriteFile { path, .. }
            | Self::ApplyPatch { path, .. }
            | Self::ReplaceRange { path, .. }
            | Self::CreateFile { path, .. }
            | Self::DeleteFile { path, .. }
            | Self::ReadFile { path, .. }
            | Self::ReadRange { path, .. } => Some(path.as_str()),
            _ => None,
        }
    }

    fn from_json(name: ToolName, args: &Value) -> Result<Self, ToolError> {
        match name {
            ToolName::Search => Ok(Self::Search {
                // Schema-aware rejection: message lists every required
                // field so the model fills them all next attempt instead
                // of guessing one at a time. (Phase 0 second-pass.)
                query: require_string_with_schema(args, "query", "search", &["query"])?,
            }),
            ToolName::ReadFile => Ok(Self::ReadFile {
                path: require_string(args, "path")?,
            }),
            ToolName::WriteFile => Ok(Self::WriteFile {
                path: require_string(args, "path")?,
                content: optional_string(args, "content")?.unwrap_or_default(),
            }),
            ToolName::RunCommand => Ok(Self::RunCommand {
                command: require_string(args, "command")?,
            }),
            ToolName::ApplyPatch => Ok(Self::ApplyPatch {
                path: require_string(args, "path")?,
                old: require_string(args, "old")?,
                new: optional_string(args, "new")?.unwrap_or_default(),
                start_line: optional_usize(args, "start_line")?,
            }),
            ToolName::ReplaceRange => Ok(Self::ReplaceRange {
                path: require_string(args, "path")?,
                start_line: require_usize(args, "start_line")?,
                end_line: require_usize(args, "end_line")?,
                new_text: optional_string(args, "new_text")?
                    .or_else(|| optional_string(args, "new").ok().flatten())
                    .unwrap_or_default(),
            }),
            ToolName::CreateFile => Ok(Self::CreateFile {
                path: require_string(args, "path")?,
                content: optional_string(args, "content")?.unwrap_or_default(),
            }),
            ToolName::DeleteFile => Ok(Self::DeleteFile {
                path: require_string(args, "path")?,
            }),
            ToolName::ListFiles => Ok(Self::ListFiles {
                prefix: optional_string(args, "prefix")?,
            }),
            ToolName::FindSymbol => Ok(Self::FindSymbol {
                name: require_string(args, "name")?,
            }),
            ToolName::FindReferences => Ok(Self::FindReferences {
                name: require_string(args, "name")?,
            }),
            ToolName::ReadRange => Ok(Self::ReadRange {
                path: require_string(args, "path")?,
                start_line: require_usize(args, "start_line")?,
                end_line: require_usize(args, "end_line")?,
            }),
            ToolName::WebSearch => Ok(Self::WebSearch {
                query: require_string(args, "query")?,
                max_results: optional_u32(args, "max_results")?,
            }),
            ToolName::AskUser => Ok(Self::AskUser {
                question: require_string_with_schema(
                    args,
                    "question",
                    "ask_user",
                    &["question"],
                )?,
                options: optional_string_array(args, "options")?,
            }),
        }
    }

    /// Compact human label used in observations / approval prompts.
    pub fn label(&self) -> String {
        match self {
            Self::Search { query } => query.clone(),
            Self::ReadFile { path } | Self::DeleteFile { path } => path.clone(),
            Self::WriteFile { path, .. } | Self::CreateFile { path, .. } => path.clone(),
            Self::RunCommand { command } => command.clone(),
            Self::ApplyPatch { path, .. } => path.clone(),
            Self::ReplaceRange {
                path,
                start_line,
                end_line,
                ..
            } => {
                format!("{path}:{start_line}-{end_line}")
            }
            Self::ListFiles { prefix } => {
                format!("list:{}", prefix.as_deref().unwrap_or("*"))
            }
            Self::FindSymbol { name } | Self::FindReferences { name } => name.clone(),
            Self::ReadRange {
                path,
                start_line,
                end_line,
            } => {
                format!("{path}:{start_line}-{end_line}")
            }
            Self::WebSearch { query, .. } => format!("web: {query}"),
            Self::AskUser { question, options } => {
                if options.is_empty() {
                    question.clone()
                } else {
                    format!("{question} (options: {})", options.join(", "))
                }
            }
        }
    }
}

fn require_string(args: &Value, key: &str) -> Result<String, ToolError> {
    match args {
        Value::Object(map) => match map.get(key) {
            Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
            Some(Value::String(_)) => Err(ToolError::invalid_args(format!(
                "`{key}` must be a non-empty string"
            ))),
            Some(other) => Err(ToolError::invalid_args(format!(
                "`{key}` must be a string, got {}",
                type_name(other)
            ))),
            None => Err(ToolError::invalid_args(format!(
                "missing required field `{key}`"
            ))),
        },
        Value::Null => Err(ToolError::invalid_args("arguments must be a JSON object")),
        other => Err(ToolError::invalid_args(format!(
            "arguments must be a JSON object, got {}",
            type_name(other)
        ))),
    }
}

/// Schema-aware variant of `require_string`: the rejection message lists
/// every required field for the tool, so the model only has to fail once
/// instead of guessing each missing field round-by-round.
///
/// The screenshot showed `search` rejected 5× in a row for missing `query`;
/// the rejection message `missing required field \`query\` — search
/// requires: query` lets the model fill all fields on the next attempt.
fn require_string_with_schema(
    args: &Value,
    key: &str,
    tool: &str,
    required_fields: &[&str],
) -> Result<String, ToolError> {
    match require_string(args, key) {
        Ok(v) => Ok(v),
        Err(e) => {
            // Only re-wrap the missing-field case; type / parse errors stay
            // terse so the model isn't drowned in schema text.
            if e.message.starts_with("missing required field") {
                let schema = required_fields.join(", ");
                Err(ToolError::invalid_args(format!(
                    "missing required field `{key}` — {tool} requires: {schema}"
                )))
            } else {
                Err(e)
            }
        }
    }
}

fn optional_string(args: &Value, key: &str) -> Result<Option<String>, ToolError> {
    match args {
        Value::Object(map) => match map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(other) => Err(ToolError::invalid_args(format!(
                "`{key}` must be a string, got {}",
                type_name(other)
            ))),
        },
        _ => Err(ToolError::invalid_args("arguments must be a JSON object")),
    }
}

fn require_usize(args: &Value, key: &str) -> Result<usize, ToolError> {
    match args {
        Value::Object(map) => match map.get(key) {
            Some(Value::Number(n)) if n.as_u64().is_some() => Ok(n.as_u64().unwrap() as usize),
            Some(Value::String(s)) => s.trim().parse::<usize>().map_err(|_| {
                ToolError::invalid_args(format!("`{key}` must be a positive integer"))
            }),
            Some(other) => Err(ToolError::invalid_args(format!(
                "`{key}` must be an integer, got {}",
                type_name(other)
            ))),
            None => Err(ToolError::invalid_args(format!(
                "missing required field `{key}`"
            ))),
        },
        _ => Err(ToolError::invalid_args("arguments must be a JSON object")),
    }
}

fn optional_string_array(args: &Value, key: &str) -> Result<Vec<String>, ToolError> {
    match args {
        Value::Object(map) => match map.get(key) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(arr)) => arr
                .iter()
                .enumerate()
                .map(|(i, v)| match v {
                    Value::String(s) => Ok(s.clone()),
                    other => Err(ToolError::invalid_args(format!(
                        "`{key}[{i}]` must be a string, got {}",
                        type_name(other)
                    ))),
                })
                .collect(),
            Some(other) => Err(ToolError::invalid_args(format!(
                "`{key}` must be an array of strings, got {}",
                type_name(other)
            ))),
        },
        _ => Err(ToolError::invalid_args("arguments must be a JSON object")),
    }
}

fn optional_usize(args: &Value, key: &str) -> Result<Option<usize>, ToolError> {
    match args {
        Value::Object(map) => match map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Number(n)) if n.as_u64().is_some() => {
                Ok(Some(n.as_u64().unwrap() as usize))
            }
            Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
            Some(Value::String(s)) => s
                .trim()
                .parse::<usize>()
                .map(Some)
                .map_err(|_| ToolError::invalid_args(format!("`{key}` must be an integer"))),
            Some(other) => Err(ToolError::invalid_args(format!(
                "`{key}` must be an integer, got {}",
                type_name(other)
            ))),
        },
        _ => Err(ToolError::invalid_args("arguments must be a JSON object")),
    }
}

fn optional_u32(args: &Value, key: &str) -> Result<Option<u32>, ToolError> {
    match args {
        Value::Object(map) => match map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Number(n)) if n.as_u64().is_some() => {
                let v = n.as_u64().unwrap();
                u32::try_from(v)
                    .map(Some)
                    .map_err(|_| ToolError::invalid_args(format!("`{key}` out of range")))
            }
            Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
            Some(Value::String(s)) => s
                .trim()
                .parse::<u32>()
                .map(Some)
                .map_err(|_| ToolError::invalid_args(format!("`{key}` must be an integer"))),
            Some(other) => Err(ToolError::invalid_args(format!(
                "`{key}` must be an integer, got {}",
                type_name(other)
            ))),
        },
        _ => Err(ToolError::invalid_args("arguments must be a JSON object")),
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Pulls the offered options out of the placeholder answer the agent hands
/// the model for an `ask_user` round-trip. The runtime prints them inline
/// as `(…问…建议选项：A / B / C；…)`; this helper recovers the list so the
/// evidence layer can record what was actually offered to the user.
/// Returns `Ok(None)` when the placeholder is the option-free variant, and
/// `Err(_)` only when the marker is present but malformed.
pub fn ask_user_options(placeholder: &str) -> Result<Option<Vec<String>>, String> {
    let Some(start) = placeholder.find("建议选项：") else {
        return Ok(None);
    };
    let after_marker = &placeholder[start + "建议选项：".len()..];
    // The options list ends at the next sentence delimiter — either the
    // semicolon that separates clauses or the closing parenthesis.
    let end = after_marker
        .find('；')
        .or_else(|| after_marker.find(')'))
        .unwrap_or(after_marker.len());
    let body = after_marker[..end].trim();
    if body.is_empty() {
        return Err("ask_user marker was present but no options followed".to_owned());
    }
    let parts: Vec<String> = body
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if parts.is_empty() {
        Err("ask_user marker was present but the list was empty".to_owned())
    } else {
        Ok(Some(parts))
    }
}

/// Structured tool failure returned to the model (and optionally the UI path).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolError {
    pub code: ToolErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorCode {
    InvalidArguments,
    UnknownTool,
    ExecutionFailed,
    PermissionDenied,
    PathEscape,
    Interrupted,
    /// Context/precondition failed — file must be re-read before retry.
    PatchConflict,
}

impl ToolError {
    pub fn new(code: ToolErrorCode, message: impl Into<String>) -> Self {
        // Redaction at construction — every consumer (model, UI, session)
        // sees a sanitized message without a second chance to leak.
        let message = crate::tools::redact_secrets(&message.into());
        Self { code, message }
    }

    pub fn invalid_args(message: impl Into<String>) -> Self {
        Self::new(ToolErrorCode::InvalidArguments, message)
    }

    pub fn unknown_tool(name: impl Into<String>) -> Self {
        Self::new(
            ToolErrorCode::UnknownTool,
            format!("unknown tool: {}", name.into()),
        )
    }

    pub fn execution(message: impl Into<String>) -> Self {
        Self::new(ToolErrorCode::ExecutionFailed, message)
    }

    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::new(ToolErrorCode::PermissionDenied, message)
    }

    pub fn path_escape(message: impl Into<String>) -> Self {
        Self::new(ToolErrorCode::PathEscape, message)
    }

    pub fn patch_conflict(message: impl Into<String>) -> Self {
        Self::new(ToolErrorCode::PatchConflict, message)
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

/// A validated tool invocation ready for execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: ToolName,
    pub args: ToolArgs,
}

/// A model-requested call that failed schema/registry validation.
/// Still produces a recoverable [`ToolResult`] for the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedCall {
    pub id: ToolCallId,
    /// Raw name as sent by the model (may be unknown).
    pub name: String,
    pub error: ToolError,
}

/// An external (MCP) tool advertised next to the built-in set.
///
/// Names are wire names (`mcp__<server>__<tool>`) and owned: they come from
/// runtime server config, not the `&'static str` builtin tables. The schema is
/// passed through to providers untouched — the server owns its shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// A call to an external (MCP) tool. Args are raw JSON: unlike [`ToolCall`]
/// there is no typed parse step — the server validates its own schema, so a
/// model turn can carry tools Kodo knows nothing about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCall {
    pub id: ToolCallId,
    /// Wire name `mcp__<server>__<tool>`, also the provider-visible name.
    pub name: String,
    pub args: Value,
}

impl ExternalCall {
    /// Compact args label for observation/result blocks.
    pub fn args_label(&self) -> String {
        self.args.to_string()
    }
}

/// One accepted or rejected call from a single model turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolInvocation {
    Ready(ToolCall),
    Rejected(RejectedCall),
    External(ExternalCall),
}

impl ToolInvocation {
    pub fn id(&self) -> &ToolCallId {
        match self {
            Self::Ready(call) => &call.id,
            Self::Rejected(reject) => &reject.id,
            Self::External(call) => &call.id,
        }
    }

    pub fn into_result(self) -> ToolResult {
        match self {
            Self::Ready(call) => ToolResult {
                id: call.id,
                name: call.name.label().to_owned(),
                input: call.args.label(),
                output: String::new(),
                ok: true,
                error: None,
            },
            Self::Rejected(reject) => ToolResult::from_rejection(&reject),
            Self::External(call) => {
                // Redacting constructor: external args are opaque and may
                // carry secrets (tokens in tool arguments, for example).
                let input = call.args_label();
                ToolResult::success(call.id, call.name, input, "")
            }
        }
    }
}

/// Outcome of running one tool, fed back to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub id: ToolCallId,
    pub name: String,
    /// Args label (query / path / command) for the observation block.
    pub input: String,
    pub output: String,
    pub ok: bool,
    pub error: Option<ToolError>,
}

impl ToolResult {
    pub fn success(
        id: ToolCallId,
        name: impl Into<String>,
        input: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        // Redact both sides before they can reach model/history/session.
        let input = crate::tools::redact_secrets(&input.into());
        let output = crate::tools::redact_secrets(&output.into());
        Self {
            id,
            name: name.into(),
            input,
            output,
            ok: true,
            error: None,
        }
    }

    pub fn failure(
        id: ToolCallId,
        name: impl Into<String>,
        input: impl Into<String>,
        error: ToolError,
    ) -> Self {
        let input = crate::tools::redact_secrets(&input.into());
        let output = error.message.clone();
        Self {
            id,
            name: name.into(),
            input,
            output,
            ok: false,
            error: Some(error),
        }
    }

    pub fn from_rejection(reject: &RejectedCall) -> Self {
        Self {
            id: reject.id.clone(),
            name: if reject.name.trim().is_empty() {
                "unknown".to_owned()
            } else {
                reject.name.clone()
            },
            input: String::new(),
            output: reject.error.message.clone(),
            ok: false,
            error: Some(reject.error.clone()),
        }
    }

    pub fn code(&self) -> Option<ToolErrorCode> {
        self.error.as_ref().map(|e| e.code)
    }
}

/// Schema + metadata advertised to the model (JSON fallback and native tools).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

/// The fixed set of tools this agent exposes, plus any external (MCP) tools
/// discovered for this run.
#[derive(Debug, Clone, Default)]
pub struct ToolRegistry {
    defs: Vec<ToolDefinition>,
    externals: Vec<ExternalToolDef>,
}

impl ToolRegistry {
    pub fn standard() -> Self {
        Self {
            defs: vec![
                ToolDefinition {
                    name: "search",
                    description: "Search project files for a text pattern. Returns matching file paths.",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "query": { "type": "string", "description": "Text pattern to search for" }
                        },
                        "required": ["query"],
                        "additionalProperties": false
                    }),
                },
                ToolDefinition {
                    name: "read_file",
                    description: "Read a UTF-8 file inside the project. Paths are relative to the project root.",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "Relative path inside the project" }
                        },
                        "required": ["path"],
                        "additionalProperties": false
                    }),
                },
                ToolDefinition {
                    name: "write_file",
                    description: "Write a UTF-8 file inside the project. Creates parent directories as needed.",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "Relative path inside the project" },
                            "content": { "type": "string", "description": "Full file content" }
                        },
                        "required": ["path", "content"],
                        "additionalProperties": false
                    }),
                },
                ToolDefinition {
                    name: "run_command",
                    description: "Run a single shell command in the project root. Requires approval when permission mode is ask/auto.",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "command": { "type": "string", "description": "Shell command to run" }
                        },
                        "required": ["command"],
                        "additionalProperties": false
                    }),
                },
                ToolDefinition {
                    name: "apply_patch",
                    description: "Replace an exact context string in a file. Fails with patch_conflict if context is missing or ambiguous — re-read and retry. Prefer this over full-file rewrites.",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "Relative path inside the project" },
                            "old": { "type": "string", "description": "Exact current context to replace (must match once)" },
                            "new": { "type": "string", "description": "Replacement text" },
                            "start_line": { "type": "integer", "description": "Optional 1-based expected line to disambiguate repeated context" }
                        },
                        "required": ["path", "old", "new"],
                        "additionalProperties": false
                    }),
                },
                ToolDefinition {
                    name: "replace_range",
                    description: "Replace inclusive 1-based lines [start_line, end_line] in a file.",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "start_line": { "type": "integer" },
                            "end_line": { "type": "integer" },
                            "new_text": { "type": "string", "description": "Replacement lines (empty deletes the range)" }
                        },
                        "required": ["path", "start_line", "end_line"],
                        "additionalProperties": false
                    }),
                },
                ToolDefinition {
                    name: "create_file",
                    description: "Create a new file (fails if it exists).",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "content": { "type": "string" }
                        },
                        "required": ["path", "content"],
                        "additionalProperties": false
                    }),
                },
                ToolDefinition {
                    name: "delete_file",
                    description: "Delete a project file. Requires approval under ask/auto permission.",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" }
                        },
                        "required": ["path"],
                        "additionalProperties": false
                    }),
                },
                ToolDefinition {
                    name: "list_files",
                    description: "List project files from the repo map (fast orientation). Optional path prefix filter.",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "prefix": { "type": "string", "description": "Optional path prefix like `crates/`" }
                        },
                        "additionalProperties": false
                    }),
                },
                ToolDefinition {
                    name: "find_symbol",
                    description: "Find symbol definitions (fn/struct/enum/class/…) in the RepoMap index: returns symbol, kind, file, line/range. Prefer this before reading files.",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "Symbol name or substring" }
                        },
                        "required": ["name"],
                        "additionalProperties": false
                    }),
                },
                ToolDefinition {
                    name: "find_references",
                    description: "Find references to a symbol with confidence=high (word-boundary) or confidence=lexical (fallback). Definition lines are marked. Do not assume precision when labeled lexical.",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "Symbol name" }
                        },
                        "required": ["name"],
                        "additionalProperties": false
                    }),
                },
                ToolDefinition {
                    name: "read_range",
                    description: "Read inclusive 1-based lines [start_line, end_line] of a project file (cheaper than full read).",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "start_line": { "type": "integer" },
                            "end_line": { "type": "integer" }
                        },
                        "required": ["path", "start_line", "end_line"],
                        "additionalProperties": false
                    }),
                },
                ToolDefinition {
                    name: "websearch",
                    description: "Search the public web via DuckDuckGo HTML (free, no API key). Returns up to N results with title, URL, and snippet. Use for fresh facts, library docs, or anything not in the project tree.",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "query": { "type": "string", "description": "Search query (kept short and specific)" },
                            "max_results": { "type": "integer", "description": "Optional cap (default 8, max 20)" }
                        },
                        "required": ["query"],
                        "additionalProperties": false
                    }),
                },
            ],
            externals: Vec::new(),
        }
    }

    /// The mode × project matrix (docs/design/DESIGN.md §11), as the base
    /// registry every skill may only narrow further:
    ///
    /// - no project → websearch only (pure conversation; the only
    ///   project-independent tool — lets the model answer fresh-fact questions
    ///   without a workspace);
    /// - Work + project → file tools, search, run_command, and websearch —
    ///   never the symbol tools;
    /// - Code + project → the full coding agent, symbol tools included,
    ///   websearch included.
    ///
    /// Keeping the matrix here means tool availability never depends on which
    /// skill files happen to be embedded.
    pub fn for_mode(mode: crate::AgentMode, has_project: bool) -> Self {
        if !has_project {
            return Self::standard()
                .filtered(|name| matches!(name, "websearch"));
        }
        match mode {
            crate::AgentMode::Code => Self::standard(),
            crate::AgentMode::Work => Self::standard()
                .filtered(|name| !matches!(name, "find_symbol" | "find_references" | "read_range")),
        }
    }

    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.defs
    }

    pub fn external_definitions(&self) -> &[ExternalToolDef] {
        &self.externals
    }

    /// Attach the external (MCP) tools discovered for this run.
    pub fn with_externals(mut self, externals: Vec<ExternalToolDef>) -> Self {
        self.externals = externals;
        self
    }

    /// Keep only **builtin** definitions whose name passes `keep` (skill
    /// allowlists). External tools always survive: a skill allowlist narrows
    /// builtins only — `mcp__*` labels pass the skill gate unconditionally and
    /// rely on Permission instead (see `SkillSpec::allows_label`).
    pub fn filtered<F: Fn(&str) -> bool>(&self, keep: F) -> Self {
        Self {
            defs: self.defs.iter().filter(|d| keep(d.name)).cloned().collect(),
            externals: self.externals.clone(),
        }
    }

    pub fn find(&self, name: &str) -> Option<&ToolDefinition> {
        self.defs.iter().find(|d| d.name == name)
    }

    pub fn find_external(&self, wire_name: &str) -> Option<&ExternalToolDef> {
        self.externals.iter().find(|e| e.name == wire_name)
    }

    pub fn contains_name(&self, name: &str) -> bool {
        ToolName::parse(name).is_some_and(|n| self.find(n.label()).is_some())
            || self.find_external(name).is_some()
    }

    /// Validate a raw wire call against the registry → typed args.
    pub fn accept(&self, id: ToolCallId, raw_name: &str, args: &Value) -> ToolInvocation {
        let trimmed = raw_name.trim();
        let Some(name) = ToolName::parse(trimmed) else {
            // Not a builtin alias — an external (MCP) wire name may still
            // resolve. Its args stay raw JSON: the server owns its schema.
            if self.find_external(trimmed).is_some() {
                return ToolInvocation::External(ExternalCall {
                    id,
                    name: trimmed.to_owned(),
                    args: args.clone(),
                });
            }
            return ToolInvocation::Rejected(RejectedCall {
                id,
                name: trimmed.to_owned(),
                error: ToolError::unknown_tool(trimmed),
            });
        };
        if self.find(name.label()).is_none() {
            return ToolInvocation::Rejected(RejectedCall {
                id,
                name: trimmed.to_owned(),
                error: ToolError::unknown_tool(trimmed),
            });
        }
        match ToolArgs::from_json(name, args) {
            Ok(typed) => ToolInvocation::Ready(ToolCall {
                id,
                name,
                args: typed,
            }),
            Err(error) => ToolInvocation::Rejected(RejectedCall {
                id,
                name: name.label().to_owned(),
                error,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Wire schemas (JSON fallback + native bridge). `Value` exists only here.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct WireToolCall {
    #[serde(default)]
    id: Option<String>,
    name: String,
    #[serde(default)]
    arguments: Value,
    /// OpenAI native style sometimes nests under `args`.
    #[serde(default)]
    args: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WireToolEnvelope {
    Wrapper { tool_calls: Vec<WireToolCall> },
    List(Vec<WireToolCall>),
    Single(Box<WireToolCall>),
}

/// What a single model turn asked the agent to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelTurn {
    /// No tool calls — final (or intermediate) prose answer.
    Final { text: String },
    /// Zero or more tool calls (accepted or rejected).
    Tools { calls: Vec<ToolInvocation> },
}

/// Parse a strict JSON tool protocol from model text.
///
/// Accepted shapes:
/// - `{"tool_calls":[…]}`
/// - `[ … ]`
/// - a single `{"id"?, "name", "arguments"}` object
///
/// Returns `Err` when the text is not that protocol (caller may fall back).
pub fn parse_json_protocol(text: &str) -> Result<Vec<WireToolCall>, String> {
    let trimmed = strip_code_fence_json(text);
    let value: Value = serde_json::from_str(trimmed).map_err(|e| e.to_string())?;
    match serde_json::from_value::<WireToolEnvelope>(value) {
        Ok(WireToolEnvelope::Wrapper { tool_calls }) => {
            if tool_calls.is_empty() {
                Err("tool_calls is empty".to_owned())
            } else {
                Ok(tool_calls)
            }
        }
        Ok(WireToolEnvelope::List(list)) => {
            if list.is_empty() {
                Err("tool_calls list is empty".to_owned())
            } else {
                Ok(list)
            }
        }
        Ok(WireToolEnvelope::Single(call)) => Ok(vec![*call]),
        Err(e) => Err(e.to_string()),
    }
}

fn strip_code_fence_json(text: &str) -> &str {
    let trimmed = text.trim();
    let without_open = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```JSON"))
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let without_close = without_open.strip_suffix("```").unwrap_or(without_open);
    without_close.trim()
}

fn next_id(provided: Option<String>, index: usize) -> ToolCallId {
    let raw = provided.unwrap_or_else(|| format!("tc_{index}"));
    ToolCallId::new(raw)
}

/// Convert wire calls into invocations using the registry (never panics).
pub fn invocations_from_wire(
    registry: &ToolRegistry,
    wire: Vec<WireToolCall>,
) -> Vec<ToolInvocation> {
    wire.into_iter()
        .enumerate()
        .map(|(index, call)| {
            let id = next_id(call.id, index);
            let args = match call.args {
                Some(args) => args,
                None => call.arguments,
            };
            registry.accept(id, &call.name, &args)
        })
        .collect()
}

/// True when the text carries the XML-ish text tool protocol
/// (`<tool_call>` blocks whose function tag opens as `<function=NAME`).
pub fn looks_like_tag_tool_protocol(text: &str) -> bool {
    text.contains("<function=") && (text.contains("</function>") || text.contains("<parameter="))
}

/// Parse the XML-ish text tool protocol some providers emit in prose mode.
///
/// Shape (angle brackets literal on the wire):
/// `<tool_call><function=search><parameter=query>--kodo-canvas</parameter></function></tool_call>`
///
/// Open tags put `name=value` in the tag body (`<function=search` opens the function
/// named search, `<parameter=query` opens its query parameter); closers are plain
/// `</function>` / `</parameter>` / `</tool_call>`. Several calls concatenate in one message.
/// Parameter values arrive as text; the args parser already accepts digit
/// strings for numeric fields. Calls are validated through the registry,
/// exactly like the JSON protocol - surrounding prose is ignored here and
/// stripped before display.
pub fn parse_tag_invocations(text: &str, registry: &ToolRegistry) -> Vec<ToolInvocation> {
    let mut calls = Vec::new();
    let mut rest = text;
    let mut index = 0usize;
    while let Some(pos) = rest.find("<function=") {
        let region = &rest[pos..];
        let Some(end) = region.find("</function>") else {
            break;
        };
        let body_start = region.find('>').map(|gap| gap + 1).unwrap_or(0).min(end);
        let open_tag = &region[..body_start];
        let name = open_tag
            .trim_start_matches('<')
            .strip_prefix("function=")
            .unwrap_or("")
            .trim_end_matches('>')
            .trim()
            .trim_matches('"')
            .to_owned();
        let body = &region[body_start..end];
        let mut map = serde_json::Map::new();
        let mut param_rest = body;
        while let Some(p) = param_rest.find("<parameter=") {
            let p_region = &param_rest[p..];
            let Some(p_end) = p_region.find("</parameter>") else {
                break;
            };
            let p_body_start = p_region
                .find('>')
                .map(|gap| gap + 1)
                .unwrap_or(0)
                .min(p_end);
            let p_open = &p_region[..p_body_start];
            let key = p_open
                .trim_start_matches('<')
                .strip_prefix("parameter=")
                .unwrap_or("")
                .trim_end_matches('>')
                .trim()
                .trim_matches('"')
                .to_owned();
            let value = p_region[p_body_start..p_end].trim().to_owned();
            if !key.is_empty() {
                map.insert(key, Value::String(value));
            }
            param_rest = &param_rest[p + p_end + "</parameter>".len()..];
        }
        let id = ToolCallId::new(format!("tag_{index}"));
        if name.is_empty() {
            calls.push(ToolInvocation::Rejected(RejectedCall {
                id,
                name: "tag_protocol".to_owned(),
                error: ToolError::invalid_args("tag tool call missing function name"),
            }));
        } else {
            calls.push(registry.accept(id, &name, &Value::Object(map)));
        }
        index += 1;
        rest = &rest[pos + end + "</function>".len()..];
    }
    calls
}

/// Inline reasoning blocks some models emit in `content` instead of
/// `reasoning_content`: ` <think>…` and ` <thinking>…`. Thinking is not the
/// reply — it is stripped before tool detection and never shown as the
/// answer (2026-10-02: a leading block hid the `{"tool_calls":…}` that
/// followed it, the call was eaten as debris, and raw thinking rendered as
/// the reply).
const THINK_OPENS: [&str; 2] = ["<think>", "<thinking>"];
const THINK_CLOSES: [&str; 2] = ["</think>", "</thinking>"];

fn earliest<'a>(text: &str, tags: &'a [&str]) -> Option<(usize, &'a str)> {
    tags.iter()
        .filter_map(|t| text.find(*t).map(|pos| (pos, *t)))
        .min_by_key(|(pos, _)| *pos)
}

/// Strip inline reasoning blocks from a complete model message. Returns the
/// prose remainder and whether a block was seen. An unterminated block
/// swallows the rest of the text — the model stopped mid-thinking, so the
/// remainder is thinking too. Leading whitespace before the first visible
/// character is also dropped — reasoning models tend to emit `<think>` after
/// a stray space or newline, and we don't want that bleeding into the reply.
pub fn strip_think_blocks(text: &str) -> (String, bool) {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut saw = false;
    loop {
        let Some((pos, open)) = earliest(rest, &THINK_OPENS) else {
            out.push_str(rest);
            if saw {
                let trimmed = out.trim_start().to_owned();
                return (trimmed, saw);
            }
            return (out, saw);
        };
        saw = true;
        out.push_str(&rest[..pos]);
        let after = &rest[pos + open.len()..];
        match earliest(after, &THINK_CLOSES) {
            Some((end, close)) => rest = &after[end + close.len()..],
            None => {
                let trimmed = out.trim_start().to_owned();
                return (trimmed, saw);
            }
        }
    }
}

/// Streaming counterpart of [`strip_think_blocks`]. Tags split across deltas
/// are held back until the next delta decides — a partial `<think` is never
/// emitted as answer text.
#[derive(Default)]
pub struct ThinkStripper {
    hold: String,
    in_think: bool,
    saw_think: bool,
    /// True once a non-whitespace visible character has been released. Until
    /// then we trim leading whitespace off the released text, so a stray
    /// space or newline before `<think>` doesn't bleed into the answer.
    released_visible: bool,
}

impl ThinkStripper {
    /// Feed one content delta; returns the visible answer text it unlocked.
    pub fn push(&mut self, delta: &str) -> String {
        self.hold.push_str(delta);
        let mut newly = String::new();
        loop {
            if self.in_think {
                match earliest(&self.hold, &THINK_CLOSES) {
                    Some((pos, close)) => {
                        self.hold.drain(..pos + close.len());
                        self.in_think = false;
                    }
                    None => {
                        // Keep only a suffix that could complete a close tag.
                        let keep = partial_tag_tail(&self.hold, &THINK_CLOSES);
                        self.hold.drain(..self.hold.len() - keep);
                        break;
                    }
                }
            } else {
                match earliest(&self.hold, &THINK_OPENS) {
                    Some((pos, open)) => {
                        newly.push_str(&self.hold[..pos]);
                        self.hold.drain(..pos + open.len());
                        self.in_think = true;
                        self.saw_think = true;
                    }
                    None => {
                        let keep = partial_tag_tail(&self.hold, &THINK_OPENS);
                        let emit = self.hold.len() - keep;
                        newly.push_str(&self.hold[..emit]);
                        self.hold.drain(..emit);
                        break;
                    }
                }
            }
        }
        if !self.released_visible {
            let trimmed: String = newly.trim_start().into();
            if !trimmed.is_empty() {
                self.released_visible = true;
            }
            return trimmed;
        }
        newly
    }

    /// End of stream: outside a block the hold is plain text; inside one the
    /// model stopped mid-thinking and the remainder stays hidden.
    pub fn finish(&mut self) -> String {
        if self.in_think {
            self.hold.clear();
            String::new()
        } else {
            std::mem::take(&mut self.hold)
        }
    }

    pub fn saw_think(&self) -> bool {
        self.saw_think
    }
}

/// Length of the longest suffix of `buf` that is a proper prefix of one of
/// `tags` — exactly the part that must wait for the next delta.
fn partial_tag_tail(buf: &str, tags: &[&str]) -> usize {
    let max = tags.iter().map(|t| t.len() - 1).max().unwrap_or(0);
    for n in (1..=max.min(buf.len())).rev() {
        let cut = buf.len() - n;
        if !buf.is_char_boundary(cut) {
            continue;
        }
        let suffix = &buf[cut..];
        if tags.iter().any(|t| suffix.len() < t.len() && t.starts_with(suffix)) {
            return n;
        }
    }
    0
}

/// Full text → model turn: JSON protocol, then tag protocol, then
/// deprecated fences.
///
/// Malformed JSON that is *almost* a tool protocol falls through to the tag
/// and fence parsers; if none yields tools, the original text is the final
/// answer.
pub fn parse_model_turn(text: &str, registry: &ToolRegistry) -> ModelTurn {
    // Inline reasoning blocks must not mask the tool protocol that follows
    // them — strip first so ` <think>…{"tool_calls":…}` is still a call.
    let (text, _saw_think) = strip_think_blocks(text);
    let text = text.as_str();
    // 1) Strict JSON fallback protocol
    if looks_like_json_tool_protocol(text) {
        match parse_json_protocol(text) {
            Ok(wire) => {
                return ModelTurn::Tools {
                    calls: invocations_from_wire(registry, wire),
                };
            }
            Err(_) => {
                // Malformed tool JSON - recover via tag/fence parsers, else treat as text.
            }
        }
    }

    // 2) XML-ish text tool protocol (`<tool_call>` + `<function=NAME`)
    if looks_like_tag_tool_protocol(text) {
        let tag_calls = parse_tag_invocations(text, registry);
        if !tag_calls.is_empty() {
            return ModelTurn::Tools { calls: tag_calls };
        }
    }

    // 3) Deprecated Markdown fence compatibility layer
    let fence_calls = parse_fence_invocations(text, registry);
    if !fence_calls.is_empty() {
        return ModelTurn::Tools { calls: fence_calls };
    }

    // 4) If the text was clearly an attempted JSON tool call that failed hard,
    // surface a structured error instead of dropping the request on the floor.
    if looks_like_json_tool_protocol(text) {
        let message = parse_json_protocol(text).unwrap_err();
        return ModelTurn::Tools {
            calls: vec![ToolInvocation::Rejected(RejectedCall {
                id: ToolCallId::new("tc_json"),
                name: "json_protocol".to_owned(),
                error: ToolError::invalid_args(format!("malformed tool JSON: {message}")),
            })],
        };
    }

    ModelTurn::Final {
        text: text.to_owned(),
    }
}

fn looks_like_json_tool_protocol(text: &str) -> bool {
    let body = strip_code_fence_json(text);
    let t = body.trim_start();
    t.starts_with('{') || t.starts_with('[')
}

// ---------------------------------------------------------------------------
// Deprecated Markdown fence parser — short-term compatibility only.
// ---------------------------------------------------------------------------

/// **Deprecated:** Markdown fence tool parser. Prefer [`parse_model_turn`].
#[deprecated(
    since = "0.1.0",
    note = "use parse_model_turn; fences are a temporary compatibility layer"
)]
#[allow(dead_code)]
pub fn extract_tool_calls_legacy(text: &str) -> Vec<ToolCall> {
    parse_fence_invocations(text, &ToolRegistry::standard())
        .into_iter()
        .filter_map(|inv| match inv {
            ToolInvocation::Ready(call) => Some(call),
            ToolInvocation::Rejected(_) | ToolInvocation::External(_) => None,
        })
        .collect()
}

/// Parse ```search / ```read / ```bash / ```write fences in document order.
pub fn parse_fence_invocations(text: &str, registry: &ToolRegistry) -> Vec<ToolInvocation> {
    let needles = [
        ("```search", "search"),
        ("```read", "read_file"),
        ("```bash", "run_command"),
        ("```write", "write_file"),
        ("```run_command", "run_command"),
        ("```read_file", "read_file"),
        ("```write_file", "write_file"),
    ];
    let mut marks: Vec<(usize, &'static str, usize)> = Vec::new();
    for (needle, name) in needles {
        let mut from = 0;
        while let Some(pos) = text[from..].find(needle) {
            let abs = from + pos;
            marks.push((abs, name, needle.len()));
            from = abs + needle.len();
        }
    }
    marks.sort_by_key(|(pos, _, _)| *pos);
    // Prefer the longest needle when two match at the same position
    // (```read vs ```read_file).
    marks.dedup_by(|a, b| {
        if a.0 == b.0 {
            if a.2 < b.2 {
                *a = *b;
            }
            true
        } else {
            false
        }
    });

    let mut calls = Vec::new();
    for (index, &(pos, name, open_len)) in marks.iter().enumerate() {
        let body_start = pos + open_len;
        let region_end = marks
            .get(index + 1)
            .map(|(next, _, _)| (*next).min(text.len()))
            .unwrap_or(text.len());
        if body_start > text.len() {
            continue;
        }
        let region = &text[body_start.min(text.len())..region_end.max(body_start.min(text.len()))];
        let block = match region.find("```") {
            Some(end) => &region[..end],
            None => region,
        };
        let id = ToolCallId::new(format!("fence_{}", calls.len()));
        let invocation = match name {
            "write_file" => match parse_write_block(block) {
                Some((path, content)) => registry.accept(
                    id,
                    name,
                    &serde_json::json!({ "path": path, "content": content }),
                ),
                None => ToolInvocation::Rejected(RejectedCall {
                    id,
                    name: name.to_owned(),
                    error: ToolError::invalid_args("write fence missing path/content"),
                }),
            },
            "read_file" => {
                let path = parse_path_block(block);
                if path.is_empty() {
                    ToolInvocation::Rejected(RejectedCall {
                        id,
                        name: name.to_owned(),
                        error: ToolError::invalid_args("read fence missing path"),
                    })
                } else {
                    registry.accept(id, name, &serde_json::json!({ "path": path }))
                }
            }
            "run_command" => {
                let command = block.trim().to_owned();
                if command.is_empty() {
                    ToolInvocation::Rejected(RejectedCall {
                        id,
                        name: name.to_owned(),
                        error: ToolError::invalid_args("command fence is empty"),
                    })
                } else {
                    registry.accept(id, name, &serde_json::json!({ "command": command }))
                }
            }
            // search
            other => {
                let query = block.trim().to_owned();
                if query.is_empty() {
                    ToolInvocation::Rejected(RejectedCall {
                        id,
                        name: other.to_owned(),
                        error: ToolError::invalid_args("search fence is empty"),
                    })
                } else {
                    registry.accept(id, other, &serde_json::json!({ "query": query }))
                }
            }
        };
        calls.push(invocation);
    }
    calls
}

fn parse_write_block(block: &str) -> Option<(String, String)> {
    let mut path = String::new();
    let mut content_lines: Vec<&str> = Vec::new();
    let mut in_body = false;
    for line in block.lines() {
        if in_body {
            content_lines.push(line);
            continue;
        }
        let trimmed = line.trim();
        if trimmed == "---" || trimmed == "===" {
            in_body = true;
            continue;
        }
        if let Some(p) = trimmed.strip_prefix("path:") {
            path = p.trim().trim_matches('"').to_owned();
        }
    }
    if path.is_empty() {
        for line in block.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with("path:") {
                continue;
            }
            if t == "---" {
                break;
            }
            path = t.to_owned();
            break;
        }
    }
    if path.is_empty() {
        return None;
    }
    if !in_body {
        let mut seen_path = false;
        let mut lines = Vec::new();
        for line in block.lines() {
            let t = line.trim();
            if !seen_path {
                if t.is_empty() {
                    continue;
                }
                if t.strip_prefix("path:").is_some() {
                    seen_path = true;
                    continue;
                }
                if t == path {
                    seen_path = true;
                    continue;
                }
            }
            lines.push(line);
        }
        content_lines = lines;
    }
    let mut content = content_lines.join("\n");
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    Some((path, content))
}

fn parse_path_block(block: &str) -> String {
    for line in block.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        return t
            .strip_prefix("path:")
            .map(|p| p.trim().trim_matches('"').to_owned())
            .unwrap_or_else(|| t.to_owned());
    }
    String::new()
}

/// Formats tool outcomes as a user message block for text-protocol models.
pub fn format_observations(results: &[ToolResult]) -> String {
    let mut out = String::from("<observations>\n");
    if results.is_empty() {
        out.push_str("(no tools ran)\n");
    }
    for result in results {
        let status = if result.ok { "ok" } else { "error" };
        let error_attr = match &result.error {
            Some(err) => format!(" error_code=\"{:?}\"", err.code),
            None => String::new(),
        };
        out.push_str(&format!(
            "<observation id=\"{}\" tool=\"{}\" status=\"{}\"{}>\n<input>\n{}\n</input>\n<output>\n{}\n</output>\n</observation>\n",
            result.id,
            result.name,
            status,
            error_attr,
            result.input,
            result.output
        ));
    }
    out.push_str("</observations>\n");
    out.push_str(
        "Use the observations above (match by id). When no more tools are needed, answer in prose without tool JSON or fences.",
    );
    out
}

/// System-prompt fragment describing the JSON tool protocol.
pub fn protocol_instructions(registry: &ToolRegistry) -> String {
    let names: Vec<&str> = registry
        .definitions()
        .iter()
        .map(|d| d.name)
        .chain(registry.external_definitions().iter().map(|e| e.name.as_str()))
        .collect();
    // No tools → no protocol (2026-09-29). An "Available tools: ." line
    // teaches a JSON tool habit with nothing behind it and contradicts the
    // conversational arm's "no file or command tools are available".
    if names.is_empty() {
        return String::new();
    }
    format!(
        "Tool protocol: when you need a tool, reply with ONLY one JSON object (no prose):\n\
         {{\"tool_calls\":[{{\"id\":\"1\",\"name\":\"<tool>\",\"arguments\":{{…}}}}]}}\n\
         Available tools: {}.\n\
         You may put multiple calls in `tool_calls`.\n\
         After tools run you will receive an <observations> block with matching ids.\n\
         When finished, answer in prose without JSON tool calls.",
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> ToolRegistry {
        ToolRegistry::standard()
    }

    #[test]
    fn for_mode_follows_the_mode_project_matrix() {
        // No project → only websearch (project-independent tool).
        let no_project_code = ToolRegistry::for_mode(crate::AgentMode::Code, false);
        assert_eq!(
            no_project_code.definitions().len(),
            1,
            "no-project cell offers only websearch"
        );
        assert!(no_project_code.find("websearch").is_some());
        let no_project_work = ToolRegistry::for_mode(crate::AgentMode::Work, false);
        assert_eq!(
            no_project_work.definitions().len(),
            1,
            "no-project cell offers only websearch"
        );
        assert!(no_project_work.find("websearch").is_some());

        // Code + project → the full coding agent, symbol tools included,
        // websearch included.
        let code = ToolRegistry::for_mode(crate::AgentMode::Code, true);
        for name in [
            "search",
            "read_file",
            "list_files",
            "find_symbol",
            "find_references",
            "read_range",
            "run_command",
            "write_file",
            "apply_patch",
            "replace_range",
            "create_file",
            "delete_file",
            "websearch",
        ] {
            assert!(code.find(name).is_some(), "code+project must offer {name}");
        }

        // Work + project → file tools, search and run_command — never symbols,
        // websearch included.
        let work = ToolRegistry::for_mode(crate::AgentMode::Work, true);
        for name in [
            "search",
            "read_file",
            "list_files",
            "run_command",
            "write_file",
            "apply_patch",
            "replace_range",
            "create_file",
            "delete_file",
            "websearch",
        ] {
            assert!(work.find(name).is_some(), "work+project must offer {name}");
        }
        for name in ["find_symbol", "find_references", "read_range"] {
            assert!(
                work.find(name).is_none(),
                "work+project must not offer {name}"
            );
        }
    }

    #[test]
    fn tag_protocol_parses_captured_wire_sample() {
        // Verbatim shape seen from mimo-v2.6-flash: four calls concatenated.
        let text = concat!(
            "<tool_call><function=search><parameter=query>--kodo-canvas</parameter></function></tool_call>",
            "<tool_call><function=search><parameter=query>:root</parameter></function></tool_call>",
            "<tool_call><function=read_file><parameter=path>apps/desktop/src/styles/app.css</parameter>",
            "<parameter=limit>400</parameter><parameter=offset>100</parameter></function></tool_call>",
            "<tool_call><function=read_file><parameter=path>apps/desktop/src/styles/conversation.css</parameter>",
            "<parameter=limit>300</parameter><parameter=offset>120</parameter></function></tool_call>"
        );
        match parse_model_turn(text, &registry()) {
            ModelTurn::Tools { calls } => {
                assert_eq!(calls.len(), 4, "two searches + two reads, all executed");
                match &calls[0] {
                    ToolInvocation::Ready(call) => {
                        assert_eq!(call.name, ToolName::Search);
                        assert_eq!(
                            call.args,
                            ToolArgs::Search {
                                query: "--kodo-canvas".to_owned()
                            }
                        );
                    }
                    other => panic!("expected ready search, got {other:?}"),
                }
                match &calls[2] {
                    ToolInvocation::Ready(call) => {
                        assert_eq!(call.name, ToolName::ReadFile);
                        assert_eq!(
                            call.args,
                            ToolArgs::ReadFile {
                                path: "apps/desktop/src/styles/app.css".to_owned()
                            }
                        );
                    }
                    other => panic!("expected ready read_file, got {other:?}"),
                }
                match &calls[3] {
                    ToolInvocation::Ready(call) => {
                        assert_eq!(call.name, ToolName::ReadFile);
                        assert_eq!(
                            call.args,
                            ToolArgs::ReadFile {
                                path: "apps/desktop/src/styles/conversation.css".to_owned()
                            }
                        );
                    }
                    other => panic!("expected ready read_file, got {other:?}"),
                }
            }
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn tag_protocol_keeps_trailing_prose_out_of_the_calls() {
        let text = "<tool_call><function=search><parameter=query>router</parameter></function></tool_call>\n**Verification status:** Partially verified";
        match parse_model_turn(text, &registry()) {
            ModelTurn::Tools { calls } => {
                assert_eq!(calls.len(), 1);
                match &calls[0] {
                    ToolInvocation::Ready(call) => {
                        assert_eq!(call.name, ToolName::Search);
                        assert_eq!(
                            call.args,
                            ToolArgs::Search {
                                query: "router".to_owned()
                            }
                        );
                    }
                    other => panic!("expected ready search, got {other:?}"),
                }
            }
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn tag_protocol_unknown_function_is_rejected_not_final() {
        let text = "<tool_call><function=launch_missiles><parameter=x>1</parameter></function></tool_call>";
        match parse_model_turn(text, &registry()) {
            ModelTurn::Tools { calls } => {
                assert_eq!(calls.len(), 1);
                assert!(matches!(calls[0], ToolInvocation::Rejected(_)));
            }
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn plain_prose_is_never_tag_protocol() {
        assert!(!looks_like_tag_tool_protocol(
            "这个项目用 function= 说明一下路由"
        ));
        assert!(looks_like_tag_tool_protocol(
            "<tool_call><function=search><parameter=query>x</parameter></function></tool_call>"
        ));
        match parse_model_turn("做好了。", &registry()) {
            ModelTurn::Final { text } => assert_eq!(text, "做好了。"),
            other => panic!("expected final, got {other:?}"),
        }
    }

    #[test]
    fn scenario_a_parses_read_file_and_search_together() {
        let text = r#"{"tool_calls":[
            {"id":"a1","name":"read_file","arguments":{"path":"src/lib.rs"}},
            {"id":"a2","name":"search","arguments":{"query":"fn main"}}
        ]}"#;
        match parse_model_turn(text, &registry()) {
            ModelTurn::Tools { calls } => {
                assert_eq!(calls.len(), 2);
                let ToolInvocation::Ready(c0) = &calls[0] else {
                    panic!("expected ready")
                };
                assert_eq!(c0.id.as_str(), "a1");
                assert_eq!(c0.name, ToolName::ReadFile);
                assert_eq!(
                    c0.args,
                    ToolArgs::ReadFile {
                        path: "src/lib.rs".into()
                    }
                );
                let ToolInvocation::Ready(c1) = &calls[1] else {
                    panic!("expected ready")
                };
                assert_eq!(c1.id.as_str(), "a2");
                assert_eq!(c1.name, ToolName::Search);
                assert_eq!(
                    c1.args,
                    ToolArgs::Search {
                        query: "fn main".into()
                    }
                );
            }
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn scenario_b_two_legal_calls_in_one_response() {
        let text = r#"{"tool_calls":[
            {"id":"1","name":"run_command","arguments":{"command":"git status --short"}},
            {"id":"2","name":"write_file","arguments":{"path":"notes/a.md","content":"hi\n"}}
        ]}"#;
        match parse_model_turn(text, &registry()) {
            ModelTurn::Tools { calls } => {
                assert_eq!(calls.len(), 2);
                assert!(calls.iter().all(|c| matches!(c, ToolInvocation::Ready(_))));
            }
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn scenario_c_missing_arguments_yields_rejected_not_panic() {
        let text = r#"{"tool_calls":[{"id":"x","name":"read_file","arguments":{}}]}"#;
        match parse_model_turn(text, &registry()) {
            ModelTurn::Tools { calls } => {
                assert_eq!(calls.len(), 1);
                match &calls[0] {
                    ToolInvocation::Rejected(r) => {
                        assert_eq!(r.error.code, ToolErrorCode::InvalidArguments);
                        assert!(r.error.message.contains("path"));
                        assert_eq!(r.id.as_str(), "x");
                    }
                    other => panic!("expected rejected, got {other:?}"),
                }
            }
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn scenario_d_unknown_tool_yields_rejected() {
        let text =
            r#"{"tool_calls":[{"id":"u","name":"drop_database","arguments":{"query":"x"}}]}"#;
        match parse_model_turn(text, &registry()) {
            ModelTurn::Tools { calls } => match &calls[0] {
                ToolInvocation::Rejected(r) => {
                    assert_eq!(r.error.code, ToolErrorCode::UnknownTool);
                    assert_eq!(r.name, "drop_database");
                }
                other => panic!("expected unknown tool rejection, got {other:?}"),
            },
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn malformed_json_recovers_without_session_panic() {
        // Broken JSON that looks like the protocol: recover to structured error.
        let text = r#"{"tool_calls":[{"id":"1","name":"read_file","arguments":{"path":"a""#;
        match parse_model_turn(text, &registry()) {
            ModelTurn::Tools { calls } => {
                assert_eq!(calls.len(), 1);
                assert!(matches!(calls[0], ToolInvocation::Rejected(_)));
            }
            ModelTurn::Final { text } => {
                // Acceptable alternate recovery: keep text as final answer.
                assert!(text.contains("tool_calls"));
            }
        }
    }

    #[test]
    fn missing_required_field_on_command() {
        let text = r#"{"tool_calls":[{"name":"run_command","arguments":{}}]}"#;
        match parse_model_turn(text, &registry()) {
            ModelTurn::Tools { calls } => match &calls[0] {
                ToolInvocation::Rejected(r) => {
                    assert_eq!(r.error.code, ToolErrorCode::InvalidArguments);
                    assert!(r.error.message.contains("command"));
                }
                other => panic!("expected rejection, got {other:?}"),
            },
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn arguments_wrong_type_rejected() {
        let text = r#"{"tool_calls":[{"name":"search","arguments":{"query":42}}]}"#;
        match parse_model_turn(text, &registry()) {
            ModelTurn::Tools { calls } => match &calls[0] {
                ToolInvocation::Rejected(r) => {
                    assert_eq!(r.error.code, ToolErrorCode::InvalidArguments)
                }
                other => panic!("expected rejection, got {other:?}"),
            },
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn final_prose_without_tools() {
        match parse_model_turn("All good. - checked tests", &registry()) {
            ModelTurn::Final { text } => assert!(text.contains("All good")),
            other => panic!("expected final, got {other:?}"),
        }
    }

    #[test]
    fn deprecated_fence_still_parses_multiple_calls() {
        let text = "\
```bash\necho one\n```\n\
```search\nfn main\n```\n\
```read\npath: src/lib.rs\n```\n\
```write\npath: notes/a.md\n---\nbody\n```";
        match parse_model_turn(text, &registry()) {
            ModelTurn::Tools { calls } => {
                assert_eq!(calls.len(), 4);
                assert!(calls.iter().all(|c| matches!(c, ToolInvocation::Ready(_))));
                let names: Vec<_> = calls
                    .iter()
                    .filter_map(|c| match c {
                        ToolInvocation::Ready(c) => Some(c.name),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    names,
                    vec![
                        ToolName::RunCommand,
                        ToolName::Search,
                        ToolName::ReadFile,
                        ToolName::WriteFile
                    ]
                );
                // Stable unique ids
                let ids: std::collections::HashSet<_> =
                    calls.iter().map(|c| c.id().as_str().to_owned()).collect();
                assert_eq!(ids.len(), 4);
            }
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn registry_rejects_unknown_and_accepts_known() {
        let reg = registry();
        let ok = reg.accept(
            ToolCallId::new("1"),
            "read_file",
            &serde_json::json!({"path":"a"}),
        );
        assert!(matches!(ok, ToolInvocation::Ready(_)));
        let bad = reg.accept(ToolCallId::new("2"), "nope", &serde_json::json!({}));
        assert!(matches!(bad, ToolInvocation::Rejected(_)));
    }

    #[test]
    fn format_observations_includes_id_and_error_code() {
        let result = ToolResult::failure(
            ToolCallId::new("e1"),
            "read_file",
            "../etc/passwd",
            ToolError::path_escape("escapes root"),
        );
        let block = format_observations(&[result]);
        assert!(block.contains("id=\"e1\""));
        assert!(block.contains("status=\"error\""));
        assert!(block.contains("PathEscape"));
        assert!(block.contains("../etc/passwd"));
    }

    #[test]
    fn tool_definitions_cover_required_tools() {
        let reg = registry();
        for name in ["search", "read_file", "write_file", "run_command"] {
            let def = reg.find(name).unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(def.name, name);
            assert!(def.input_schema.get("type").is_some());
            assert!(def.input_schema.get("properties").is_some());
        }
    }

    fn registry_with_external() -> ToolRegistry {
        registry().with_externals(vec![ExternalToolDef {
            name: "mcp__fake__echo".to_owned(),
            description: "Echo the input back".to_owned(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }),
        }])
    }

    #[test]
    fn accept_resolves_external_wire_names_to_external_calls() {
        let reg = registry_with_external();
        let args = serde_json::json!({ "text": "hi" });
        let inv = reg.accept(ToolCallId::new("1"), "mcp__fake__echo", &args);
        match inv {
            ToolInvocation::External(call) => {
                assert_eq!(call.id.as_str(), "1");
                assert_eq!(call.name, "mcp__fake__echo");
                assert_eq!(call.args, args);
            }
            other => panic!("expected external call, got {other:?}"),
        }
    }

    #[test]
    fn accept_still_rejects_unknown_wire_names() {
        let reg = registry_with_external();
        let inv = reg.accept(ToolCallId::new("2"), "mcp__other__tool", &serde_json::json!({}));
        match inv {
            ToolInvocation::Rejected(reject) => {
                assert_eq!(reject.error.code, ToolErrorCode::UnknownTool);
                assert_eq!(reject.name, "mcp__other__tool");
            }
            other => panic!("expected rejection, got {other:?}"),
        }
        // And with no externals registered at all.
        let inv = registry().accept(ToolCallId::new("3"), "mcp__fake__echo", &serde_json::json!({}));
        assert!(matches!(inv, ToolInvocation::Rejected(_)));
    }

    #[test]
    fn filtered_keeps_externals_when_narrowing_builtins() {
        // Skill allowlists narrow builtins only: externals survive filtering.
        let reg = registry_with_external().filtered(|name| name == "read_file");
        assert_eq!(reg.definitions().len(), 1);
        assert_eq!(reg.external_definitions().len(), 1);
        assert!(reg.contains_name("mcp__fake__echo"));
    }

    #[test]
    fn protocol_instructions_list_external_tool_names() {
        let text = protocol_instructions(&registry_with_external());
        assert!(text.contains("mcp__fake__echo"));
        assert!(text.contains("read_file"));
    }

    #[test]
    fn protocol_instructions_is_silent_for_an_empty_registry() {
        // Projectless constrained Q&A: no tools means no protocol — the
        // "Available tools: ." line must never reach the model.
        let text = protocol_instructions(&ToolRegistry::default());
        assert!(text.is_empty(), "empty registry must render no protocol: {text:?}");
        assert!(!text.contains("Available tools"));
    }

    #[test]
    fn external_into_result_redacts_and_reports_wire_name() {
        let inv = ToolInvocation::External(ExternalCall {
            id: ToolCallId::new("e1"),
            name: "mcp__fake__echo".to_owned(),
            args: serde_json::json!({ "text": "hello", "api_key": "sk-live-abcdef123456" }),
        });
        let result = inv.into_result();
        assert!(result.ok);
        assert_eq!(result.name, "mcp__fake__echo");
        assert_eq!(result.id.as_str(), "e1");
        assert!(result.input.contains("hello"));
        assert!(
            !result.input.contains("sk-live-abcdef123456"),
            "external args must be redacted: {}",
            result.input
        );
        assert!(result.output.is_empty());
    }

    #[test]
    fn rejection_becomes_structured_tool_result() {
        let inv = ToolInvocation::Rejected(RejectedCall {
            id: ToolCallId::new("r1"),
            name: "drop_database".into(),
            error: ToolError::unknown_tool("drop_database"),
        });
        let result = inv.into_result();
        assert!(!result.ok);
        assert_eq!(result.id.as_str(), "r1");
        assert_eq!(
            result.error.as_ref().unwrap().code,
            ToolErrorCode::UnknownTool
        );
    }

    #[test]
    fn format_observations_uses_input_label() {
        let block = format_observations(&[ToolResult::success(
            ToolCallId::new("i1"),
            "search",
            "fn main",
            "3 files",
        )]);
        assert!(block.contains("<input>\nfn main\n</input>"));
        assert!(block.contains("3 files"));
    }

    #[test]
    fn json_protocol_inside_code_fence_is_accepted() {
        let text = "```json\n{\"tool_calls\":[{\"id\":\"z\",\"name\":\"search\",\"arguments\":{\"query\":\"foo\"}}]}\n```";
        match parse_model_turn(text, &registry()) {
            ModelTurn::Tools { calls } => {
                assert_eq!(calls.len(), 1);
                assert!(matches!(&calls[0], ToolInvocation::Ready(c) if c.id.as_str() == "z"));
            }
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn strip_think_blocks_removes_inline_reasoning() {
        let (text, saw) = strip_think_blocks("before <think>secret</think>after");
        assert!(saw);
        assert_eq!(text, "before after");

        let (text, saw) = strip_think_blocks(" <thinking>long thought</thinking>答案");
        assert!(saw);
        assert_eq!(text, "答案");

        // Multiple blocks interleave with prose — surrounding whitespace
        // is preserved verbatim; only leading whitespace before the first
        // visible character is trimmed.
        let (text, _) = strip_think_blocks("a <think>x</think>b <think>y</think>c");
        assert_eq!(text, "a b c");
    }

    #[test]
    fn strip_think_blocks_unterminated_block_swallows_the_rest() {
        // The model stopped mid-thinking — the remainder is thinking too.
        let (text, saw) = strip_think_blocks(" <think>never closed");
        assert!(saw);
        assert_eq!(text, "");
    }

    #[test]
    fn think_blocks_do_not_hide_the_tool_call_that_follows() {
        // 2026-10-02 incident: a leading ` <think>` made the JSON tool call
        // undetectable; it got eaten and raw thinking rendered as the reply.
        let text = " <think>Let me list the files first.\
            </think>{\"tool_calls\":[{\"id\":\"1\",\"name\":\"list_files\",\"arguments\":{}}]}";
        match parse_model_turn(text, &registry()) {
            ModelTurn::Tools { calls } => {
                assert_eq!(calls.len(), 1);
                assert!(matches!(&calls[0], ToolInvocation::Ready(c) if c.name.label() == "list_files"));
            }
            other => panic!("expected tools, got {other:?}"),
        }
    }

    #[test]
    fn think_only_message_becomes_empty_final() {
        match parse_model_turn(" <think>only thinking</think>", &registry()) {
            ModelTurn::Final { text } => assert_eq!(text.trim(), ""),
            other => panic!("expected final, got {other:?}"),
        }
    }

    #[test]
    fn think_stripper_streams_across_chunk_boundaries() {
        let mut s = ThinkStripper::default();
        let mut out = String::new();
        // Open tag split across deltas: nothing leaks.
        out.push_str(&s.push("pre <th"));
        out.push_str(&s.push("ink>inner"));
        assert_eq!(out, "pre ");
        // Close tag split across deltas: block content stays hidden.
        out.push_str(&s.push(" text</th"));
        out.push_str(&s.push("ink>post"));
        assert_eq!(out, "pre post");
        out.push_str(&s.finish());
        assert_eq!(out, "pre post");
        assert!(s.saw_think());
    }

    #[test]
    fn think_stripper_finish_holds_unterminated_block() {
        let mut s = ThinkStripper::default();
        assert_eq!(s.push(" <think>cut off"), "");
        assert_eq!(s.finish(), "");
    }

    #[test]
    fn think_stripper_keeps_plain_text_and_partial_lookalikes() {
        let mut s = ThinkStripper::default();
        let mut out = s.push("a <think but not a tag");
        out.push_str(&s.finish());
        assert_eq!(out, "a <think but not a tag");
    }

    /// Phase 0 second-pass — schema-aware search rejection tells the model
    /// every required field at once, so it doesn't have to guess them
    /// round-by-round. The screenshot showed 5 consecutive failures all
    /// missing `query`; this message names the field plus the tool.
    #[test]
    fn search_rejection_message_lists_required_fields() {
        let v = serde_json::json!({});
        let err = ToolArgs::from_json(ToolName::Search, &v)
            .expect_err("missing query must reject");
        assert!(
            err.message.contains("`query`"),
            "schema hint must name the missing field: {}",
            err.message
        );
        assert!(
            err.message.contains("search requires"),
            "schema hint must name the tool's required schema: {}",
            err.message
        );
    }

    /// Phase 0 second-pass — `ask_user` accepts every alias the docs list
    /// (`ask_user`, `ask`, `ask-user`) and emits a typed label.
    #[test]
    fn parse_ask_user_accepts_aliases() {
        for alias in ["ask_user", "ask", "ask-user"] {
            let v = serde_json::json!({"question": "用哪个框架？"});
            let args = ToolArgs::from_json(ToolName::parse(alias).unwrap(), &v)
                .expect("ask_user must parse");
            match args {
                ToolArgs::AskUser { question, options } => {
                    assert_eq!(question, "用哪个框架？");
                    assert!(options.is_empty());
                }
                other => panic!("alias {alias} produced {other:?}"),
            }
        }
    }

    /// Phase 0 second-pass — `ask_user` without `question` is rejected with
    /// the schema-aware message, so the model fixes it on the next turn.
    #[test]
    fn parse_ask_user_rejects_missing_question_with_schema_hint() {
        let v = serde_json::json!({});
        let err = ToolArgs::from_json(ToolName::AskUser, &v)
            .expect_err("missing question must reject");
        assert!(err.message.contains("`question`"), "got: {}", err.message);
        assert!(
            err.message.contains("ask_user requires"),
            "got: {}",
            err.message
        );
    }

    /// Phase 0 second-pass — `ask_user` with `options` array round-trips.
    #[test]
    fn parse_ask_user_options_round_trip() {
        let v = serde_json::json!({
            "question": "用哪个框架？",
            "options": ["Rust", "TypeScript"]
        });
        let args = ToolArgs::from_json(ToolName::AskUser, &v).unwrap();
        match args {
            ToolArgs::AskUser { question, options } => {
                assert_eq!(question, "用哪个框架？");
                assert_eq!(options, vec!["Rust".to_owned(), "TypeScript".to_owned()]);
            }
            other => panic!("expected AskUser, got {other:?}"),
        }
    }
}
