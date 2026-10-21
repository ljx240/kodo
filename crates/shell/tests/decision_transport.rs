//! Transport-level acceptance for the decision component.
//!
//! Everything here runs against a loopback mock service with a fictional
//! credential: no request leaves the machine, and the real user state
//! store is never touched.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use kodo_agent::decision::{
    DecisionAccess, DecisionClient, DecisionInput, DecisionMode, DecisionProvider, DecisionResult,
    DecisionScenario, DecisionSettings, TaskRouteInput,
};
use kodo_shell::decision::{
    DecisionConfig, DecisionConsent, DecisionConsentAnswer, DecisionPreview, DecisionRuntime,
    DecisionService,
};

static TEST_LOCK: Mutex<()> = Mutex::new(());

/// What the mock service answers.
#[derive(Clone)]
enum Reply {
    /// A well formed answer for the task-route question.
    Ok,
    ChangeConfig,
    Cancel(Arc<AtomicBool>),
    /// A well formed OpenRouter answer, with token logprobs.
    OpenRouterOk,
    /// A well formed OpenRouter answer without token logprobs.
    OpenRouterNoLogprobs,
    /// A raw status code and body.
    Raw(u16, String),
    /// A well formed answer, but delayed.
    Delayed(Duration),
}

/// One request the mock service received.
#[derive(Clone, Debug)]
struct Recorded {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Recorded {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// A loopback HTTP service that records what it received.
struct MockService {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<Recorded>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockService {
    fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let received = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let received = Arc::clone(&received);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { break };
                    let received = Arc::clone(&received);
                    let reply = reply.clone();
                    thread::spawn(move || {
                        let _ = serve(stream, reply, received);
                    });
                }
            })
        };
        Self {
            addr,
            received,
            stop,
            handle: Some(handle),
        }
    }

    fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn requests(&self) -> Vec<Recorded> {
        self.received
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Drop for MockService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Unblock the accept loop.
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(
    mut stream: TcpStream,
    reply: Reply,
    received: Arc<Mutex<Vec<Recorded>>>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    // Read until the headers are complete.
    let header_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default().to_owned();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_owned();
        let value = value.trim().to_owned();
        if key.eq_ignore_ascii_case("content-length") {
            content_length = value.parse().unwrap_or(0);
        }
        headers.push((key, value));
    }
    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    received
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(Recorded {
            method,
            path,
            headers,
            body: body.clone(),
        });

    let (status, payload, delay) = match reply {
        Reply::Ok => {
            let body = ok_body("refactor", 0.93, 0.02);
            (200, body, Duration::ZERO)
        }
        Reply::ChangeConfig => {
            kodo_shell::decision::bump_config_generation();
            (200, ok_body("refactor", 0.93, 0.02), Duration::ZERO)
        }
        Reply::Cancel(alive) => {
            alive.store(false, Ordering::SeqCst);
            (200, ok_body("refactor", 0.93, 0.02), Duration::ZERO)
        }
        Reply::OpenRouterOk => (200, openrouter_body(true), Duration::ZERO),
        Reply::OpenRouterNoLogprobs => (200, openrouter_body(false), Duration::ZERO),
        Reply::Raw(status, body) => (status, body, Duration::ZERO),
        Reply::Delayed(delay) => {
            let body = ok_body("refactor", 0.93, 0.02);
            (200, body, delay)
        }
    };
    std::thread::sleep(delay);
    let response = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn ok_body(choice: &str, top: f64, _rest: f64) -> String {
    // Seven options: the winner plus six others that share what is left.
    let others = [
        "bug-fix",
        "feature",
        "test",
        "code-review",
        "docs",
        "unknown",
    ];
    let rest = (1.0 - top) / others.len() as f64;
    let mut probabilities = Vec::new();
    for other in others {
        probabilities.push(format!("\"{other}\":{rest}"));
    }
    probabilities.push(format!("\"{choice}\":{top}"));
    format!(
        "{{\"model\":\"jev-1.13.0\",\"answers\":{{\"task_type\":{{\"type\":\"choice\",\"choice\":\"{choice}\",\"probabilities\":{{{}}},\"confidence\":0.9}}}}}}",
        probabilities.join(",")
    )
}

/// A well formed OpenRouter chat completion. The answer is a single option
/// letter; the alternatives are reported as the token logprobs.
fn openrouter_body(with_logprobs: bool) -> String {
    let options = [
        "bug-fix",
        "feature",
        "test",
        "refactor",
        "code-review",
        "docs",
        "unknown",
    ];
    let rest = (1.0f64 - 0.93) / (options.len() as f64 - 1.0);
    let mut alternatives = Vec::new();
    for (index, option) in options.iter().enumerate() {
        let letter = (b'A' + index as u8) as char;
        let logprob = if *option == "refactor" {
            0.93f64.ln()
        } else {
            rest.ln()
        };
        alternatives.push(format!("{{\"token\":\"{letter}\",\"logprob\":{logprob}}}"));
    }
    let logprobs = if with_logprobs {
        format!(
            "{{\"content\":[{{\"token\":\"D\",\"logprob\":{},\"top_logprobs\":[{}]}}]}}",
            0.93f64.ln(),
            alternatives.join(",")
        )
    } else {
        "null".to_owned()
    };
    format!(
        "{{\"model\":\"openai/gpt-4o-mini\",\"choices\":[{{\"message\":{{\"role\":\"assistant\",\"content\":\"D\"}},\"logprobs\":{logprobs}}}],\"usage\":{{\"prompt_tokens\":180,\"completion_tokens\":1}}}}",
    )
}

/// A consent host that always allows.
struct AllowAll;

impl DecisionConsent for AllowAll {
    fn request(&self, _preview: &DecisionPreview) -> DecisionConsentAnswer {
        DecisionConsentAnswer::Allow
    }
}

/// A consent host that always refuses.
struct DenyAll;

impl DecisionConsent for DenyAll {
    fn request(&self, _preview: &DecisionPreview) -> DecisionConsentAnswer {
        DecisionConsentAnswer::Skip
    }
}

fn config(mode: DecisionMode, timeout_ms: u64) -> DecisionConfig {
    DecisionConfig {
        settings: DecisionSettings {
            mode,
            model: "jev-1.13.0".to_owned(),
            task_route_enabled: true,
            failure_triage_enabled: true,
            request_timeout_ms: timeout_ms,
            ..DecisionSettings::default()
        },
        credential: Some("synthetic-credential".to_owned()),
        config_generation: kodo_shell::decision::current_config_generation(),
        config_error: None,
        recovered: false,
    }
}

/// The configuration for the OpenRouter path.
fn openrouter_config(mode: DecisionMode, timeout_ms: u64) -> DecisionConfig {
    DecisionConfig {
        settings: DecisionSettings {
            provider: DecisionProvider::OpenRouter,
            mode,
            model: "openai/gpt-4o-mini".to_owned(),
            task_route_enabled: true,
            failure_triage_enabled: true,
            request_timeout_ms: timeout_ms,
            ..DecisionSettings::default()
        },
        credential: Some("synthetic-openrouter-key".to_owned()),
        config_generation: kodo_shell::decision::current_config_generation(),
        config_error: None,
        recovered: false,
    }
}

fn service(
    origin: &str,
    mode: DecisionMode,
    timeout_ms: u64,
    consent: Arc<dyn DecisionConsent>,
) -> Arc<DecisionService> {
    Arc::new(
        DecisionService::new(
            config(mode, timeout_ms),
            DecisionClient::for_local_endpoint(DecisionProvider::TypeSafe, origin),
            consent,
            "session-1",
            1,
        )
        .with_runtime(Arc::new(DecisionRuntime::default())),
    )
}

/// The service on the OpenRouter path, pointed at the loopback mock.
fn openrouter_service(
    origin: &str,
    mode: DecisionMode,
    timeout_ms: u64,
    consent: Arc<dyn DecisionConsent>,
) -> Arc<DecisionService> {
    Arc::new(
        DecisionService::new(
            openrouter_config(mode, timeout_ms),
            DecisionClient::for_local_endpoint(DecisionProvider::OpenRouter, origin),
            consent,
            "session-1",
            1,
        )
        .with_runtime(Arc::new(DecisionRuntime::default())),
    )
}

/// The configuration for the Tencent EdgeOne path.
fn tencent_config(mode: DecisionMode, timeout_ms: u64) -> DecisionConfig {
    DecisionConfig {
        settings: DecisionSettings {
            provider: DecisionProvider::Tencent,
            mode,
            model: "@makers/jev".to_owned(),
            task_route_enabled: true,
            failure_triage_enabled: true,
            request_timeout_ms: timeout_ms,
            ..DecisionSettings::default()
        },
        credential: Some("synthetic-tencent-key".to_owned()),
        config_generation: kodo_shell::decision::current_config_generation(),
        config_error: None,
        recovered: false,
    }
}

/// The service on the Tencent path, pointed at the loopback mock.
fn tencent_service(
    origin: &str,
    mode: DecisionMode,
    timeout_ms: u64,
    consent: Arc<dyn DecisionConsent>,
) -> Arc<DecisionService> {
    Arc::new(
        DecisionService::new(
            tencent_config(mode, timeout_ms),
            DecisionClient::for_local_endpoint(DecisionProvider::Tencent, origin),
            consent,
            "session-1",
            1,
        )
        .with_runtime(Arc::new(DecisionRuntime::default())),
    )
}

fn task_route_input() -> DecisionInput {
    DecisionInput::TaskRoute(TaskRouteInput {
        message: "把重复的校验逻辑合并一下".to_owned(),
        mode: "code".to_owned(),
        baseline: kodo_agent::TaskType::Feature,
    })
}

#[test]
fn off_makes_no_request_at_all() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::Ok);
    let service = service(&mock.origin(), DecisionMode::Off, 1500, Arc::new(AllowAll));
    let outcome = service.evaluate(&task_route_input(), &|| true);
    assert!(matches!(outcome.result, DecisionResult::Skipped(_)));
    assert_eq!(mock.requests().len(), 0, "off must not touch the network");
}

