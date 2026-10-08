//! Streamable HTTP transport: one POST per JSON-RPC message against an
//! `http(s)` endpoint. Responses come back as `application/json` (a single
//! message) or `text/event-stream` (possibly interleaved with server
//! notifications). `Mcp-Session-Id` is optional: captured when the server
//! issues one and replayed on later requests; Drop sends the spec's
//! session-terminating DELETE as a best effort.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::jsonrpc::{self, Message};
use super::{McpError, McpServerConfig, McpTransport, McpTransportKind};
use crate::provider::{is_read_timeout, SseEventAssembler, Utf8LineDecoder};

/// How long one `request` waits for its response before giving up — the same
/// budget as the stdio transport.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Cap on error-body text pulled into [`McpError::Http`].
const ERROR_BODY_LIMIT: usize = 512;

/// A Streamable HTTP MCP endpoint. One request in flight at a time, matching
/// the transport contract.
pub struct HttpTransport {
    agent: ureq::Agent,
    url: String,
    headers: Vec<(String, String)>,
    session_id: Option<String>,
    alive: AtomicBool,
    next_id: u64,
    request_timeout: Duration,
}

impl HttpTransport {
    /// Connects to the server configured as `transport: "http"`. No network
    /// traffic happens here — the first request performs the handshake.
    pub fn spawn(config: &McpServerConfig) -> Result<Self, McpError> {
        if config.transport != McpTransportKind::Http {
            return Err(McpError::Protocol(
                "http transport for a non-http config".to_owned(),
            ));
        }
        if config.url.trim().is_empty() {
            return Err(McpError::Transport("empty server url".to_owned()));
        }
        // Short read timeout so a mute socket surfaces as retryable reads the
        // deadline loop can give up on, instead of pinning a blocking call.
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_millis(1500))
            .timeout_read(Duration::from_millis(200))
            .timeout_write(Duration::from_secs(30))
            .build();
        Ok(Self {
            agent,
            url: config.url.clone(),
            headers: config
                .headers
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            session_id: None,
            alive: AtomicBool::new(true),
            next_id: 1,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        })
    }

    /// Shortens the per-request wait (tests only — real tool calls may be slow).
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// The session id the server issued, if any.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// POSTs one JSON envelope and returns the raw response. Status errors
    /// keep their code (the run layer maps 401/403 to permission failures);
    /// transport errors mark the endpoint dead.
    fn post(&mut self, body: &str) -> Result<ureq::Response, McpError> {
        let mut request = self
            .agent
            .post(&self.url)
            .timeout(self.request_timeout)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json, text/event-stream");
        for (key, value) in &self.headers {
            request = request.set(key, value);
        }
        if let Some(session) = &self.session_id {
            request = request.set("Mcp-Session-Id", session);
        }
        request.send_string(body).map_err(|error| match error {
            ureq::Error::Status(status, response) => {
                let body = response.into_string().unwrap_or_default();
                let body = body.chars().take(ERROR_BODY_LIMIT).collect();
                McpError::Http(status, body)
            }
            ureq::Error::Transport(error) => {
                self.alive.store(false, Ordering::SeqCst);
                McpError::Transport(error.to_string())
            }
        })
    }

    fn capture_session(&mut self, response: &ureq::Response) {
        if let Some(session) = response.header("Mcp-Session-Id") {
            self.session_id = Some(session.to_owned());
        }
    }

    /// Reads a non-SSE body and matches the response to `id`.
    fn read_json_response(
        &self,
        reader: &mut dyn std::io::Read,
        id: u64,
        deadline: Instant,
    ) -> Result<serde_json::Value, McpError> {
        let bytes = read_all(reader, deadline)?;
        let body = std::str::from_utf8(&bytes)
            .map_err(|error| McpError::Protocol(format!("non-utf8 response body: {error}")))?;
        match jsonrpc::decode_message(body)? {
            Message::Response(response) if response.id == Some(id) => jsonrpc::into_result(response),
            Message::Response(response) => Err(McpError::Protocol(format!(
                "response id {:?} does not match request id {id}",
                response.id
            ))),
            Message::Notification(_) => Err(McpError::Protocol(
                "expected a response, got a notification".to_owned(),
            )),
        }
    }

    /// Reads an SSE body and returns the first event matching `id`. Other
    /// events — server notifications, unrelated messages — are skipped.
    fn read_sse_response(
        &self,
        reader: &mut dyn std::io::Read,
        id: u64,
        deadline: Instant,
    ) -> Result<serde_json::Value, McpError> {
        let mut decoder = Utf8LineDecoder::new();
        let mut assembler = SseEventAssembler::new();
        let mut buf = [0u8; 4096];
        loop {
            if Instant::now() >= deadline {
                return Err(McpError::Transport("request timed out".to_owned()));
            }
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    for line in decoder.push(&buf[..n]) {
                        if let Some(event) = assembler.push_line(&line) {
                            if let Some(result) = take_sse_message(&event, id) {
                                return result;
                            }
                        }
                    }
                }
                Err(error) if is_read_timeout(&error) => continue,
                Err(error) => return Err(McpError::Transport(error.to_string())),
            }
        }
        // EOF: flush the half-decoded line and the unterminated event.
        if let Some(line) = decoder.finish() {
            if let Some(event) = assembler.push_line(&line) {
                if let Some(result) = take_sse_message(&event, id) {
                    return result;
                }
            }
        }
        if let Some(event) = assembler.flush() {
            if let Some(result) = take_sse_message(&event, id) {
                return result;
            }
        }
        Err(McpError::Protocol(
            "stream ended before the response arrived".to_owned(),
        ))
    }
}

