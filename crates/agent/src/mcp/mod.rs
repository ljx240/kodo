//! MCP (Model Context Protocol) thin client.
//!
//! External tool servers are configured in `settings.log` under `mcp-servers`
//! and their secret values (env / header values) live in `credentials.log`
//! under `mcp.{id}` — the same split the providers use. This module owns that
//! data model, the secret split/merge, and the transport seam; the stdio /
//! Streamable HTTP transports live in the sibling modules.

pub mod http;
pub mod jsonrpc;
pub mod stdio;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The protocol revision this client speaks. We pin instead of negotiating
/// and accept the server's echo of this value in `initialize`.
pub const PROTOCOL_VERSION: &str = "2026-07-28";

/// Why an MCP exchange failed.
///
/// Everything degrades to a zh-CN note at the run level — a broken server
/// never fails the whole run — so this type only needs to be displayable and
/// classifiable, never serialized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpError {
    /// The connection is gone: spawn failure, dead child, IO error, timeout.
    Transport(String),
    /// The server answered with a JSON-RPC `error` object.
    Rpc(jsonrpc::ErrorObject),
    /// The envelope was unusable: not JSON, wrong shape, bad version.
    Protocol(String),
    /// The server answered with a non-success HTTP status. The status stays
    /// attached so the run layer can map 401/403 to a permission failure.
    Http(u16, String),
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpError::Transport(message) => write!(f, "transport: {message}"),
            McpError::Rpc(error) => write!(f, "rpc: {} {}", error.code, error.message),
            McpError::Protocol(message) => write!(f, "protocol: {message}"),
            McpError::Http(status, message) => write!(f, "http {status}: {message}"),
        }
    }
}

/// The seam between the MCP client and a concrete transport. Synchronous by
/// design — the whole stack is blocking — and one request at a time: `run()`
/// dispatches tool calls sequentially, so transports need no multiplexing.
pub trait McpTransport {
    /// Sends a request and blocks until its response arrives. The transport
    /// owns id generation and correlation; the value inside the envelope's
    /// `result` is what comes back.
    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, McpError>;

    /// Fire-and-forget notification (`notifications/initialized`, …).
    fn notify(&mut self, method: &str, params: serde_json::Value) -> Result<(), McpError>;

    /// False once the connection is gone; the client stops using it.
    fn alive(&self) -> bool;
}

/// How an MCP server is reached: a persistent local process speaking
/// JSON-RPC over stdio, or a Streamable HTTP endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransportKind {
    Stdio,
    Http,
}

/// One configured MCP server.
///
/// Secret values never live in `settings.log`: `env` / `headers` values are
/// stored as empty strings there (keys kept, so the UI knows the shape) and
/// the real values live in `credentials.log` as one JSON blob per server.
/// [`split_secrets`] / [`merge_secrets`] move between the two views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    /// Stable id — the credentials key (`mcp.{id}`) and half the wire name
    /// (`mcp__{id}__{tool}`).
    pub id: String,
    /// Display name for the settings list.
    pub name: String,
    /// Disabled servers stay configured but are never contacted. Default
    /// false: an external server is opt-in.
    #[serde(default)]
    pub enabled: bool,
    pub transport: McpTransportKind,
    /// stdio only: the executable to spawn.
    #[serde(default)]
    pub command: String,
    /// stdio only: argv after the executable.
    #[serde(default)]
    pub args: Vec<String>,
    /// stdio only: child environment additions (values are secrets).
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// http only: the Streamable HTTP endpoint.
    #[serde(default)]
    pub url: String,
    /// http only: extra request headers (values are secrets).
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// The secret halves of one server config — what `credentials.log` holds
/// under `mcp.{id}` as a JSON blob.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpSecrets {
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// Splits the secret values out of `config` (env and header values).
pub fn split_secrets(config: &McpServerConfig) -> McpSecrets {
    McpSecrets {
        env: config.env.clone(),
        headers: config.headers.clone(),
    }
}

