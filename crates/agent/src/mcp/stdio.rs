//! stdio transport: one persistent child process speaking JSON-RPC over its
//! stdin/stdout. Reuses the process primitives — new process group, scrubbed
//! env, tree kill — but not [`crate::process::ProcessRunner`], which runs a
//! command to completion. This child lives for the whole run.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use super::jsonrpc::{self, Message, Response};
use super::{McpError, McpServerConfig, McpTransport, McpTransportKind};
use crate::process::{apply_env, isolate_process_group, kill_process_tree, EnvPolicy};
use crate::tools::agent_env_policy;

/// How long one `request` waits for its response before giving up. Generous
/// because `tools/call` can legitimately run for minutes; the point is that a
/// mute server cannot hang the run forever.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Cap on retained child stderr (diagnostics only — the full stream is drained
/// so a chatty server cannot fill the pipe and block).
const STDERR_TAIL_LIMIT: usize = 4096;

/// A connected stdio MCP server. One request in flight at a time — the trait
/// takes `&mut self` — so correlation is "the response with my id", with a
/// spillover buffer kept only for robustness against interleaved noise.
pub struct StdioTransport {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: mpsc::Receiver<Message>,
    reader: Option<std::thread::JoinHandle<()>>,
    alive: Arc<AtomicBool>,
    stderr_tail: Arc<Mutex<String>>,
    next_id: u64,
    spillover: Vec<Response>,
    request_timeout: Duration,
}

impl StdioTransport {
    /// Spawns the server configured as `transport: "stdio"`: `command` is the
    /// executable and `args` its argv. The child runs with the agent env
    /// policy (scrubbed parent env) plus the server's own `env` additions.
    pub fn spawn(config: &McpServerConfig) -> Result<Self, McpError> {
        if config.transport != McpTransportKind::Stdio {
            return Err(McpError::Protocol(
                "stdio transport for a non-stdio config".to_owned(),
            ));
        }
        if config.command.trim().is_empty() {
            return Err(McpError::Transport("empty server command".to_owned()));
        }

        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        isolate_process_group(&mut cmd);
        apply_env(&mut cmd, &child_env_policy(&config.env));
        let mut child = cmd
            .spawn()
            .map_err(|error| McpError::Transport(format!("spawn {}: {error}", config.command)))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Transport("no stdout pipe".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| McpError::Transport("no stderr pipe".to_owned()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Transport("no stdin pipe".to_owned()))?;

        let alive = Arc::new(AtomicBool::new(true));
        let stderr_tail = Arc::new(Mutex::new(String::new()));

        // Drain stderr unconditionally so a noisy server cannot block on a
        // full pipe; keep only the tail for diagnostics.
        {
            let stderr_tail = Arc::clone(&stderr_tail);
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let mut tail = stderr_tail.lock().expect("stderr tail lock");
                    if !tail.is_empty() {
                        tail.push('\n');
                    }
                    tail.push_str(&line);
                    let len = tail.len();
                    if len > STDERR_TAIL_LIMIT {
                        tail.drain(..len - STDERR_TAIL_LIMIT);
                    }
                }
            });
        }

        // The stdout reader owns message correlation's half: decode each line
        // and hand it to `request` over the channel. Junk lines are skipped —
        // some wrappers print banners — and EOF marks the transport dead.
        let (tx, rx) = mpsc::channel::<Message>();
        let reader = {
            let alive = Arc::clone(&alive);
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let line = match line {
                        Ok(line) => line,
                        Err(_) => break,
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    match jsonrpc::decode_message(&line) {
                        Ok(message) => {
                            if tx.send(message).is_err() {
                                break;
                            }
                        }
                        // Not JSON-RPC: ignore the line and keep the stream.
                        Err(_) => continue,
                    }
                }
                alive.store(false, Ordering::SeqCst);
            })
        };

        Ok(Self {
            child,
            stdin: Some(stdin),
            rx,
            reader: Some(reader),
            alive,
            stderr_tail,
            next_id: 1,
            spillover: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        })
    }

    /// Shortens the per-request wait (tests only — the default is generous
    /// enough for real tool calls).
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// The child's pid — diagnostics and tests.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The drained tail of the child's stderr, for error context.
    pub fn stderr_tail(&self) -> String {
        self.stderr_tail.lock().expect("stderr tail lock").clone()
    }

    fn send_line(&mut self, line: &str) -> Result<(), McpError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| McpError::Transport("stdin closed".to_owned()))?;
        stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .and_then(|_| stdin.flush())
            .map_err(|error| {
                self.alive.store(false, Ordering::SeqCst);
                McpError::Transport(format!("write failed: {error}"))
            })
    }
}