impl McpTransport for HttpTransport {
    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, McpError> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(McpError::Transport("server is gone".to_owned()));
        }
        let id = self.next_id;
        self.next_id += 1;
        let response = self.post(&jsonrpc::encode_request(id, method, params))?;
        self.capture_session(&response);
        let deadline = Instant::now() + self.request_timeout;
        let content_type = response
            .header("content-type")
            .unwrap_or("")
            .to_ascii_lowercase();
        let mut reader = response.into_reader();
        if content_type.starts_with("text/event-stream") {
            self.read_sse_response(&mut reader, id, deadline)
        } else {
            self.read_json_response(&mut reader, id, deadline)
        }
    }

    fn notify(&mut self, method: &str, params: serde_json::Value) -> Result<(), McpError> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(McpError::Transport("server is gone".to_owned()));
        }
        // Streamable HTTP servers acknowledge notifications with 202 and an
        // empty body; any 2xx is success and the body is ignored.
        let response = self.post(&jsonrpc::encode_notification(method, params))?;
        self.capture_session(&response);
        Ok(())
    }

    fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

impl Drop for HttpTransport {
    /// Best-effort session teardown (`DELETE` per Streamable HTTP). A dead or
    /// silent server must not hang or panic on the way out.
    fn drop(&mut self) {
        if let Some(session) = self.session_id.take() {
            let _ = self
                .agent
                .delete(&self.url)
                .timeout(Duration::from_secs(2))
                .set("Mcp-Session-Id", &session)
                .call();
        }
    }
}

/// One SSE event → the answer for `id`, if it is that answer. Notifications
/// and events that fail to decode are skipped noise.
fn take_sse_message(
    event: &str,
    id: u64,
) -> Option<Result<serde_json::Value, McpError>> {
    let message = jsonrpc::decode_message(event).ok()?;
    match message {
        Message::Response(response) if response.id == Some(id) => {
            Some(jsonrpc::into_result(response))
        }
        _ => None,
    }
}