#[test]
fn a_refused_request_makes_no_request_at_all() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::Ok);
    let service = service(
        &mock.origin(),
        DecisionMode::Observe,
        1500,
        Arc::new(DenyAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    assert!(matches!(outcome.result, DecisionResult::Skipped(_)));
    assert_eq!(
        mock.requests().len(),
        0,
        "a refusal must not touch the network"
    );
}

#[test]
fn an_allowed_request_reaches_the_destination_with_the_frozen_bytes() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::Ok);
    let service = service(
        &mock.origin(),
        DecisionMode::Observe,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    let decision = match outcome.result {
        DecisionResult::Suggested(decision) => decision,
        other => panic!("expected a suggestion, got {other:?}"),
    };
    assert_eq!(decision.choice, "refactor");
    assert_eq!(decision.scenario, DecisionScenario::TaskRoute);
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/systemone");
    assert_eq!(
        request.header("authorization"),
        Some("Bearer synthetic-credential"),
        "the credential must travel in the authorization header"
    );
    let body = String::from_utf8(request.body.clone()).expect("utf-8 body");
    assert!(body.contains("把重复的校验逻辑合并一下"), "{body}");
    assert!(body.contains("\"model\":\"jev-1.13.0\""), "{body}");
    assert!(body.contains("\"task_type\""), "{body}");
    // The local baseline must not travel: it would anchor the answer.
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("json body");
    let state = parsed["state"].as_object().expect("state object");
    let mut keys: Vec<&String> = state.keys().collect();
    keys.sort();
    assert_eq!(keys, vec!["message", "mode"], "{body}");
}

