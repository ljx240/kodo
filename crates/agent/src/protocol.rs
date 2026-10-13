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
                question: require_string_with_schema(args, "question", "ask_user", &["question"])?,
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
                changed_files: Vec::new(),
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
    pub changed_files: Vec<String>,
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
            changed_files: Vec::new(),
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
            changed_files: Vec::new(),
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
            changed_files: Vec::new(),
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
                    description: "Read UTF-8, DOCX body/table text, or spreadsheet cells inside the project. Paths are relative to the project root.",
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
                    description: "Write a UTF-8 file, or generate a DOCX from text/Markdown headings and pipe tables. Creates parent directories; DOCX is validated by readback.",
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
                    name: "ask_user",
                    description: "Ask for missing information or required authorization. Pauses the turn; wait for the user's next message before proceeding.",
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "question": { "type": "string" },
                            "options": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["question"],
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
            return Self::standard().filtered(|name| matches!(name, "websearch"));
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

    /// Filter advertised builtin and external tools using the same skill policy.
    pub fn filtered<F: Fn(&str) -> bool>(&self, keep: F) -> Self {
        Self {
            defs: self.defs.iter().filter(|d| keep(d.name)).cloned().collect(),
            externals: self
                .externals
                .iter()
                .filter(|d| keep(&d.name))
                .cloned()
                .collect(),
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

/// Optional public progress text from the JSON fallback, separate from tool arguments.
pub fn tool_commentary(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(strip_code_fence_json(text)).ok()?;
    value.get("commentary")?.as_str().map(str::to_owned)
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

/// Return the largest char-boundary index `<= idx`. Used when a byte
/// offset measured against one string must be transferred to a slice of
/// another and the two have different multi-byte widths.
fn char_floor(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// 2026-10-13 follow-up: some preview channels (notably `MiniMax-M3`) end
/// their answer with a stray private control token such as `.[<|minimax|>[`
/// or a bare `[*[` channel-marker left over from the model's chat template.
/// These never belong in the user-facing reply. We strip any tail of the
/// text that contains a `<|…|>` style token (the inner contents are kept
/// here, but on the wire those are *private* provider channel markers, so
/// the safe move is to remove the whole token, not its label).
fn strip_trailing_control_tokens(text: &str) -> (String, bool) {
    // Repeatedly drop a `<|...|>` token from anywhere in the text. Providers
    // occasionally emit more than one (one per channel switch), and the
    // answer can end after a chain of them. We also eat any leading
    // separator that the chat template padded the token with (`.`, `[`,
    // `]`, `,`, `;`, `:`, ` `) and any trailing `[`/`]` that the template
    // uses to bracket channel markers. The boolean return signals whether
    // at least one `<|…|>` token was seen — callers use this to know
    // whether trailing brackets at finish() are leftover template noise
    // (true) or real prose (false).
    // MiniMax also emits bracket delimiters instead of pipes (tmue47-61aehc-0).
    let mut out = text.replace("<]minimax[", "<|minimax|");
    let mut saw_token = false;
    loop {
        let Some(start) = out.find("<|") else {
            break;
        };
        saw_token = true;
        let Some(end_rel) = out[start + 2..].find("|>") else {
            // Unterminated `<|…` — the safest interpretation is "the model
            // went off the rails; drop the rest". We don't expect to see
            // these in the wild outside preview channels emitting garbage
            // before a stop, so the strict cut is fine.
            out.truncate(start);
            break;
        };
        let mut end = start + 2 + end_rel + 2;
        // Eat a trailing `]` or `[` that templates use to close channel
        // markers (e.g. `<|minimax|>[`). One character is enough; chains
        // of bracket noise are uncommon and would be a bigger template
        // problem.
        if let Some(next) = out[end..].chars().next() {
            if next == '[' || next == ']' {
                end += next.len_utf8();
            }
        }
        // Eat leading separators (one or more) that the chat template
        // used to pad the token. The pattern we have seen in the wild
        // is `.[<|minimax|>[` — a `.` and a `[` immediately before
        // `<|`. The first version of this scrubber ate at most one
        // separator, but that left the leading `.` in the answer
        // (e.g. `已锁定这些关键函数.[<|minimax|>[` → `已锁定这些关键函数.`).
        // The risk of eating back further is over-truncating real
        // sentence punctuation like `answer is 8. .[<|…` — that case
        // would lose the first `.`. We accept that trade-off because
        // the model almost never ends a sentence with `.` immediately
        // before a chat-template marker; the only place that pattern
        // shows up is the template itself, and dropping a real
        // sentence-final period is far less harmful than leaking the
        // template's private channel marker.
        let mut cut_start = start;
        while cut_start > 0 {
            let prev = out[..cut_start].chars().last().unwrap();
            if matches!(prev, '.' | '[' | ']' | ',' | ';' | ':' | ' ') {
                cut_start -= prev.len_utf8();
            } else {
                break;
            }
        }
        out.replace_range(cut_start..end, "");
    }
    (out, saw_token)
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
            let (cleaned, _had_token) = strip_trailing_control_tokens(&out);
            if saw {
                let trimmed = cleaned.trim_start().to_owned();
                return (trimmed, saw);
            }
            return (cleaned, saw);
        };
        saw = true;
        out.push_str(&rest[..pos]);
        let after = &rest[pos + open.len()..];
        match earliest(after, &THINK_CLOSES) {
            Some((end, close)) => rest = &after[end + close.len()..],
            None => {
                let (cleaned, _had_token) = strip_trailing_control_tokens(&out);
                let trimmed = cleaned.trim_start().to_owned();
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
    /// 2026-10-13 (third round): set when a `<|…|>` token was actually
    /// observed and dropped from a visible chunk. Lets `finish()` decide
    /// whether a trailing single bracket in `hold` is leftover chat-
    /// template noise (drop) or real prose (keep). Without this signal
    /// `finish()` cannot distinguish `answer is 8[` (held back from a
    /// real control-token split) from `the array [1]` (held back from
    /// an array-literal split).
    saw_control_token: bool,
}

impl ThinkStripper {
    /// Feed one content delta; returns the visible answer text it unlocked.
    pub fn push(&mut self, delta: &str) -> String {
        // 2026-10-13 (third round): preview channels leak a private
        // `<|…|>` channel marker into the answer tail. The marker is
        // often split across deltas (`.[`, `<|minimax|`, `>[`) so a
        // post-hoc scrub of `newly` alone cannot reach the `.[` that was
        // already emitted. Two complementary mechanisms:
        //
        //   1. If `hold` already contains a partial control-token
        //      sequence (e.g. `.[` or `.[<|minimax|`) and the incoming
        //      `delta` would continue that sequence, we absorb the
        //      whole delta into `hold` and return nothing. The eventual
        //      `finish()` scrubber drops the whole thing.
        //
        //   2. If `hold` has a partial control-token sequence but the
        //      incoming `delta` does *not* continue it (e.g. the model
        //      was just writing prose like `正文 a.[` followed by
        //      `[后面的内容`), the held prefix is re-emitted as plain
        //      text and the delta is processed normally.
        if !self.hold.is_empty() {
            let first_new = delta.chars().next();
            if control_token_continues(&self.hold, first_new) {
                self.hold.push_str(delta);
                return String::new();
            }
            let prefix_len = control_prefix_len(&self.hold);
            if prefix_len > 0 {
                // The held prefix was a control-token candidate but the
                // next delta disproved it. Release the prefix back into
                // the input stream so the main loop treats it as plain
                // text.
                let released = self.hold[..prefix_len].to_owned();
                // Collapse the duplicate-bracket boundary: if the
                // released prefix ends in `[` (or `<`) and the new
                // delta starts with the same char, drop the delta's
                // leading char. Otherwise the user would see
                // `正文 a.` + `[` + `[后面的内容` = `正文 a.[[后面的内容`
                // (two adjacent brackets) when the model actually
                // wrote `正文 a.[后面的内容` (one bracket, just
                // punctuation that the next delta extended with more
                // prose). This is the streaming equivalent of letting
                // the held prefix and the next delta share their
                // boundary character.
                let mut delta_to_process = delta;
                if let (Some(last_released), Some(first_delta)) =
                    (released.chars().last(), delta.chars().next())
                {
                    if last_released == first_delta
                        && (last_released == '[' || last_released == '<')
                    {
                        delta_to_process = &delta[first_delta.len_utf8()..];
                    }
                }
                let mut new_hold =
                    String::with_capacity(self.hold.len() - prefix_len + delta_to_process.len());
                new_hold.push_str(&self.hold[prefix_len..]);
                new_hold.push_str(delta_to_process);
                self.hold = new_hold;
                let newly = self.drain_visible();
                let (cleaned, had_token) = strip_trailing_control_tokens(&newly);
                if had_token {
                    self.saw_control_token = true;
                }
                return self.finalize_visible(&format!("{released}{cleaned}"));
            }
        }
        self.hold.push_str(delta);
        let newly = self.drain_visible();
        let (cleaned, had_token) = strip_trailing_control_tokens(&newly);
        if had_token {
            self.saw_control_token = true;
        }
        self.finalize_visible(&cleaned)
    }

    /// Run the main think-block dispatch loop on `self.hold`, returning the
    /// newly-released visible chunk. The remaining `hold` (a partial
    /// think-tag tail or a control-token prefix) is left in `self.hold`.
    /// The defer step that pulls a trailing control-token opener back
    /// into `self.hold` is part of this helper so both the "fresh
    /// delta" path and the "release prefix" path in `push` get it.
    fn drain_visible(&mut self) -> String {
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
        if let Some(start) = newly.rfind("<]") {
            let tail = &newly[start..];
            if "<]minimax[>".starts_with(tail) || tail.starts_with("<]minimax[>") {
                let mut cut = start;
                while cut > 0 && matches!(newly[..cut].chars().last(), Some(']' | '[' | ' ' | '.'))
                {
                    cut -= 1;
                }
                let held = newly.split_off(cut);
                self.hold = format!("{held}{}", self.hold);
                return newly;
            }
        }
        // Defer the trailing control-token opener (and the chat
        // template's leading `.` or space separator when applicable)
        // so the next delta can either grow into a full marker (which
        // `finish()` will drop) or disprove the pattern (which causes
        // re-emission on the next push).
        //
        // 2026-10-13 (fourth round): the chat template also wraps the
        // marker in `]<|…|>[` (close-bracket variant, e.g. after
        // `consumer ]<|minimax|>[` in real model output). `]` is now
        // treated as a control-token opener so the next delta decides
        // whether to absorb it (continues the marker) or release it
        // (real prose, e.g. an array index `arr[0]`-like closing).
        //
        // Note on the ` ` (space) separator: a trailing space is
        // *not* deferred here even though the template uses ` ]<|…|>`
        // — pulling it back would also over-defer real prose like
        // `pre <think>` (the think-tag framing test). Instead, the
        // ` ]` case is handled when both characters land in the same
        // `newly` (the `last == ']' && prev == ' '` pull-back below)
        // or when the space and `]` arrive in the same delta. A
        // space that lives in its own SSE chunk (between prose and a
        // `]` chunk) is a known small leak — the scrubber at
        // `finish()` eats the marker but the leading space stays in
        // the answer. This is documented as a known limitation
        // because fixing it without API change would also over-defer
        // real prose.
        if let Some(last) = newly.chars().last() {
            if last == '<' || last == '[' || last == ']' {
                let last_len = last.len_utf8();
                // For `[` the chat template's separator is often a
                // single `.` immediately before (giving `.[`). For `]`
                // the template's separator is usually a single space
                // immediately before (giving ` ]`). Pull the separator
                // back too so `hold` carries the full `.[` or ` ]`
                // prefix — without it, a real `<|minimax|>` in the
                // next delta would scrub only `<|minimax|>` and leave
                // the separator in the answer. We do NOT pull other
                // separators (`,`, `;`, `:`, `.` for `]`) because they
                // are too common in prose and the template does not
                // use them.
                let mut bytes_to_defer = last_len;
                let chars: Vec<char> = newly.chars().collect();
                if last == '[' && chars.len() >= 2 && chars[chars.len() - 2] == '.' {
                    bytes_to_defer += '.'.len_utf8();
                }
                if last == ']' && chars.len() >= 2 && chars[chars.len() - 2] == ' ' {
                    bytes_to_defer += ' '.len_utf8();
                }
                // For `<` the template's framing is `]<|…|>` (close
                // bracket variant). When the prior two chars are `] `
                // (i.e. the same ` ]<` from the ` ]<|minimax|>[`
                // pattern, both in this same `newly`), pull both back
                // so the hold carries the full `]<` prefix. Without
                // this, a same-delta ` ]<|minimax|>` would emit ` ]`
                // and only hold `<` — then `control_token_continues`
                // would absorb the next delta correctly but the
                // already-emitted ` ]` would be left in the answer.
                if last == '<' && chars.len() >= 2 && chars[chars.len() - 2] == ']' {
                    bytes_to_defer += 1;
                    if chars.len() >= 3 && chars[chars.len() - 3] == ' ' {
                        bytes_to_defer += 1;
                    }
                }
                let keep_from = char_floor(&newly, newly.len() - bytes_to_defer);
                let held = newly.split_off(keep_from);
                self.hold = format!("{held}{}", self.hold);
            }
        }
        newly
    }

    /// Apply the "trim leading whitespace until first visible" rule and
    /// return what `push` should return for this delta.
    fn finalize_visible(&mut self, cleaned: &str) -> String {
        if !self.released_visible {
            let trimmed: String = cleaned.trim_start().into();
            if !trimmed.is_empty() {
                self.released_visible = true;
            }
            return trimmed;
        }
        cleaned.to_owned()
    }

    /// End of stream: outside a block the hold is plain text; inside one the
    /// model stopped mid-thinking and the remainder stays hidden.
    pub fn finish(&mut self) -> String {
        if self.in_think {
            self.hold.clear();
            return String::new();
        }
        // 2026-10-13 (third round): when the stream ends with a partial
        // control-token sequence in `hold` (e.g. `.[` or `.[<|minimax|>[`),
        // `strip_trailing_control_tokens` drops the whole thing if a
        // `<|…|>` token is present. If the partial was only a single
        // trailing bracket with no `<|`, we still drop it when
        // `saw_control_token` is true: the bracket is the chat
        // template's channel-close marker left dangling after a control
        // token was already scrubbed. Real prose does not end in a
        // solitary `[`/`]`, so the over-stripping risk is negligible.
        let hold = std::mem::take(&mut self.hold);
        let (cleaned, had_token) = strip_trailing_control_tokens(&hold);
        let mut cleaned = cleaned;
        if had_token || self.saw_control_token {
            while let Some(last) = cleaned.chars().last() {
                if last == '[' || last == ']' {
                    cleaned.pop();
                } else {
                    break;
                }
            }
        }
        cleaned
    }

    pub fn saw_think(&self) -> bool {
        self.saw_think
    }
}

/// 2026-10-13: would appending `first_new` to `hold` extend a chat-
/// template control-token prefix? Returns true when the combined bytes
/// could grow into `.[<|…|>[`, `[<|…|>[`, ` ]<|…|>[`, `]<|…|>[`, or
/// `<|…|>`.
fn control_token_continues(hold: &str, first_new: Option<char>) -> bool {
    if hold.contains("<|") || hold.contains("<]") {
        return true;
    }
    let Some(first) = first_new else {
        return false;
    };
    if hold.ends_with('<') && first == ']' {
        return true;
    }
    if hold.ends_with(".<") && first == '|' {
        return true;
    }
    if hold.ends_with(".[") && first == '<' {
        return true;
    }
    if hold.ends_with("[<") && first == '|' {
        return true;
    }
    if hold.ends_with("<") && first == '|' {
        return true;
    }
    if hold.ends_with("[") && first == '<' {
        return true;
    }
    // Fourth-round additions: the chat template also wraps the marker
    // with `]…<|…|>[`, so a held `]` (or ` ]` if the space was in the
    // same delta) and a `<` continuation in the next delta must keep.
    if hold.ends_with(" <") && first == '|' {
        return true;
    }
    if hold.ends_with(" ]") && first == '<' {
        return true;
    }
    if hold.ends_with("]<") && first == '|' {
        return true;
    }
    if hold.ends_with("]") && first == '<' {
        return true;
    }
    false
}

/// 2026-10-13: length of the control-token opener at the end of
/// `hold` (so the stripper can release just that prefix back to the
/// caller when the next delta disproves the pattern). Returns 0 if the
/// tail is not a control-token opener.
fn control_prefix_len(hold: &str) -> usize {
    if hold.ends_with(".[") || hold.ends_with(" ]") {
        return 2;
    }
    if hold.ends_with('[') || hold.ends_with('<') || hold.ends_with(']') {
        return 1;
    }
    0
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
        if tags
            .iter()
            .any(|t| suffix.len() < t.len() && t.starts_with(suffix))
        {
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
///
/// Both native and text-fallback paths render the same protocol header so a
/// single system prompt covers every advertised tool. Without them, text-only
/// models guess parameter shapes round-by-round (e.g. `find_symbol` →
/// `query=…`, `read_range` → guessing `start_line` vs. `limit`). The schema
/// block lists every required field, its JSON type, and one valid example per
/// tool — strictly from the registry, never fabricated.
pub fn protocol_instructions(registry: &ToolRegistry) -> String {
    let defs = registry.definitions();
    let externals = registry.external_definitions();
    let mut all_names: Vec<&str> = defs.iter().map(|d| d.name).collect();
    all_names.extend(externals.iter().map(|e| e.name.as_str()));
    // No tools → no protocol (2026-09-29). An "Available tools: ." line
    // teaches a JSON tool habit with nothing behind it and contradicts the
    // conversational arm's "no file or command tools are available".
    if all_names.is_empty() {
        return String::new();
    }
    let header = format!(
        "Tool protocol: prefer native tool calls when available, with public progress in the assistant text.\n\
         If native tools are unavailable, reply with ONLY one JSON object (no prose outside it):\n\
         {{\"commentary\":\"<brief public progress update>\",\"tool_calls\":[{{\"id\":\"1\",\"name\":\"<tool>\",\"arguments\":{{…}}}}]}}\n\
         Available tools: {names}.\n\
         You may put multiple calls in `tool_calls`.\n\
         Use the optional `commentary` field for a short user-facing update, never private reasoning or tool arguments.\n\
         After tools run you will receive an <observations> block with matching ids.\n\
         When finished, answer in prose without JSON tool calls.",
        names = all_names.join(", ")
    );
    let mut schema_lines = Vec::with_capacity(defs.len() + externals.len());
    for def in defs {
        schema_lines.push(render_tool_schema(def));
    }
    for ext in externals {
        schema_lines.push(render_ext_tool_schema(ext));
    }
    let schema_block = schema_lines.join("\n");
    format!("{header}\n\nTool schemas (JSON-fallback path; native calls follow the same shapes):\n{schema_block}")
}

fn render_tool_schema(def: &ToolDefinition) -> String {
    let required = def
        .input_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let example = schema_example(def.name, &def.input_schema);
    let required_suffix = if required.is_empty() {
        String::new()
    } else {
        format!(" — required: {required}")
    };
    let mut line = format!(
        "- {name}: {description}{required_suffix}",
        name = def.name,
        description = def.description.trim_end_matches('.'),
        required_suffix = required_suffix,
    );
    if let Some(example) = example {
        line.push_str(&format!(
            "\n    example: {{\"id\":\"1\",\"name\":\"{name}\",\"arguments\":{example}}}",
            name = def.name
        ));
    }
    line
}

fn render_ext_tool_schema(def: &ExternalToolDef) -> String {
    let required = def
        .input_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let required_suffix = if required.is_empty() {
        String::new()
    } else {
        format!(" — required: {required}")
    };
    format!(
        "- {name} (external): {description}{required_suffix}",
        name = def.name,
        description = def.description.trim_end_matches('.'),
        required_suffix = required_suffix
    )
}

/// Build a single-shot valid example for the schema. The example mirrors the
/// real shape the registry accepts (`additionalProperties: false`) so a model
/// that copies it verbatim succeeds on the first try. Empty string when no
/// reasonable example exists.
fn schema_example(name: &str, schema: &Value) -> Option<String> {
    let obj = schema.get("properties")?.as_object()?;
    let mut sample = serde_json::Map::new();
    let required: std::collections::BTreeSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let pick_example = |key: &str| -> Value {
        match key {
            "query" => Value::String("router".into()),
            "command" => Value::String("git status --short".into()),
            "path" => Value::String("README.md".into()),
            "content" => Value::String("# project\n".into()),
            "old" => Value::String("fn old_name".into()),
            "new" => Value::String("fn new_name".into()),
            "new_text" => Value::String("fn new_name".into()),
            "name" => Value::String("AgentMachine".into()),
            "prefix" => Value::String("crates/".into()),
            "question" => Value::String("Use module A or B?".into()),
            "options" => Value::Array(vec![Value::String("A".into()), Value::String("B".into())]),
            "start_line" => Value::Number(1.into()),
            "limit" => Value::Number(400.into()),
            "offset" => Value::Number(120.into()),
            "max_results" => Value::Number(8.into()),
            _ => Value::String("x".into()),
        }
    };
    for (key, _) in obj.iter() {
        if required.contains(key.as_str()) || obj.len() == 1 {
            sample.insert(key.clone(), pick_example(key));
        }
    }
    if sample.is_empty() {
        // No required fields; emit one canonical example anyway so the model
        // sees the exact envelope.
        let canonical = canonical_example(name)?;
        return Some(canonical);
    }
    let json = serde_json::to_string(&Value::Object(sample)).ok()?;
    Some(json)
}

/// Canonical full-shape example for tools whose `properties` is empty / the
/// model otherwise needs a concrete reference.
fn canonical_example(name: &str) -> Option<String> {
    let body = match name {
        "search" => r#"{"query":"router"}"#,
        "read_file" => r#"{"path":"README.md"}"#,
        "write_file" => "{\"path\":\"notes/a.md\",\"content\":\"hello\\n\"}",
        "run_command" => r#"{"command":"git status --short"}"#,
        "apply_patch" => r#"{"path":"src/lib.rs","old":"fn old_name()","new":"fn new_name()"}"#,
        "replace_range" => {
            r#"{"path":"src/lib.rs","start_line":10,"end_line":12,"new_text":"// fixed"}"#
        }
        "create_file" => "{\"path\":\"new.md\",\"content\":\"# new\\n\"}",
        "delete_file" => r#"{"path":"notes/a.md"}"#,
        "list_files" => r#"{"prefix":"crates/"}"#,
        "find_symbol" => r#"{"name":"AgentMachine"}"#,
        "find_references" => r#"{"name":"AgentMachine"}"#,
        "read_range" => r#"{"path":"crates/agent/src/lib.rs","start_line":1,"end_line":80}"#,
        "ask_user" => r#"{"question":"Which framework?"}"#,
        "websearch" => r#"{"query":"kodo agent finalization"}"#,
        _ => return None,
    };
    Some(body.to_owned())
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
        let inv = reg.accept(
            ToolCallId::new("2"),
            "mcp__other__tool",
            &serde_json::json!({}),
        );
        match inv {
            ToolInvocation::Rejected(reject) => {
                assert_eq!(reject.error.code, ToolErrorCode::UnknownTool);
                assert_eq!(reject.name, "mcp__other__tool");
            }
            other => panic!("expected rejection, got {other:?}"),
        }
        // And with no externals registered at all.
        let inv = registry().accept(
            ToolCallId::new("3"),
            "mcp__fake__echo",
            &serde_json::json!({}),
        );
        assert!(matches!(inv, ToolInvocation::Rejected(_)));
    }

    #[test]
    fn filtered_removes_disallowed_externals() {
        let reg = registry_with_external().filtered(|name| name == "read_file");
        assert_eq!(reg.definitions().len(), 1);
        assert_eq!(reg.external_definitions().len(), 0);
        assert!(!reg.contains_name("mcp__fake__echo"));
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
        assert!(
            text.is_empty(),
            "empty registry must render no protocol: {text:?}"
        );
        assert!(!text.contains("Available tools"));
    }

    /// 2026-10-13 regression: the text-only fallback must enumerate every
    /// required field per tool, otherwise the model guesses parameter names
    /// (`find_symbol → query=…`, `read_range → limit=…`) round-by-round.
    #[test]
    fn protocol_instructions_lists_required_fields_and_examples() {
        let text = protocol_instructions(&ToolRegistry::standard());
        for (tool, needle) in [
            ("search", "required: query"),
            ("read_file", "required: path"),
            ("run_command", "required: command"),
            ("find_symbol", "required: name"),
            ("find_references", "required: name"),
            ("read_range", "required: path, start_line, end_line"),
            ("replace_range", "required: path, start_line, end_line"),
            ("ask_user", "required: question"),
        ] {
            assert!(
                text.contains(needle),
                "fallback prompt must list `{needle}` for {tool}: missing in {text}"
            );
        }
        // Spot-check at least one canonical example per affected tool.
        for needle in [
            "\"query\":\"router\"",
            "\"start_line\":1",
            "\"name\":\"AgentMachine\"",
            "\"command\":\"git status --short\"",
        ] {
            assert!(
                text.contains(needle),
                "fallback prompt example missing for {needle}"
            );
        }
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

    /// 2026-10-13 regression: preview channels (notably `MiniMax-M3`) leak
    /// private `<|…|>` control tokens into the answer tail. They must never
    /// reach the user-facing reply; the stripper scrubs both think blocks
    /// and these markers in one pass.
    #[test]
    fn strip_think_blocks_drops_preview_channel_control_tokens() {
        // Bare trailing token: the leading CJK period is sentence
        // punctuation (not in the marker set), so it stays.
        let (text, _) = strip_think_blocks("评审结论已交付。<|minimax|>[");
        assert_eq!(text, "评审结论已交付。");

        // Token wrapped in a `.[<|…|>[` template bracket. The leading
        // `.` and `[` are the chat template's padding, so the scrubber
        // eats both along with the token. The user's intended answer is
        // `answer is 8` (no period); the period was never the model's
        // own — it was the template's separator. The risk of eating
        // a real sentence-final period (`answer is 8..[<|…`) is
        // accepted because models almost never end a sentence with
        // `.` immediately before a chat-template marker.
        let (text, _) = strip_think_blocks("answer is 8.[<|minimax|>[");
        assert_eq!(text, "answer is 8");

        // Multiple control tokens back-to-back.
        let (text, _) = strip_think_blocks("done <|minimax|>[<|minimax|>[<|minimax|>[");
        assert_eq!(text, "done");

        // Control token inside a think block: think block wins, the
        // remaining plain text still has its trailing token scrubbed.
        let (text, saw) = strip_think_blocks("a<think>secret<|inner|>end</think>b.<|minimax|>[");
        assert!(saw);
        // The leading `.` is consumed as a separator and the trailing `[`
        // is consumed as a closing bracket — the visible remainder is `ab`.
        assert_eq!(text, "ab");
    }

    #[test]
    fn minimax_bracket_marker_is_removed_at_every_stream_boundary() {
        let wire = "下一步聚焦这些信任边界。]<]minimax[>[";
        assert_eq!(strip_think_blocks(wire).0, "下一步聚焦这些信任边界。");
        for split in (0..=wire.len()).filter(|&split| wire.is_char_boundary(split)) {
            let mut stripper = ThinkStripper::default();
            let mut out = stripper.push(&wire[..split]);
            out.push_str(&stripper.push(&wire[split..]));
            out.push_str(&stripper.finish());
            assert_eq!(out, "下一步聚焦这些信任边界。", "split={split}");
        }
        let mut stripper = ThinkStripper::default();
        let mut out = String::new();
        for ch in wire.chars() {
            out.push_str(&stripper.push(&ch.to_string()));
        }
        out.push_str(&stripper.finish());
        assert_eq!(out, "下一步聚焦这些信任边界。");
        let literal = "数组 arr[0] 和 <]other[> 示例";
        assert_eq!(strip_think_blocks(literal).0, literal);
    }

    /// 2026-10-13 (third round) regression: the `.[<|minimax|>[` glitch
    /// is split across SSE deltas. The first delta is `.[`, the second is
    /// `<|minimax|`, the third is `>[`. A naive per-delta scrub cannot
    /// retract the already-emitted `.[`. The fix holds back any tail that
    /// could grow into a control token until the next delta either
    /// completes it (the scrubber drops the whole thing) or disproves it
    /// (we re-emit). The contract: once the whole pattern is on the wire,
    /// the user sees the answer WITHOUT the marker and WITHOUT the
    /// leading `.[` the template padded it with. Real sentence-final
    /// periods (`。`, `!.`, `?`) stay — only the template's literal
    /// `.[`/`]/,`/`];` framing is pulled back.
    #[test]
    fn think_stripper_holds_back_cross_delta_control_token_prefix() {
        let mut s = ThinkStripper::default();
        // Delta 1: prose ends in `.[`. The stripper holds `.[` back as a
        // potential control-token opener. Visible output is the prose
        // WITHOUT the trailing `.[`.
        let visible1 = s.push("已锁定这些关键函数.[");
        assert_eq!(visible1, "已锁定这些关键函数");

        // Delta 2: `<|minimax|` confirms the marker is real. The
        // stripper pulls the held `.[` together with this delta into
        // `hold` so the eventual scrubber can drop the whole thing.
        let visible2 = s.push("<|minimax|");
        assert_eq!(visible2, "");

        // Delta 3: `>[` closes the template. The whole thing is now in
        // `hold`; on `finish()` the scrubber drops it.
        let visible3 = s.push(">[");
        assert_eq!(visible3, "");
        let tail = s.finish();
        assert!(
            tail.is_empty(),
            "control token + leading punctuation must be scrubbed from the final output: got {tail:?}"
        );
    }

    /// 2026-10-13 (third round) regression: when a tail *looks like* a
    /// control-token prefix but the next delta is just normal text, we
    /// must re-emit the held bytes verbatim — never silently swallow real
    /// user-facing content.
    #[test]
    fn think_stripper_reemits_held_bytes_when_next_delta_is_plain() {
        let mut s = ThinkStripper::default();
        let visible1 = s.push("正文 a.[");
        // `.[` is held back because it *could* be the start of a control
        // token. Real content is everything before the hold.
        assert_eq!(visible1, "正文 a");

        // Next delta is plain text — the `[` was just punctuation. The
        // hold releases `.[`, collapses its trailing `[` with the delta's
        // leading `[` (one bracket, not two), and the next delta joins
        // verbatim.
        let visible2 = s.push("[后面的内容");
        let combined = format!("{visible1}{visible2}{}", s.finish());
        assert_eq!(combined, "正文 a.[后面的内容");
    }

    /// 2026-10-13 (third round, second regression): the chat template's
    /// ` ]<|minimax|>[` variant (a space + `]` + marker instead of the
    /// `.[<|minimax|>[` we previously regressed against) must also be
    /// scrubbed. The leading separator set must include `]` for the
    /// scrubber to eat the `]` and the space before `<|minimax|>`.
    #[test]
    fn think_stripper_scrubs_space_bracket_control_token_variant() {
        // Single-delta: full marker in one chunk.
        let mut s = ThinkStripper::default();
        let v = s.push("调用端消费。 ]<|minimax|>[");
        let t = s.finish();
        let combined = format!("{v}{t}");
        assert_eq!(
            combined, "调用端消费。",
            "single-delta  ]<|minimax|>[  must be scrubbed, leaving only the CJK period: got {combined:?}"
        );

        // 3-delta: `]` is its own delta, the marker is split across two more.
        let mut s = ThinkStripper::default();
        let v1 = s.push("调用端消费。");
        let v2 = s.push(" ]");
        let v3 = s.push("<|minimax|");
        let v4 = s.push(">[");
        let t = s.finish();
        let combined = format!("{v1}{v2}{v3}{v4}{t}");
        assert_eq!(
            combined, "调用端消费。",
            "3-delta split  ]<|minimax|>[  must be scrubbed: got {combined:?}"
        );
    }

    /// 2026-10-13 (fourth round) end-to-end replay: simulate the SSE
    /// chunking that the screenshot of tmu34a-6disnc-0 produced.
    /// Realistic chunking groups `|>` together (SSE token boundaries
    /// fall between semantic units, not inside `<|minimax|>`). Drives
    /// the full `ThinkStripper` state machine (defer → hold → scrub)
    /// the same way the production `provider.rs` SSE loop does, and
    /// asserts the user-visible answer never contains the chat-
    /// template private marker.
    #[test]
    fn replay_minimax_m3_close_bracket_variant_end_to_end() {
        // Realistic SSE chunking for MiniMax-M3.1-Flash-Preview final
        // answer: the answer text and the template noise land in
        // separate chunks; `|>` stays together because SSE tokenizers
        // split between semantic units, not inside a single template
        // marker.
        let chunks: Vec<&str> = vec![
            "调用端消费。",
            " ]<|minimax|>", // answer text + template prefix + full marker
        ];

        let mut s = ThinkStripper::default();
        let mut out = String::new();
        for chunk in &chunks {
            out.push_str(&s.push(chunk));
        }
        out.push_str(&s.finish());

        assert_eq!(
            out, "调用端消费。",
            "realistic chunking of close-bracket variant must scrub the entire ` ]<|minimax|>` block: got {out:?}"
        );
        assert!(
            !out.contains("<|"),
            "any `<|` substring means the scrubber let the marker through: {out:?}"
        );
        assert!(
            !out.contains("]"),
            "trailing `]` must be dropped when preceded by a real control token: {out:?}"
        );
        assert!(
            !out.contains("minimax"),
            "provider name must never leak to the user-facing answer: {out:?}"
        );
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
                assert!(
                    matches!(&calls[0], ToolInvocation::Ready(c) if c.name.label() == "list_files")
                );
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
        let err = ToolArgs::from_json(ToolName::Search, &v).expect_err("missing query must reject");
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
        let err =
            ToolArgs::from_json(ToolName::AskUser, &v).expect_err("missing question must reject");
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