/// Fills `config`'s secret values from `secrets` (the connect-time view).
/// Keys present in both prefer `secrets`; keys only in `config` keep their
/// value. Missing secrets become empty strings.
pub fn merge_secrets(config: &McpServerConfig, secrets: &McpSecrets) -> McpServerConfig {
    let mut out = config.clone();
    for map in [&mut out.env, &mut out.headers] {
        for value in map.values_mut() {
            value.clear();
        }
    }
    for (key, value) in &secrets.env {
        out.env.insert(key.clone(), value.clone());
    }
    for (key, value) in &secrets.headers {
        out.headers.insert(key.clone(), value.clone());
    }
    out
}

/// The settings copy: every secret value replaced by `mask` (the settings log
/// stores empty strings; the webview sees the UI mask).
pub fn with_secret_values(config: &McpServerConfig, mask: &str) -> McpServerConfig {
    let mut out = config.clone();
    for map in [&mut out.env, &mut out.headers] {
        for value in map.values_mut() {
            *value = mask.to_owned();
        }
    }
    out
}

/// True when an incoming secret value is a placeholder (mask or blank) the
/// save path must replace with the already-stored value.
pub fn is_masked_value(value: &str) -> bool {
    value.is_empty() || value.contains('•')
}

/// True when this server has at least one non-empty secret value.
pub fn has_secrets(secrets: &McpSecrets) -> bool {
    secrets
        .env
        .values()
        .chain(secrets.headers.values())
        .any(|value| !value.is_empty())
}

/// One tool as advertised by a server (`tools/list` item, raw MCP shape —
/// wire naming happens at the registry seam).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The tool's JSON Schema (`inputSchema`), passed through as-is.
    #[serde(default)]
    pub input_schema: serde_json::Value,
}

/// What a `tools/call` produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolResult {
    /// `content[]` text blocks joined by newlines. Non-text blocks (images,
    /// embedded resources) are non-goals and skipped.
    pub text: String,
    /// The server marked the call failed (`isError: true`) — the run layer
    /// turns this into a tool error, not a transport error.
    pub is_error: bool,
}

/// The wire name of one external tool: `mcp__{server_id}__{tool}`. Double
/// underscores because some providers reject `:` in tool names.
pub fn wire_name(server_id: &str, tool: &str) -> String {
    format!("mcp__{server_id}__{tool}")
}

/// Splits a wire name into `(server_id, tool)`. Server ids must not contain
/// `__` — the first double underscore after the prefix is the separator;
/// tool names may keep theirs.
pub fn split_wire_name(wire: &str) -> Option<(&str, &str)> {
    let rest = wire.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    (!server.is_empty() && !tool.is_empty()).then_some((server, tool))
}

/// One server, handshake done, tool list cached for the run. Created once per
/// `run()` and dropped with it — transports own their children/connections.
pub struct McpClient {
    server_id: String,
    #[allow(dead_code)]
    name: String,
    transport: Box<dyn McpTransport>,
    /// Cached `tools/list` (all pages). Never refetched within a run.
    tools: Vec<McpTool>,
}

impl McpClient {
    /// Spawns the configured transport and performs the full handshake:
    /// `initialize` → `notifications/initialized` → `tools/list` (all pages).
    pub fn connect(config: &McpServerConfig) -> Result<Self, McpError> {
        let transport: Box<dyn McpTransport> = match config.transport {
            McpTransportKind::Stdio => Box::new(stdio::StdioTransport::spawn(config)?),
            McpTransportKind::Http => Box::new(http::HttpTransport::spawn(config)?),
        };
        Self::handshake(&config.id, &config.name, transport)
    }