#[test]
fn the_openrouter_path_sends_its_own_shape_to_its_own_path() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::OpenRouterOk);
    let service = openrouter_service(
        &mock.origin(),
        DecisionMode::Observe,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    let decision = match outcome.result {
        DecisionResult::Suggested(decision) => decision,
        other => panic!("expected a suggestion, got {other:?}"),
    };
    assert_eq!(decision.choice, "refactor");
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.path, "/api/v1/chat/completions");
    assert_eq!(
        request.header("authorization"),
        Some("Bearer synthetic-openrouter-key"),
        "the OpenRouter key must be used for the OpenRouter request"
    );
    let body = String::from_utf8(request.body.clone()).expect("utf-8 body");
    let value: serde_json::Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(value["model"], "openai/gpt-4o-mini");
    assert_eq!(value["logprobs"], true);
    assert_eq!(value["temperature"], 0.0);
    let system = value["messages"][0]["content"]
        .as_str()
        .expect("system message");
    assert!(system.contains("A. bug-fix"), "{system}");
    let user = value["messages"][1]["content"]
        .as_str()
        .expect("user message");
    assert!(user.contains("把重复的校验逻辑合并一下"), "{user}");
}

#[test]
fn an_openrouter_answer_without_logprobs_is_never_adoptable() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::OpenRouterNoLogprobs);
    let service = openrouter_service(
        &mock.origin(),
        DecisionMode::Assist,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    match outcome.result {
        DecisionResult::Uncalibrated { choice } => assert_eq!(choice, "refactor"),
        other => panic!("expected an uncalibrated answer, got {other:?}"),
    }
}