/// Reads to EOF, treating short read timeouts as "keep waiting" until the
/// deadline (the agent's read timeout is deliberately short).
fn read_all(reader: &mut dyn std::io::Read, deadline: Instant) -> Result<Vec<u8>, McpError> {
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        if Instant::now() >= deadline {
            return Err(McpError::Transport("request timed out".to_owned()));
        }
        match reader.read(&mut buf) {
            Ok(0) => return Ok(out),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(error) if is_read_timeout(&error) => continue,
            Err(error) => return Err(McpError::Transport(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    /// One request the canned server saw.
    #[derive(Debug)]
    struct Recorded {
        method: String,
        #[allow(dead_code)]
        path: String,
        headers: Vec<(String, String)>,
        body: String,
    }

    impl Recorded {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        }
    }

    fn config(url: &str) -> McpServerConfig {
        McpServerConfig {
            id: "loopback".to_owned(),
            name: "Loopback".to_owned(),
            enabled: true,
            transport: McpTransportKind::Http,
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            url: url.to_owned(),
            headers: BTreeMap::new(),
        }
    }

    /// The `id` field of a JSON-RPC request body.
    fn request_id(body: &str) -> u64 {
        serde_json::from_str::<serde_json::Value>(body)
            .expect("json body")["id"]
            .as_u64()
            .expect("numeric id")
    }

    /// Spawns a loopback HTTP/1.1 server on `127.0.0.1:0`. The handler sees
    /// every request (Drop's DELETE included) and returns the response; every
    /// request is also recorded for assertions. Keep-alive connections are
    /// served request-by-request until the client hangs up.
    fn serve(
        handler: impl Fn(&Recorded) -> (u16, Vec<(String, String)>, String) + Send + Sync + 'static,
    ) -> (String, Arc<Mutex<Vec<Recorded>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let log = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        {
            let log = Arc::clone(&log);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let handler = Arc::clone(&handler);
                    let log = Arc::clone(&log);
                    std::thread::spawn(move || {
                        while let Some(recorded) = read_request(&mut stream) {
                            let (status, headers, body) = handler(&recorded);
                            // Record before responding: when the client's
                            // call returns, the entry is already in the log.
                            log.lock().expect("log").push(recorded);
                            write_response(&mut stream, status, &headers, &body);
                        }
                    });
                }
            });
        }
        (format!("http://{addr}/mcp"), log)
    }

    fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
        let mut head_bytes = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte) {
                Ok(0) => return None,
                Ok(_) => head_bytes.push(byte[0]),
                Err(_) => return None,
            }
            if head_bytes.ends_with(b"\r\n\r\n") {
                break;
            }
            if head_bytes.len() > 65536 {
                return None;
            }
        }
        let head = String::from_utf8_lossy(&head_bytes).into_owned();
        let mut lines = head.split("\r\n");
        let mut parts = lines.next()?.split_whitespace();
        let method = parts.next()?.to_owned();
        let path = parts.next()?.to_owned();
        let mut headers = Vec::new();
        let mut content_length = 0usize;
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                let name = name.trim().to_owned();
                let value = value.trim().to_owned();
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.parse().unwrap_or(0);
                }
                headers.push((name, value));
            }
        }
        let mut body = Vec::new();
        while body.len() < content_length {
            let mut chunk = [0u8; 4096];
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => body.extend_from_slice(&chunk[..n]),
                Err(_) => return None,
            }
        }
        body.truncate(content_length);
        Some(Recorded {
            method,
            path,
            headers,
            body: String::from_utf8_lossy(&body).into_owned(),
        })
    }

    fn write_response(
        stream: &mut TcpStream,
        status: u16,
        headers: &[(String, String)],
        body: &str,
    ) {
        let reason = match status {
            200 => "OK",
            202 => "Accepted",
            401 => "Unauthorized",
            403 => "Forbidden",
            500 => "Internal Server Error",
            _ => "Status",
        };
        let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str(&format!(
            "Content-Length: {}\r\nConnection: keep-alive\r\n\r\n",
            body.len()
        ));
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body.as_bytes());
        let _ = stream.flush();
    }

    #[test]
    fn json_response_with_session_capture_and_replay() {
        let (url, log) = serve(|recorded| {
            let id = request_id(&recorded.body);
            (
                200,
                vec![
                    ("Content-Type".to_owned(), "application/json".to_owned()),
                    ("Mcp-Session-Id".to_owned(), "sess-1".to_owned()),
                ],
                format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"ok":true}}}}"#),
            )
        });
        let mut transport = HttpTransport::spawn(&config(&url)).expect("spawn");
        assert_eq!(transport.session_id(), None);

        let result = transport
            .request("initialize", serde_json::json!({}))
            .expect("initialize");
        assert_eq!(result["ok"], true);
        assert_eq!(transport.session_id(), Some("sess-1"));

        transport
            .request("tools/list", serde_json::json!({}))
            .expect("tools/list");
        let log = log.lock().expect("log");
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].header("Mcp-Session-Id"), None, "first request: no session yet");
        assert_eq!(
            log[1].header("Mcp-Session-Id"),
            Some("sess-1"),
            "second request replays the captured session"
        );
    }

    #[test]
    fn sse_stream_skips_notifications_and_returns_the_response() {
        let (url, _log) = serve(|recorded| {
            let id = request_id(&recorded.body);
            let body = format!(
                "event: message\ndata: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{{\"level\":\"info\"}}}}\n\n\
                 data: {{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"tools\":[{{\"name\":\"echo\"}}]}}}}\n\n"
            );
            (
                200,
                vec![("Content-Type".to_owned(), "text/event-stream".to_owned())],
                body,
            )
        });
        let mut transport = HttpTransport::spawn(&config(&url)).expect("spawn");
        let result = transport
            .request("tools/list", serde_json::json!({}))
            .expect("list");
        assert_eq!(result["tools"][0]["name"], "echo");
        assert!(transport.alive());
    }

    #[test]
    fn drop_terminates_the_session_with_delete() {
        let (url, log) = serve(|recorded| {
            if recorded.method == "DELETE" {
                return (200, Vec::new(), String::new());
            }
            let id = request_id(&recorded.body);
            (
                200,
                vec![
                    ("Content-Type".to_owned(), "application/json".to_owned()),
                    ("Mcp-Session-Id".to_owned(), "sess-9".to_owned()),
                ],
                format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{}}}}"#),
            )
        });
        let mut transport = HttpTransport::spawn(&config(&url)).expect("spawn");
        transport
            .request("initialize", serde_json::json!({}))
            .expect("initialize");
        drop(transport);

        let log = log.lock().expect("log");
        assert_eq!(log.len(), 2, "initialize + DELETE");
        assert_eq!(log[1].method, "DELETE");
        assert_eq!(log[1].header("Mcp-Session-Id"), Some("sess-9"));
    }

    #[test]
    fn http_error_statuses_carry_their_code() {
        let (url, _log) = serve(|_| {
            (
                401,
                vec![("Content-Type".to_owned(), "text/plain".to_owned())],
                "missing token".to_owned(),
            )
        });
        let mut transport = HttpTransport::spawn(&config(&url)).expect("spawn");
        let error = transport
            .request("initialize", serde_json::json!({}))
            .expect_err("401");
        match error {
            McpError::Http(401, body) => assert!(body.contains("missing token"), "body: {body}"),
            other => panic!("expected http 401, got {other:?}"),
        }
        // A rejected request is not a dead connection.
        assert!(transport.alive());
    }

    #[test]
    fn notify_posts_an_idless_notification_and_accepts_202() {
        let (url, log) = serve(|_| (202, Vec::new(), String::new()));
        let mut transport = HttpTransport::spawn(&config(&url)).expect("spawn");
        transport
            .notify("notifications/initialized", serde_json::json!({}))
            .expect("notify");
        let log = log.lock().expect("log");
        assert_eq!(log.len(), 1);
        assert!(log[0].body.contains("notifications/initialized"));
        assert!(
            !log[0].body.contains("\"id\""),
            "notifications carry no id: {}",
            log[0].body
        );
    }

    #[test]
    fn wrong_config_shapes_are_rejected_without_network() {
        let mut wrong_kind = config("http://127.0.0.1:1/mcp");
        wrong_kind.transport = McpTransportKind::Stdio;
        let error = match HttpTransport::spawn(&wrong_kind) {
            Ok(_) => panic!("a non-http config must not connect"),
            Err(error) => error,
        };
        assert!(matches!(error, McpError::Protocol(_)), "got {error:?}");
        let error = match HttpTransport::spawn(&config("  ")) {
            Ok(_) => panic!("an empty url must not connect"),
            Err(error) => error,
        };
        assert!(matches!(error, McpError::Transport(_)), "got {error:?}");
    }
}