impl McpTransport for StdioTransport {
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
        self.send_line(&jsonrpc::encode_request(id, method, params))?;

        let deadline = Instant::now() + self.request_timeout;
        let mut spillover = std::mem::take(&mut self.spillover);
        let result = loop {
            if let Some(response) = jsonrpc::take_response(&mut spillover, id) {
                break jsonrpc::into_result(response);
            }
            let now = Instant::now();
            if now >= deadline {
                break Err(McpError::Transport(format!("request timed out: {method}")));
            }
            match self.rx.recv_timeout(deadline - now) {
                Ok(Message::Response(response)) => spillover.push(response),
                // Server-initiated requests and logging all land here; both
                // are non-goals, so they are dropped.
                Ok(Message::Notification(_)) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    break Err(McpError::Transport(format!("request timed out: {method}")));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.alive.store(false, Ordering::SeqCst);
                    break Err(McpError::Transport("server closed the connection".to_owned()));
                }
            }
        };
        self.spillover = spillover;
        result
    }

    fn notify(&mut self, method: &str, params: serde_json::Value) -> Result<(), McpError> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(McpError::Transport("server is gone".to_owned()));
        }
        self.send_line(&jsonrpc::encode_notification(method, params))
    }

    fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

impl Drop for StdioTransport {
    /// Tear down the whole tree: the server and any grandchildren it started.
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        kill_process_tree(&mut self.child);
        // Killing closes the stdout pipe, which ends the reader thread; join
        // it so the channel is fully drained before the transport disappears.
        self.stdin.take();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// The child env for a server: the agent scrubbed allowlist, plus the
/// server's own `env` entries (secret values already merged in at load time).
/// Server entries win on key collision.
fn child_env_policy(extra: &BTreeMap<String, String>) -> EnvPolicy {
    let mut policy = agent_env_policy();
    if let EnvPolicy::Scrubbed { vars } = &mut policy {
        for (key, value) in extra {
            vars.push((key.clone(), value.clone()));
        }
    }
    policy
}

/// A canned JSON-RPC responder over `sh`: one request per line, reply keyed
/// by method, echoing the request id. Notifications get silence. Unknown
/// methods get a Method-not-found error. Shared by the transport tests below
/// and the `run()` integration tests in `lib.rs`.
#[cfg(all(unix, test))]
pub(crate) const CANNED_SH_RESPONDER: &str = r#"while IFS= read -r line; do
  set -- $(printf '%s' "$line" | sed -n 's/^{"jsonrpc":"2.0","id":\([0-9]*\),"method":"\([^"]*\)".*/\1 \2/p')
  id=$1
  method=$2
  case "$method" in
    initialize)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2026-07-28","capabilities":{},"serverInfo":{"name":"canned","version":"0"}}}\n' "$id"
      ;;
    tools/list)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"echo back","inputSchema":{"type":"object"}}]}}\n' "$id"
      ;;
    tools/call)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"pong"}]}}\n' "$id"
      ;;
    *)
      if [ -n "$id" ]; then
        printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32601,"message":"Method not found"}}\n' "$id"
      fi
      ;;
  esac