#[test]
fn the_tencent_path_sends_the_native_shape_to_its_own_path() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::Ok);
    let service = tencent_service(
        &mock.origin(),
        DecisionMode::Observe,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    let decision = match outcome.result {
        DecisionResult::Suggested(decision) => decision,
        other => panic!("expected a suggestion, got {other:?}"),
    };
    assert_eq!(decision.choice, "refactor");
    // The alias resolved to a build; the record names the build.
    assert_eq!(decision.model, "jev-1.13.0");
    assert_eq!(outcome.receipt.model, "jev-1.13.0");
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.path, "/v1/systemone");
    assert_eq!(
        request.header("authorization"),
        Some("Bearer synthetic-tencent-key"),
        "the Tencent key must be used for the Tencent request"
    );
    let body = String::from_utf8(request.body.clone()).expect("utf-8 body");
    let value: serde_json::Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(value["model"], "@makers/jev");
    assert!(value["questions"]["task_type"].is_object(), "{value}");
    // The native shape, not a chat completion.
    assert!(value.get("messages").is_none(), "{value}");
    let state = value["state"].as_object().expect("state object");
    let mut keys: Vec<&String> = state.keys().collect();
    keys.sort();
    assert_eq!(keys, vec!["message", "mode"], "{value}");
}

#[test]
fn a_tencent_answer_for_another_model_is_refused() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let body = "{\"model\":\"jev-1.13.0-experimental\",\"answers\":{\"task_type\":{\"type\":\"choice\",\"choice\":\"refactor\",\"probabilities\":{\"bug-fix\":0.02,\"feature\":0.02,\"test\":0.02,\"refactor\":0.93,\"code-review\":0.0,\"docs\":0.0,\"unknown\":0.01},\"confidence\":0.9}}}";
    let mock = MockService::start(Reply::Raw(200, body.to_owned()));
    let service = tencent_service(
        &mock.origin(),
        DecisionMode::Observe,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    match outcome.result {
        DecisionResult::Fallback(failure) => {
            assert_eq!(failure.code, "invalid_response");
            assert!(failure.detail.contains("model mismatch"), "{failure:?}");
        }
        other => panic!("expected a fallback, got {other:?}"),
    }
    assert_eq!(mock.requests().len(), 1, "there must be no automatic retry");
}

