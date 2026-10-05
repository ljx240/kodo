//! JSON-RPC 2.0 envelopes for the MCP client.
//!
//! One request line in, one message out. The layer knows nothing about
//! transports — it encodes requests/notifications, decodes whatever the
//! server sends back, and correlates responses to request ids even when they
//! arrive out of order or interleaved with notifications.

use serde::{Deserialize, Serialize};

use super::McpError;

/// JSON-RPC version marker. MCP is JSON-RPC 2.0 only.
pub const JSONRPC: &str = "2.0";

/// A client → server request. `id` is a plain counter; servers must echo it.
#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
}

/// A client → server notification (no `id`, no response expected).
#[derive(Debug, Clone, Serialize)]
pub struct Notification {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
}

/// A server → client error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// A server → client response. Exactly one of `result` / `error` is set by a
/// well-behaved server; both absent is treated as [`McpError::Protocol`].
#[derive(Debug, Clone, Deserialize)]
pub struct Response {
    #[allow(dead_code)]
    pub jsonrpc: String,
    /// Absent on parse errors (spec allows null). Correlation then fails.
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<ErrorObject>,
}

/// Any server → client message: a response to one of our requests, or a
/// notification (logging, progress, …) we mostly ignore.
#[derive(Debug, Clone)]
pub enum Message {
    Response(Response),
    Notification(Notification),
}

/// Wire shape for decoding: responses carry `id`/`result`/`error`,
/// notifications carry `method`/`params`.
#[derive(Deserialize)]
struct Wire {
    #[allow(dead_code)]
    jsonrpc: String,
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: Option<serde_json::Value>,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<ErrorObject>,
}

/// Encodes a request as one JSON line (newline left to the transport).
pub fn encode_request(id: u64, method: &str, params: serde_json::Value) -> String {
    serde_json::to_string(&Request {
        jsonrpc: JSONRPC,
        id,
        method: method.to_owned(),
        params,
    })
    .expect("request is always serializable")
}

/// Encodes a notification as one JSON line.
pub fn encode_notification(method: &str, params: serde_json::Value) -> String {
    serde_json::to_string(&Notification {
        jsonrpc: JSONRPC,
        method: method.to_owned(),
        params,
    })
    .expect("notification is always serializable")
}

/// Decodes one server message line.
pub fn decode_message(line: &str) -> Result<Message, McpError> {
    let wire: Wire =
        serde_json::from_str(line.trim()).map_err(|error| McpError::Protocol(error.to_string()))?;
    if wire.jsonrpc != JSONRPC {
        return Err(McpError::Protocol(format!(
            "unexpected jsonrpc version {:?}",
            wire.jsonrpc
        )));
    }
    if let Some(method) = wire.method {
        // Server-initiated requests (sampling, roots/list) land here too —
        // we never answer them (both are non-goals), so they read as
        // notifications and get ignored.
        return Ok(Message::Notification(Notification {
            jsonrpc: JSONRPC,
            method,
            params: wire.params.unwrap_or(serde_json::Value::Null),
        }));
    }
    if wire.result.is_none() && wire.error.is_none() {
        return Err(McpError::Protocol(
            "response has neither result nor error".to_owned(),
        ));
    }
    Ok(Message::Response(Response {
        jsonrpc: wire.jsonrpc,
        id: wire.id,
        result: wire.result,
        error: wire.error,
    }))
}

/// Pulls the response for `id` out of a buffer of decoded responses.
/// Out-of-order arrival is fine — the buffer is searched, not consumed
/// front-to-back — so a slow tools/list cannot swallow a later tools/call.
pub fn take_response(buffer: &mut Vec<Response>, id: u64) -> Option<Response> {
    let index = buffer.iter().position(|response| response.id == Some(id))?;
    Some(buffer.remove(index))
}

/// Turns a matched response into a result value or the server's error.
pub fn into_result(response: Response) -> Result<serde_json::Value, McpError> {
    if let Some(error) = response.error {
        return Err(McpError::Rpc(error));
    }
    response.result.ok_or_else(|| {
        McpError::Protocol("response has neither result nor error".to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_encodes_as_one_jsonrpc_line() {
        let line = encode_request(7, "tools/list", serde_json::json!({ "cursor": null }));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line).expect("json"),
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/list",
                "params": { "cursor": null }
            })
        );
    }

    #[test]
    fn notification_omits_the_id() {
        let line = encode_notification("notifications/initialized", serde_json::json!({}));
        let value: serde_json::Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value.get("id"), None);
        assert_eq!(value["method"], "notifications/initialized");
    }

    #[test]
    fn decode_result_response() {
        let message = decode_message(r#"{"jsonrpc":"2.0","id":3,"result":{"tools":[]}}"#).expect("decode");
        match message {
            Message::Response(response) => {
                assert_eq!(response.id, Some(3));
                assert_eq!(response.result, Some(serde_json::json!({"tools": []})));
            }
            other => panic!("expected a response, got {other:?}"),
        }
    }

    #[test]
    fn decode_error_response_and_map_it() {
        let message = decode_message(
            r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32601,"message":"Method not found"}}"#,
        )
        .expect("decode");
        let Message::Response(response) = message else {
            panic!("expected a response");
        };
        let error = into_result(response).expect_err("should be an error");
        match error {
            McpError::Rpc(object) => {
                assert_eq!(object.code, -32601);
                assert_eq!(object.message, "Method not found");
            }
            other => panic!("expected an rpc error, got {other:?}"),
        }
    }

    #[test]
    fn decode_notification_message() {
        let message =
            decode_message(r#"{"jsonrpc":"2.0","method":"notifications/message","params":{"level":"info"}}"#)
                .expect("decode");
        match message {
            Message::Notification(notification) => {
                assert_eq!(notification.method, "notifications/message");
                assert_eq!(notification.params, serde_json::json!({"level": "info"}));
            }
            other => panic!("expected a notification, got {other:?}"),
        }
    }

    #[test]
    fn decode_rejects_junk() {
        assert!(decode_message("not json").is_err());
        assert!(decode_message(r#"{"jsonrpc":"1.0","id":1,"result":{}}"#).is_err());
        assert!(decode_message(r#"{"jsonrpc":"2.0","id":1}"#).is_err());
    }

    #[test]
    fn take_response_handles_out_of_order_arrival() {
        let mut buffer = vec![
            Response {
                jsonrpc: "2.0".to_owned(),
                id: Some(9),
                result: Some(serde_json::json!("slow")),
                error: None,
            },
            Response {
                jsonrpc: "2.0".to_owned(),
                id: Some(2),
                result: Some(serde_json::json!("fast")),
                error: None,
            },
        ];
        assert_eq!(
            into_result(take_response(&mut buffer, 2).expect("id 2")).expect("ok"),
            serde_json::json!("fast")
        );
        assert_eq!(buffer.len(), 1);
        assert_eq!(
            into_result(take_response(&mut buffer, 9).expect("id 9")).expect("ok"),
            serde_json::json!("slow")
        );
        assert!(take_response(&mut buffer, 2).is_none());
    }
}