done"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpSecrets;

    fn config(command: &str, args: &[&str]) -> McpServerConfig {
        McpServerConfig {
            id: "canned".to_owned(),
            name: "Canned".to_owned(),
            enabled: true,
            transport: McpTransportKind::Stdio,
            command: command.to_owned(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: BTreeMap::new(),
            url: String::new(),
            headers: BTreeMap::new(),
        }
    }

    /// The canned JSON-RPC responder (`CANNED_SH_RESPONDER`) answers
    /// initialize / tools/list / echo and doubles as the error-mapping case.
    #[cfg(unix)]
    const CANNED: &str = CANNED_SH_RESPONDER;

    #[cfg(unix)]
    #[test]
    fn handshake_and_tool_call_with_a_canned_sh_server() {
        let mut transport = StdioTransport::spawn(&config("/bin/sh", &["-c", CANNED]))
            .expect("spawn canned server");
        assert!(transport.alive());

        let init = transport
            .request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": super::super::PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "kodo-test", "version": "0"}
                }),
            )
            .expect("initialize");
        assert_eq!(init["serverInfo"]["name"], "canned");

        transport
            .notify("notifications/initialized", serde_json::json!({}))
            .expect("notify");

        let tools = transport
            .request("tools/list", serde_json::json!({}))
            .expect("tools/list");
        assert_eq!(tools["tools"][0]["name"], "echo");

        let call = transport
            .request(
                "tools/call",
                serde_json::json!({"name": "echo", "arguments": {"text": "ping"}}),
            )
            .expect("tools/call");
        assert_eq!(call["content"][0]["text"], "pong");

        // Unknown method → the server's JSON-RPC error surfaces as Rpc.
        let error = transport
            .request("nope", serde_json::json!({}))
            .expect_err("unknown method");
        match error {
            McpError::Rpc(object) => {
                assert_eq!(object.code, -32601);
                assert_eq!(object.message, "Method not found");
            }
            other => panic!("expected an rpc error, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn request_times_out_when_the_server_never_answers() {
        // `cat` echoes stdin back to stdout — the decoder sees our own
        // requests (id-bearing, method-bearing → notifications) and never a
        // response, so `request` waits out its deadline.
        let mut transport = StdioTransport::spawn(&config("/bin/cat", &[]))
            .expect("spawn cat")
            .with_request_timeout(Duration::from_millis(200));
        let error = transport
            .request("initialize", serde_json::json!({}))
            .expect_err("no answer");
        assert!(
            matches!(error, McpError::Transport(ref message) if message.contains("timed out")),
            "expected a timeout, got {error:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_dead_server_is_reported_not_hung() {
        let mut transport = StdioTransport::spawn(&config("/bin/sh", &["-c", "exit 0"]))
            .expect("spawn instant exit");
        let error = transport
            .request("initialize", serde_json::json!({}))
            .expect_err("server exited");
        assert!(matches!(error, McpError::Transport(_)), "got {error:?}");
        assert!(!transport.alive());
    }

    #[cfg(unix)]
    #[test]
    fn drop_kills_the_child_process_tree() {
        let transport = StdioTransport::spawn(&config("/bin/sh", &["-c", "while :; do sleep 1; done"]))
            .expect("spawn sleeper");
        let pid = transport.pid() as i32;
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "child should be alive");
        drop(transport);
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "child should be gone after Drop"
        );
    }

    #[test]
    fn non_stdio_configs_are_rejected() {
        let mut wrong_kind = config("/bin/true", &[]);
        wrong_kind.transport = McpTransportKind::Http;
        let error = match StdioTransport::spawn(&wrong_kind) {
            Ok(_) => panic!("a non-stdio config must not spawn"),
            Err(error) => error,
        };
        assert!(matches!(error, McpError::Protocol(_)), "got {error:?}");
        let error = match StdioTransport::spawn(&config("", &[])) {
            Ok(_) => panic!("an empty command must not spawn"),
            Err(error) => error,
        };
        assert!(matches!(error, McpError::Transport(_)), "got {error:?}");
    }

    #[test]
    fn child_env_keeps_the_allowlist_and_adds_server_vars() {
        let extra = BTreeMap::from([("TOKEN".to_owned(), "t-1".to_owned())]);
        let policy = child_env_policy(&extra);
        let vars = match policy {
            EnvPolicy::Scrubbed { vars } => vars,
            EnvPolicy::Inherit => panic!("expected a scrubbed env"),
        };
        assert!(
            vars.iter().any(|(key, value)| key == "TOKEN" && value == "t-1"),
            "server env must be applied: {vars:?}"
        );
        // The allowlist guarantees a PATH even when the parent had none.
        assert!(vars.iter().any(|(key, _)| key == "PATH"));

        // Round-trip the secrets merge the spawn path expects.
        let mut server = config("/bin/sh", &["-c", "true"]);
        server.env = extra.clone();
        let merged = crate::mcp::merge_secrets(
            &crate::mcp::with_secret_values(&server, ""),
            &McpSecrets {
                env: extra,
                headers: BTreeMap::new(),
            },
        );
        let expected = match agent_env_policy() {
            EnvPolicy::Scrubbed { mut vars } => {
                vars.push(("TOKEN".to_owned(), "t-1".to_owned()));
                EnvPolicy::Scrubbed { vars }
            }
            EnvPolicy::Inherit => panic!("expected a scrubbed env"),
        };
        assert_eq!(child_env_policy(&merged.env), expected);
    }
}