#[test]
fn an_authentication_failure_is_reported_and_not_retried() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::Raw(401, "{\"error\":\"unauthorized\"}".to_owned()));
    let service = service(
        &mock.origin(),
        DecisionMode::Observe,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    match outcome.result {
        DecisionResult::Fallback(failure) => assert_eq!(failure.code, "authentication"),
        other => panic!("expected a fallback, got {other:?}"),
    }
    assert_eq!(mock.requests().len(), 1, "there must be no automatic retry");
}

#[test]
fn a_rate_limit_is_reported_and_not_retried() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::Raw(429, "{}".to_owned()));
    let service = service(
        &mock.origin(),
        DecisionMode::Observe,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    match outcome.result {
        DecisionResult::Fallback(failure) => assert_eq!(failure.code, "rate_limited"),
        other => panic!("expected a fallback, got {other:?}"),
    }
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn a_server_error_is_reported_and_not_retried() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::Raw(503, "{}".to_owned()));
    let service = service(
        &mock.origin(),
        DecisionMode::Observe,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    match outcome.result {
        DecisionResult::Fallback(failure) => assert_eq!(failure.code, "unavailable"),
        other => panic!("expected a fallback, got {other:?}"),
    }
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn a_redirect_is_refused() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::Raw(302, "{}".to_owned()));
    let service = service(
        &mock.origin(),
        DecisionMode::Observe,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    match outcome.result {
        DecisionResult::Fallback(failure) => assert_eq!(failure.code, "redirect"),
        other => panic!("expected a fallback, got {other:?}"),
    }
    // The redirect must not be followed: exactly one request was made.
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn an_invalid_body_is_refused() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::Raw(200, "not json".to_owned()));
    let service = service(
        &mock.origin(),
        DecisionMode::Observe,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    match outcome.result {
        DecisionResult::Fallback(failure) => assert_eq!(failure.code, "invalid_response"),
        other => panic!("expected a fallback, got {other:?}"),
    }
}

#[test]
fn an_oversized_response_is_refused() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let body = "x".repeat(200_000);
    let mock = MockService::start(Reply::Raw(200, body));
    let service = service(
        &mock.origin(),
        DecisionMode::Observe,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    match outcome.result {
        DecisionResult::Fallback(failure) => assert_eq!(failure.code, "invalid_response"),
        other => panic!("expected a fallback, got {other:?}"),
    }
}

#[test]
fn a_slow_answer_times_out_without_a_retry() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::Delayed(Duration::from_millis(600)));
    let service = service(
        &mock.origin(),
        DecisionMode::Observe,
        150,
        Arc::new(AllowAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    match outcome.result {
        DecisionResult::Fallback(failure) => {
            assert!(
                failure.code == "timeout" || failure.code == "network",
                "unexpected code {}",
                failure.code
            );
        }
        other => panic!("expected a fallback, got {other:?}"),
    }
    assert_eq!(mock.requests().len(), 1, "there must be no automatic retry");
}

#[test]
fn a_model_mismatch_is_refused() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let body = "{\"model\":\"jev-1.14.0\",\"answers\":{\"task_type\":{\"type\":\"choice\",\"choice\":\"refactor\",\"probabilities\":{\"bug-fix\":0.02,\"feature\":0.02,\"test\":0.02,\"refactor\":0.93,\"code-review\":0.0,\"docs\":0.0,\"unknown\":0.01},\"confidence\":0.9}}}}";
    let mock = MockService::start(Reply::Raw(200, body.to_owned()));
    let service = service(
        &mock.origin(),
        DecisionMode::Observe,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = service.evaluate(&task_route_input(), &|| true);
    match outcome.result {
        DecisionResult::Fallback(failure) => assert_eq!(failure.code, "invalid_response"),
        other => panic!("expected a fallback, got {other:?}"),
    }
}

#[test]
fn stale_configuration_never_reaches_the_server() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::Ok);
    let svc = service(
        &mock.origin(),
        DecisionMode::Assist,
        1500,
        Arc::new(AllowAll),
    );
    kodo_shell::decision::bump_config_generation();
    let outcome = svc.evaluate(&task_route_input(), &|| true);
    assert!(matches!(outcome.result, DecisionResult::Fallback(f) if f.code == "config_changed"));
    assert!(mock.requests().is_empty());
}