    /// The handshake over an already-built transport — the test seam (and the
    /// only place that knows the protocol order).
    pub fn handshake(
        server_id: &str,
        name: &str,
        mut transport: Box<dyn McpTransport>,
    ) -> Result<Self, McpError> {
        transport.request(
            "initialize",
            serde_json::json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "kodo", "version": env!("CARGO_PKG_VERSION")},
            }),
        )?;
        // The server's echo of `protocolVersion` is accepted as-is: we pin
        // one revision and treat compatible servers as interchangeable.
        transport.notify("notifications/initialized", serde_json::json!({}))?;
        let tools = fetch_tools(transport.as_mut())?;
        Ok(Self {
            server_id: server_id.to_owned(),
            name: name.to_owned(),
            transport,
            tools,
        })
    }

    /// The cached tool list (all pages merged).
    pub fn tools(&self) -> &[McpTool] {
        &self.tools
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    /// Calls one tool by its bare name (not the wire name) and flattens the
    /// result. `isError: true` is carried in the result, not as an `Err`.
    pub fn call_tool(
        &mut self,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult, McpError> {
        let result = self.transport.request(
            "tools/call",
            serde_json::json!({"name": tool, "arguments": arguments}),
        )?;
        Ok(flatten_content(&result))
    }
}

/// Walks `tools/list` with `nextCursor` until exhausted. A server that pages
/// forever is cut off with a protocol error rather than looping.
fn fetch_tools(transport: &mut dyn McpTransport) -> Result<Vec<McpTool>, McpError> {
    const MAX_PAGES: usize = 32;
    let mut tools = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let params = match &cursor {
            Some(cursor) => serde_json::json!({"cursor": cursor}),
            None => serde_json::json!({}),
        };
        let page = transport.request("tools/list", params)?;
        let items = page.get("tools").and_then(serde_json::Value::as_array).ok_or_else(|| {
            McpError::Protocol("tools/list result has no tools array".to_owned())
        })?;
        for item in items {
            let tool: McpTool = serde_json::from_value(item.clone()).map_err(|error| {
                McpError::Protocol(format!("unusable tool entry: {error}"))
            })?;
            tools.push(tool);
        }
        cursor = page
            .get("nextCursor")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        if cursor.is_none() {
            return Ok(tools);
        }
    }
    Err(McpError::Protocol("tools/list paginates forever".to_owned()))
}

