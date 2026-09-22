//! Typed tool protocol between the model and the agent loop.
//!
//! The agent loop only sees [`ToolInvocation`] values. Wire formats (strict
//! JSON fallback, native provider tool calls, deprecated Markdown fences)
//! are converted here so business logic never walks raw `Value` maps.

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
}

impl ToolArgs {
    fn from_json(name: ToolName, args: &Value) -> Result<Self, ToolError> {
        match name {
            ToolName::Search => Ok(Self::Search {
                query: require_string(args, "query")?,
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
        Self {
            code,
            message: message.into(),
        }
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

/// One accepted or rejected call from a single model turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolInvocation {
    Ready(ToolCall),
    Rejected(RejectedCall),
}

impl ToolInvocation {
    pub fn id(&self) -> &ToolCallId {
        match self {
            Self::Ready(call) => &call.id,
            Self::Rejected(reject) => &reject.id,
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
        Self {
            id,
            name: name.into(),
            input: input.into(),
            output: output.into(),
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
        let output = error.message.clone();
        Self {
            id,
            name: name.into(),
            input: input.into(),
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

/// The fixed set of tools this agent exposes.
#[derive(Debug, Clone, Default)]
pub struct ToolRegistry {
    defs: Vec<ToolDefinition>,
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
            ],
        }
    }

    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.defs
    }

    /// Keep only definitions whose name passes `keep` (skill allowlists).
    pub fn filtered<F: Fn(&str) -> bool>(&self, keep: F) -> Self {
        Self {
            defs: self.defs.iter().filter(|d| keep(d.name)).cloned().collect(),
        }
    }

    pub fn find(&self, name: &str) -> Option<&ToolDefinition> {
        self.defs.iter().find(|d| d.name == name)
    }

    pub fn contains_name(&self, name: &str) -> bool {
        ToolName::parse(name).is_some_and(|n| self.find(n.label()).is_some())
    }

    /// Validate a raw wire call against the registry → typed args.
    pub fn accept(&self, id: ToolCallId, raw_name: &str, args: &Value) -> ToolInvocation {
        let trimmed = raw_name.trim();
        let Some(name) = ToolName::parse(trimmed) else {
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

/// Full text → model turn: JSON protocol first, then deprecated fences.
///
/// Malformed JSON that is *almost* a tool protocol falls through to the fence
/// parser; if neither yields tools, the original text is the final answer.
pub fn parse_model_turn(text: &str, registry: &ToolRegistry) -> ModelTurn {
    // 1) Strict JSON fallback protocol
    if looks_like_json_tool_protocol(text) {
        match parse_json_protocol(text) {
            Ok(wire) => {
                return ModelTurn::Tools {
                    calls: invocations_from_wire(registry, wire),
                };
            }
            Err(_) => {
                // Malformed tool JSON — recover via fence parser, else treat as text.
            }
        }
    }

    // 2) Deprecated Markdown fence compatibility layer
    let fence_calls = parse_fence_invocations(text, registry);
    if !fence_calls.is_empty() {
        return ModelTurn::Tools { calls: fence_calls };
    }

    // 3) If the text was clearly an attempted JSON tool call that failed hard,
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
            ToolInvocation::Rejected(_) => None,
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
    let names: Vec<&str> = registry.definitions().iter().map(|d| d.name).collect();
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
}