#[test]
fn configuration_changed_during_http_discards_the_answer() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::ChangeConfig);
    let svc = service(
        &mock.origin(),
        DecisionMode::Assist,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = svc.evaluate(&task_route_input(), &|| true);
    assert!(matches!(outcome.result, DecisionResult::Fallback(f) if f.code == "config_changed"));
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn cancellation_during_http_discards_the_answer() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let alive = Arc::new(AtomicBool::new(true));
    let mock = MockService::start(Reply::Cancel(alive.clone()));
    let svc = service(
        &mock.origin(),
        DecisionMode::Assist,
        1500,
        Arc::new(AllowAll),
    );
    let outcome = svc.evaluate(&task_route_input(), &|| alive.load(Ordering::SeqCst));
    assert!(matches!(outcome.result, DecisionResult::Cancelled));
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn saved_assist_applies_a_high_probability_response() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::Ok);
    let svc = service(
        &mock.origin(),
        DecisionMode::Assist,
        1500,
        Arc::new(AllowAll),
    );
    let mut diagnostics = Vec::new();
    let input = match task_route_input() {
        DecisionInput::TaskRoute(input) => input,
        _ => unreachable!(),
    };
    let outcome = kodo_agent::decision::consult_task_route(
        &svc.handle(),
        &input,
        kodo_agent::TaskType::Feature,
        &|| true,
        &mut |event| {
            if let kodo_agent::SinkEvent::Finished {
                step:
                    kodo_agent::Step::Reasoning {
                        diagnostics: Some(record),
                        ..
                    },
                ..
            } = event
            {
                diagnostics.push(record);
            }
            true
        },
    );
    assert!(matches!(
        outcome,
        kodo_agent::decision::TaskRouteOutcome::Applied(kodo_agent::TaskType::Refactor)
    ));
    assert_eq!(mock.requests().len(), 1);
    assert!(diagnostics.iter().any(
        |record| record.contains("\"mode\":\"assist\"") && record.contains("\"applied\":true")
    ));
}

#[test]
fn three_runs_share_the_same_two_network_slots() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mock = MockService::start(Reply::Delayed(Duration::from_millis(300)));
    let runtime = Arc::new(DecisionRuntime::default());
    let make = |id| {
        Arc::new(
            DecisionService::new(
                config(DecisionMode::Observe, 1500),
                DecisionClient::for_local_endpoint(DecisionProvider::TypeSafe, mock.origin()),
                Arc::new(AllowAll),
                format!("session-{id}"),
                1,
            )
            .with_runtime(runtime.clone()),
        )
    };
    let one = make(1);
    let two = make(2);
    let third = make(3);
    let first = thread::spawn(move || one.evaluate(&task_route_input(), &|| true));
    let second = thread::spawn(move || two.evaluate(&task_route_input(), &|| true));
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while mock.requests().len() < 2 && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(mock.requests().len(), 2);
    let outcome = third.evaluate(&task_route_input(), &|| true);
    assert!(matches!(outcome.result, DecisionResult::Fallback(f) if f.code == "busy"));
    assert!(matches!(
        first.join().unwrap().result,
        DecisionResult::Suggested(_)
    ));
    assert!(matches!(
        second.join().unwrap().result,
        DecisionResult::Suggested(_)
    ));
    assert_eq!(mock.requests().len(), 2);
}