/// Flattens a `tools/call` result: `content[]` text blocks joined by newline,
/// `isError` flagged. Everything else in the result object is ignored.
fn flatten_content(result: &serde_json::Value) -> McpToolResult {
    let text = result
        .get("content")
        .and_then(serde_json::Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(serde_json::Value::as_str) == Some("text"))
                .filter_map(|block| {
                    block
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    McpToolResult {
        text,
        is_error: result
            .get("isError")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    }
}

/// Every MCP server for one run. Connecting is all-or-nothing per server: a
/// broken one becomes a zh-CN note and the run continues without it.
#[derive(Default)]
pub struct McpClients {
    clients: Vec<McpClient>,
    /// zh-CN notes for servers that failed to connect, ready to surface.
    pub notes: Vec<String>,
}

/// An argv value still carrying the catalog's `/path/to/…` placeholder — the
/// user saved the template without picking a real directory or file.
fn placeholder_arg(config: &McpServerConfig) -> Option<&str> {
    config
        .args
        .iter()
        .find(|arg| arg.starts_with("/path/to/"))
        .map(String::as_str)
}

impl McpClients {
    /// Connects each **enabled** server in configuration order.
    pub fn connect(configs: &[McpServerConfig]) -> Self {
        let mut out = Self::default();
        for config in configs.iter().filter(|config| config.enabled) {
            if let Some(placeholder) = placeholder_arg(config) {
                // The connector catalog ships `/path/to/…` argv the user must
                // replace; spawning it only yields a server that exits on the
                // first exchange, so say what to fix instead of surfacing a
                // transport error on every run (2026-09-30).
                out.notes.push(format!(
                    "MCP 服务器「{}」配置仍是模板占位值（{placeholder}），未启动；请在设置中改成真实路径",
                    config.name
                ));
                continue;
            }
            match McpClient::connect(config) {
                Ok(client) => out.clients.push(client),
                Err(error) => out.push_note(&config.name, &error),
            }
        }
        out
    }

    /// Direct assembly from already-handshaken clients — the test seam.
    #[cfg(test)]
    pub(crate) fn from_clients(clients: Vec<McpClient>, notes: Vec<String>) -> Self {
        Self { clients, notes }
    }

    fn push_note(&mut self, server_name: &str, error: &McpError) {
        self.notes.push(format!(
            "MCP 服务器「{server_name}」连接失败：{}",
            crate::tools::redact_secrets(&error.to_string())
        ));
    }

    /// True when no server is connected — the run can skip the whole wiring.
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// Single zh-CN line summarising which configured servers did not
    /// start (placeholder template / transport failure). Ready servers
    /// are omitted — no news = good news. Empty string when everything
    /// is up.
    ///
    /// Surfaced once via the system prompt (see `mcp_status_hint()` in
    /// `lib.rs`) so the model can acknowledge MCP gaps up-front instead
    /// of re-discovering them mid-run and echoing them in the final
    /// "已执行" list.
    pub fn status_hint(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for note in &self.notes {
            lines.push(note.clone());
        }
        lines.join("\n")
    }

    /// Every cached tool across connected servers, paired with its wire name,
    /// in server configuration order.
    pub fn tools(&self) -> Vec<(String, McpTool)> {
        self.clients
            .iter()
            .flat_map(|client| {
                client.tools.iter().map(|tool| {
                    (wire_name(&client.server_id, &tool.name), tool.clone())
                })
            })
            .collect()
    }

    /// Routes a wire-named call to its owning server and flattens the result.
    pub fn call(
        &mut self,
        wire: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult, McpError> {
        let (server_id, tool) = split_wire_name(wire)
            .ok_or_else(|| McpError::Protocol(format!("bad wire name {wire}")))?;
        let client = self
            .clients
            .iter_mut()
            .find(|client| client.server_id == server_id)
            .ok_or_else(|| McpError::Transport(format!("no MCP server {server_id}")))?;
        client.call_tool(tool, arguments)
    }
}

/// Scripted transport for tests: answers each method from a per-method queue
/// of canned replies and records the whole conversation, so a test can assert
/// both what went out and what came back. Visible crate-wide under `cfg(test)`
/// — `run()` integration tests script the handshake and tool calls through it.
#[cfg(test)]
pub struct FakeTransport {
    /// One reply per call, consumed front to back. The last reply is *not*
    /// sticky: a method called more often than scripted fails loudly.
    replies: BTreeMap<String, Vec<Result<serde_json::Value, McpError>>>,
    /// Every `(method, params)` passed to `request`, in order.
    pub requests: Vec<(String, serde_json::Value)>,
    /// Every `method` passed to `notify`, in order.
    pub notifications: Vec<String>,
    /// Every method passed to `request`/`notify`, in order — clone this
    /// before the transport is boxed into an [`McpClient`] to still see the
    /// conversation afterwards.
    pub log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    alive: bool,
}

#[cfg(test)]
impl Default for FakeTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl FakeTransport {
    /// A transport with no scripted replies (every request errors).
    pub fn new() -> Self {
        Self {
            replies: BTreeMap::new(),
            requests: Vec::new(),
            notifications: Vec::new(),
            log: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            alive: true,
        }
    }

    /// Queues a successful reply for `method`. Order of `reply` calls for the
    /// same method is the order of `request` calls.
    pub fn reply(mut self, method: &str, result: serde_json::Value) -> Self {
        self.replies
            .entry(method.to_owned())
            .or_default()
            .push(Ok(result));
        self
    }

    /// Queues a failing reply for `method`.
    pub fn fail(mut self, method: &str, error: McpError) -> Self {
        self.replies
            .entry(method.to_owned())
            .or_default()
            .push(Err(error));
        self
    }

    /// Simulates a dropped connection (`alive` goes false).
    pub fn kill(&mut self) {
        self.alive = false;
    }
}

#[cfg(test)]
impl McpTransport for FakeTransport {
    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, McpError> {
        self.requests.push((method.to_owned(), params));
        self.log.lock().expect("log").push(method.to_owned());
        let queue = self
            .replies
            .get_mut(method)
            .filter(|queue| !queue.is_empty())
            .ok_or_else(|| McpError::Transport(format!("no canned reply for {method}")))?;
        queue.remove(0)
    }

    fn notify(&mut self, method: &str, _params: serde_json::Value) -> Result<(), McpError> {
        self.notifications.push(method.to_owned());
        self.log.lock().expect("log").push(method.to_owned());
        Ok(())
    }

    fn alive(&self) -> bool {
        self.alive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> McpServerConfig {
        McpServerConfig {
            id: "fs".to_owned(),
            name: "Filesystem".to_owned(),
            enabled: true,
            transport: McpTransportKind::Stdio,
            command: "npx".to_owned(),
            args: vec!["-y".to_owned(), "@modelcontextprotocol/server-filesystem".to_owned()],
            env: BTreeMap::from([("TOKEN".to_owned(), "t-1234".to_owned())]),
            url: String::new(),
            headers: BTreeMap::new(),
        }
    }

    #[test]
    fn serde_roundtrip_keeps_camel_case_shape() {
        let config = sample();
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(json.contains("\"transport\":\"stdio\""));
        assert!(json.contains("\"args\""));
        assert_eq!(
            serde_json::from_str::<McpServerConfig>(&json).expect("deserialize"),
            config
        );
    }

    #[test]
    fn disabled_by_default_when_the_flag_is_absent() {
        let json = r#"{"id":"x","name":"X","transport":"http","url":"http://127.0.0.1:1/mcp"}"#;
        let config: McpServerConfig = serde_json::from_str(json).expect("deserialize");
        assert!(!config.enabled);
        assert_eq!(config.transport, McpTransportKind::Http);
    }

    #[test]
    fn split_and_merge_move_secret_values() {
        let config = sample();
        let secrets = split_secrets(&config);
        assert_eq!(secrets.env.get("TOKEN").map(String::as_str), Some("t-1234"));

        let settings_copy = with_secret_values(&config, "");
        assert_eq!(settings_copy.env.get("TOKEN").map(String::as_str), Some(""));
        assert!(
            !serde_json::to_string(&settings_copy)
                .expect("serialize")
                .contains("t-1234"),
            "the settings copy must not carry the secret"
        );

        let merged = merge_secrets(&settings_copy, &secrets);
        assert_eq!(merged, config);

        // A key the blob knows nothing about ends up empty, not stale.
        let mut with_extra = settings_copy.clone();
        with_extra.env.insert("EXTRA".to_owned(), "••••34".to_owned());
        let merged = merge_secrets(&with_extra, &secrets);
        assert_eq!(merged.env.get("EXTRA").map(String::as_str), Some(""));
        assert_eq!(merged.env.get("TOKEN").map(String::as_str), Some("t-1234"));
    }

    #[test]
    fn masked_values_are_recognized() {
        assert!(is_masked_value(""));
        assert!(is_masked_value("••••34"));
        assert!(!is_masked_value("t-1234"));
        assert!(has_secrets(&split_secrets(&sample())));
        assert!(!has_secrets(&McpSecrets::default()));
    }

    #[test]
    fn fake_transport_scripts_replies_in_order_and_records_the_conversation() {
        let mut transport = FakeTransport::new()
            .reply("tools/list", serde_json::json!({"tools": []}))
            .reply("tools/list", serde_json::json!({"tools": ["second"]}))
            .reply("tools/call", serde_json::json!({"content": []}))
            .fail("initialize", McpError::Transport("boom".to_owned()));

        assert_eq!(
            transport
                .request("tools/list", serde_json::json!({}))
                .expect("first page"),
            serde_json::json!({"tools": []})
        );
        assert_eq!(
            transport
                .request("tools/list", serde_json::json!({"cursor": "1"}))
                .expect("second page"),
            serde_json::json!({"tools": ["second"]})
        );
        // The queue is not sticky: a third call fails loudly.
        assert!(transport.request("tools/list", serde_json::json!({})).is_err());

        let error = transport
            .request("initialize", serde_json::json!({}))
            .expect_err("scripted failure");
        assert_eq!(error, McpError::Transport("boom".to_owned()));

        transport
            .notify("notifications/initialized", serde_json::json!({}))
            .expect("notify");
        assert_eq!(
            transport.requests,
            vec![
                ("tools/list".to_owned(), serde_json::json!({})),
                (
                    "tools/list".to_owned(),
                    serde_json::json!({"cursor": "1"})
                ),
                ("tools/list".to_owned(), serde_json::json!({})),
                ("initialize".to_owned(), serde_json::json!({})),
            ]
        );
        assert_eq!(transport.notifications, vec!["notifications/initialized"]);
    }

    #[test]
    fn fake_transport_reports_unscripted_methods_and_death() {
        let mut transport = FakeTransport::new();
        assert!(transport.alive());
        let error = transport
            .request("tools/call", serde_json::json!({}))
            .expect_err("unscripted");
        assert_eq!(
            error,
            McpError::Transport("no canned reply for tools/call".to_owned())
        );
        transport.kill();
        assert!(!transport.alive());
    }

    #[test]
    fn mcp_error_displays_classified_messages() {
        assert_eq!(
            McpError::Transport("child exited".to_owned()).to_string(),
            "transport: child exited"
        );
        assert_eq!(
            McpError::Rpc(jsonrpc::ErrorObject {
                code: -32601,
                message: "Method not found".to_owned(),
                data: None,
            })
            .to_string(),
            "rpc: -32601 Method not found"
        );
        assert_eq!(
            McpError::Protocol("bad json".to_owned()).to_string(),
            "protocol: bad json"
        );
        assert_eq!(
            McpError::Http(401, "no token".to_owned()).to_string(),
            "http 401: no token"
        );
    }

    #[test]
    fn handshake_follows_the_protocol_order_and_caches_every_page() {
        let transport = FakeTransport::new()
            .reply(
                "initialize",
                serde_json::json!({"protocolVersion": PROTOCOL_VERSION}),
            )
            .reply(
                "tools/list",
                serde_json::json!({
                    "tools": [{"name": "echo", "description": "echo", "inputSchema": {"type": "object"}}],
                    "nextCursor": "page-2"
                }),
            )
            .reply(
                "tools/list",
                serde_json::json!({"tools": [{"name": "store"}]}),
            )
            .reply("tools/call", serde_json::json!({"content": []}));
        let log = transport.log.clone();

        let mut client =
            McpClient::handshake("fs", "FS", Box::new(transport)).expect("handshake");
        // The cache holds both pages — no third tools/list sneak-fetch.
        assert_eq!(
            client.tools().iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            vec!["echo", "store"]
        );
        assert_eq!(client.tools()[0].description, "echo");
        assert_eq!(client.tools()[1].input_schema, serde_json::Value::Null);

        client
            .call_tool("echo", serde_json::json!({"text": "hi"}))
            .expect("call");
        assert_eq!(
            *log.lock().expect("log"),
            vec![
                "initialize",
                "notifications/initialized",
                "tools/list",
                "tools/list",
                "tools/call"
            ]
        );
    }

    #[test]
    fn call_tool_flattens_text_blocks_and_flags_errors() {
        let transport = FakeTransport::new()
            .reply("initialize", serde_json::json!({}))
            .reply("tools/list", serde_json::json!({"tools": []}))
            .reply(
                "tools/call",
                serde_json::json!({
                    "content": [
                        {"type": "text", "text": "first"},
                        {"type": "image", "data": "AAAA"},
                        {"type": "text", "text": "second"}
                    ],
                    "isError": true
                }),
            );
        let mut client =
            McpClient::handshake("fake", "Fake", Box::new(transport)).expect("handshake");
        let result = client
            .call_tool("anything", serde_json::json!({}))
            .expect("call");
        assert_eq!(result.text, "first\nsecond");
        assert!(result.is_error);
    }

    #[test]
    fn a_failing_tools_list_fails_the_handshake() {
        let transport = FakeTransport::new()
            .reply("initialize", serde_json::json!({}))
            .fail("tools/list", McpError::Transport("boom".to_owned()));
        let error = match McpClient::handshake("x", "X", Box::new(transport)) {
            Ok(_) => panic!("a failing tools/list must fail the handshake"),
            Err(error) => error,
        };
        assert_eq!(error, McpError::Transport("boom".to_owned()));
    }

    #[test]
    fn tools_list_paginating_forever_is_cut_off() {
        let mut transport = FakeTransport::new().reply("initialize", serde_json::json!({}));
        for page in 0..40 {
            transport = transport.reply(
                "tools/list",
                serde_json::json!({"tools": [], "nextCursor": format!("p{page}")}),
            );
        }
        let error = match McpClient::handshake("x", "X", Box::new(transport)) {
            Ok(_) => panic!("unbounded pagination must be cut off"),
            Err(error) => error,
        };
        assert!(matches!(error, McpError::Protocol(_)), "got {error:?}");
    }

    #[test]
    fn wire_names_split_on_the_first_double_underscore() {
        assert_eq!(
            split_wire_name(&wire_name("fs", "read_file")),
            Some(("fs", "read_file"))
        );
        assert_eq!(split_wire_name("mcp__fs__deep__tool"), Some(("fs", "deep__tool")));
        assert_eq!(split_wire_name("mcp____tool"), None);
        assert_eq!(split_wire_name("builtin__grep"), None);
    }

    #[test]
    fn wire_names_route_calls_to_the_right_server() {
        let a = McpClient::handshake(
            "a",
            "A",
            Box::new(
                FakeTransport::new()
                    .reply("initialize", serde_json::json!({}))
                    .reply("tools/list", serde_json::json!({"tools": [{"name": "echo"}]}))
                    .reply(
                        "tools/call",
                        serde_json::json!({"content": [{"type": "text", "text": "from-a"}]}),
                    ),
            ),
        )
        .expect("handshake a");
        let b = McpClient::handshake(
            "b",
            "B",
            Box::new(
                FakeTransport::new()
                    .reply("initialize", serde_json::json!({}))
                    .reply("tools/list", serde_json::json!({"tools": [{"name": "store"}]}))
                    .reply(
                        "tools/call",
                        serde_json::json!({"content": [{"type": "text", "text": "from-b"}]}),
                    ),
            ),
        )
        .expect("handshake b");
        let mut clients = McpClients::from_clients(vec![a, b], Vec::new());

        let tools: Vec<String> = clients.tools().into_iter().map(|(wire, _)| wire).collect();
        assert_eq!(tools, vec!["mcp__a__echo", "mcp__b__store"]);

        assert_eq!(
            clients
                .call("mcp__b__store", serde_json::json!({}))
                .expect("call b")
                .text,
            "from-b"
        );
        assert_eq!(
            clients
                .call("mcp__a__echo", serde_json::json!({}))
                .expect("call a")
                .text,
            "from-a"
        );
        let error = clients
            .call("mcp__gone__tool", serde_json::json!({}))
            .expect_err("unknown server");
        assert!(matches!(error, McpError::Transport(_)), "got {error:?}");
    }

    #[test]
    fn a_failed_server_becomes_a_note_not_an_error() {
        // A missing binary fails at spawn; the disabled one is never touched.
        let configs = vec![
            McpServerConfig {
                id: "broken".to_owned(),
                name: "Broken".to_owned(),
                enabled: true,
                transport: McpTransportKind::Stdio,
                command: "kodo-no-such-mcp-binary".to_owned(),
                args: Vec::new(),
                env: BTreeMap::new(),
                url: String::new(),
                headers: BTreeMap::new(),
            },
            McpServerConfig {
                id: "off".to_owned(),
                name: "Off".to_owned(),
                enabled: false,
                transport: McpTransportKind::Http,
                command: String::new(),
                args: Vec::new(),
                env: BTreeMap::new(),
                url: "http://127.0.0.1:1/mcp".to_owned(),
                headers: BTreeMap::new(),
            },
        ];
        let clients = McpClients::connect(&configs);
        assert!(clients.is_empty());
        assert_eq!(clients.notes.len(), 1);
        assert!(
            clients.notes[0].contains("Broken") && clients.notes[0].contains("连接失败"),
            "note: {}",
            clients.notes[0]
        );
    }

    #[test]
    fn placeholder_paths_are_skipped_with_an_actionable_note() {
        // The catalog's filesystem template saved as-is: spawning it would
        // only produce `transport: server closed the connection` on every run.
        let mut config = sample();
        config
            .args
            .push("/path/to/allowed".to_owned());
        let clients = McpClients::connect(&[config]);
        assert!(clients.is_empty(), "no server should be spawned");
        assert_eq!(clients.notes.len(), 1);
        assert!(
            clients.notes[0].contains("模板占位值")
                && clients.notes[0].contains("/path/to/allowed"),
            "note: {}",
            clients.notes[0]
        );
        assert!(!clients.notes[0].contains("连接失败"));
    }

    /// Phase 0 second-pass — `status_hint()` mirrors `notes` for system-prompt
    /// exposure. All notes go through, no filtering (Ready servers never
    /// produce notes in the first place).
    #[test]
    fn status_hint_returns_notes_verbatim_for_system_prompt() {
        let mut config = sample();
        config.args.push("/path/to/allowed".to_owned());
        let clients = McpClients::connect(&[config]);
        let hint = clients.status_hint();
        assert!(hint.contains("Filesystem"));
        assert!(hint.contains("/path/to/allowed"));
    }

    #[test]
    fn status_hint_empty_when_no_notes() {
        let clients = McpClients::default();
        assert!(clients.status_hint().is_empty());
    }
}
