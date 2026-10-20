//! Run-level decision assistance (Jev): consent, quotas and the transport.
//!
//! The agent asks *whether* it may consult the decision service; this
//! module decides *how*, and owns everything that must never live in the
//! agent: the credential, the outbound consent, the per-turn quotas, the
//! circuit breaker and the frozen request bytes.
//!
//! Two rules shape the whole module:
//!
//! 1. Nothing leaves the machine without a one-time, explicitly granted
//!    consent that is bound to the exact bytes that will be sent. A grant
//!    is consumed atomically and cannot be replayed.
//! 2. Every failure — a refusal, an expired grant, a timeout, a rate
//!    limit — ends in "carry on with the local rule". The decision service
//!    is never allowed to become a second source of authority.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kodo_agent::decision::{
    build_provider_request, looks_sensitive, parse_openrouter_response, plan_failure_triage,
    plan_task_route, validate_response, DecisionAccess, DecisionClient, DecisionFailure,
    DecisionInput, DecisionMode, DecisionOutcome, DecisionProvider, DecisionReceipt,
    DecisionResult, DecisionScenario, DecisionSettings, OpenRouterAnswer, TaskRouteInput,
    CREDENTIAL_NAMESPACE, SETTINGS_KEY,
};
use kodo_core::settings;

/// How long a consent grant stays usable.
pub const CONSENT_TTL: Duration = Duration::from_secs(300);
/// How many decision requests one turn may make.
const DEFAULT_MAX_CALLS_PER_TURN: u32 = 3;
/// How many decision requests may be in flight at the same time.
const MAX_INFLIGHT: u32 = 2;
/// How many consecutive transport failures open the circuit.
const BREAKER_THRESHOLD: u32 = 3;
/// How long the circuit stays open.
const BREAKER_COOLDOWN: Duration = Duration::from_secs(60);

/// The process-wide configuration generation.
///
/// Every outstanding grant is bound to the generation that was current when
/// it was minted. Saving a setting, replacing the credential or turning the
/// feature off bumps the generation, which makes every older grant and
/// every late answer inert.
static CONFIG_GENERATION: AtomicU64 = AtomicU64::new(1);

/// The current configuration generation.
pub fn current_config_generation() -> u64 {
    CONFIG_GENERATION.load(Ordering::SeqCst)
}

/// Invalidate every outstanding grant and every in-flight answer.
pub fn bump_config_generation() -> u64 {
    CONFIG_GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

/// The decision configuration plus the credential, as read from the local
/// stores. The credential never leaves the Rust side.
#[derive(Clone)]
pub struct DecisionConfig {
    pub settings: DecisionSettings,
    /// The plaintext credential. `None` when nothing is stored.
    pub credential: Option<String>,
    pub config_generation: u64,
    /// Set when the stored configuration cannot be used. The feature stays
    /// off in that case — an invalid record never downgrades to "on".
    pub config_error: Option<String>,
}

impl DecisionConfig {
    /// True when a request could be made at all: the configuration is
    /// valid, the feature is on and a credential is stored.
    pub fn usable(&self) -> bool {
        self.config_error.is_none()
            && self.settings.mode != DecisionMode::Off
            && self.credential.is_some()
    }
}

/// The last configuration this process saw, so a content change can be
/// turned into a generation bump.
static LAST_CONFIG: Mutex<Option<(String, DecisionProvider, bool)>> = Mutex::new(None);

/// Read the decision configuration from the local stores.
///
/// Never fails: a missing record, an unparsable record and a missing
/// `HOME` all degrade to "off" plus a configuration problem.
pub fn load_decision_config() -> DecisionConfig {
    let settings_path = settings::settings_path();
    let creds = settings::credentials_path();
    load_decision_config_from(settings_path.as_deref(), creds.as_deref())
}

/// Read the decision configuration from an explicit pair of files.
///
/// The stores are passed in so the save→read round trip can be tested
/// against a real file: a record that is written but cannot be read back
/// is worse than no record at all.
pub fn load_decision_config_from(
    settings_path: Option<&std::path::Path>,
    creds: Option<&std::path::Path>,
) -> DecisionConfig {
    let raw = settings_path
        .and_then(|path| settings::read(path, SETTINGS_KEY))
        .filter(|value| !value.trim().is_empty());

    let (parsed, config_error) = match raw.as_deref() {
        None => (DecisionSettings::default(), None),
        Some(value) => match DecisionSettings::from_json(value) {
            Ok(parsed) => (parsed, None),
            Err(error) => (DecisionSettings::default(), Some(error)),
        },
    };

    // The credential belongs to the configured provider. One provider's key
    // never stands in for another's: a request must never be signed with a
    // key the user gave for a different service.
    let credential = creds
        .and_then(|path| {
            settings::read_secret(path, CREDENTIAL_NAMESPACE, parsed.provider.credential_id())
        })
        .filter(|value| !value.trim().is_empty());

    // A content change invalidates every outstanding grant.
    let fingerprint = (
        raw.clone().unwrap_or_default(),
        parsed.provider,
        credential.is_some(),
    );
    let changed = {
        let mut last = LAST_CONFIG
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let changed = last.as_ref() != Some(&fingerprint);
        *last = Some(fingerprint);
        changed
    };
    let config_generation = if changed {
        bump_config_generation()
    } else {
        current_config_generation()
    };

    DecisionConfig {
        settings: parsed,
        credential,
        config_generation,
        config_error,
    }
}

/// What the settings screen may show. The credential is reduced to a mask
/// and a boolean; the plaintext never leaves the Rust side.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionSettingsView {
    /// The configured provider: `typeSafe` or `openRouter`.
    pub provider: String,
    pub mode: String,
    pub model: String,
    pub task_route_enabled: bool,
    pub failure_triage_enabled: bool,
    /// Whether a credential is stored for the configured provider. The
    /// plaintext never leaves Rust.
    pub has_credential: bool,
    /// The last few characters of the stored credential, or an empty string.
    pub credential_mask: String,
    /// The credential state of every provider, so the form can show which
    /// key belongs where.
    pub credentials: Vec<DecisionCredentialView>,
    /// Set when the stored configuration cannot be used.
    pub config_error: Option<String>,
    pub config_generation: u64,
    /// The default configuration, as JSON. The settings form starts from
    /// this and overrides the fields the user changed, so the numeric
    /// limits live in exactly one place.
    pub defaults_json: String,
}

/// The credential state of one provider.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionCredentialView {
    pub provider: String,
    pub id: String,
    pub has_credential: bool,
    pub credential_mask: String,
}

/// The settings screen's view of the decision configuration.
pub fn decision_settings_view() -> DecisionSettingsView {
    let config = load_decision_config();
    let credentials = DecisionProvider::ALL
        .iter()
        .map(|provider| {
            let stored = stored_credential(*provider);
            DecisionCredentialView {
                provider: provider.label().to_owned(),
                id: provider.credential_id().to_owned(),
                has_credential: stored.is_some(),
                credential_mask: stored.as_deref().map(mask_secret).unwrap_or_default(),
            }
        })
        .collect();
    DecisionSettingsView {
        provider: config.settings.provider.label().to_owned(),
        mode: config.settings.mode.label().to_owned(),
        model: config.settings.model.clone(),
        task_route_enabled: config.settings.task_route_enabled,
        failure_triage_enabled: config.settings.failure_triage_enabled,
        has_credential: config.credential.is_some(),
        credential_mask: config
            .credential
            .as_deref()
            .map(mask_secret)
            .unwrap_or_default(),
        credentials,
        config_error: config.config_error.clone(),
        config_generation: config.config_generation,
        defaults_json: DecisionSettings::default().to_json(),
    }
}

/// The stored credential of one provider, if there is one.
fn stored_credential(provider: DecisionProvider) -> Option<String> {
    settings::credentials_path()
        .as_ref()
        .and_then(|path| {
            settings::read_secret(path, CREDENTIAL_NAMESPACE, provider.credential_id())
        })
        .filter(|value| !value.trim().is_empty())
}

/// The last four characters of a secret, everything else masked.
pub fn mask_secret(secret: &str) -> String {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let tail: String = trimmed
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    format!("••••{tail}")
}

/// Persist the decision settings. The record is validated first: an
/// invalid record is refused, never stored.
pub fn save_decision_settings(raw: &str) -> Result<u64, String> {
    let path = settings::settings_path().ok_or("settings directory is unavailable")?;
    save_decision_settings_to(&path, raw)
}

/// Store the decision settings in an explicit file.
///
/// The record is normalized before it is written: the log is line-based,
/// so the stored value has to be a single line. A pretty-printed value
/// would be cut at the first newline and read back as a truncated object.
pub fn save_decision_settings_to(path: &std::path::Path, raw: &str) -> Result<u64, String> {
    let parsed = DecisionSettings::from_json(raw)?;
    let normalized = parsed.to_json();
    if normalized.contains('\n') || normalized.contains('\r') {
        return Err("jev-decision: the stored record must be a single line".to_owned());
    }
    settings::append(path, SETTINGS_KEY, &normalized).map_err(|error| error.to_string())?;
    Ok(bump_config_generation())
}

/// Store the decision credential. An empty secret is refused: clearing is
/// an explicit, separate action.
pub fn save_decision_credential(provider: DecisionProvider, secret: &str) -> Result<u64, String> {
    let secret = secret.trim();
    if secret.is_empty() {
        return Err("the credential must not be empty".to_owned());
    }
    let path = settings::credentials_path().ok_or("credentials directory is unavailable")?;
    settings::write_secret(
        &path,
        CREDENTIAL_NAMESPACE,
        provider.credential_id(),
        secret,
    )
    .map_err(|error| error.to_string())?;
    Ok(bump_config_generation())
}

/// Remove one provider's decision credential. The other provider's key and
/// every other credential family are untouched.
pub fn clear_decision_credential(provider: DecisionProvider) -> Result<u64, String> {
    let path = settings::credentials_path().ok_or("credentials directory is unavailable")?;
    // `clear_secrets` rebuilds a whole namespace, so the other ids are read
    // back in and kept.
    let mut keep: Vec<(String, String)> = Vec::new();
    for other in DecisionProvider::ALL {
        if other == provider {
            continue;
        }
        if let Some(value) = stored_credential(other) {
            keep.push((other.credential_id().to_owned(), value));
        }
    }
    settings::clear_secrets(&path, CREDENTIAL_NAMESPACE, &keep)
        .map_err(|error| error.to_string())?;
    Ok(bump_config_generation())
}

/// What the user decided about one outbound request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionConsentAnswer {
    /// Send this request, once.
    Allow,
    /// Do not send it. The scenario is not asked again in this run.
    Skip,
    /// The user stopped the run.
    Cancelled,
}

/// The consent request shown to the user.
///
/// It carries the *actual* bytes that will be sent, not a summary: a
/// summary cannot be approved, only inspected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionPreview {
    pub request_id: String,
    pub scenario: DecisionScenario,
    pub mode: DecisionMode,
    pub origin: String,
    pub path: String,
    pub model: String,
    pub policy_version: String,
    /// A short description of what the request is for.
    pub purpose: String,
    /// The exact JSON body that will be sent.
    pub request_body: String,
    /// The questions and their answer space.
    pub questions: String,
    pub request_bytes: usize,
    pub state_bytes: usize,
    pub expires_in_seconds: u64,
}

/// The host's consent channel. The desktop and the CLI both implement it;
/// the shell only asks.
pub trait DecisionConsent: Send + Sync {
    fn request(&self, preview: &DecisionPreview) -> DecisionConsentAnswer;
}

/// A consent that has not been consumed yet.
#[derive(Debug, Clone)]
struct PendingDecision {
    request_id: String,
    session_id: String,
    run_generation: u64,
    config_generation: u64,
    scenario: DecisionScenario,
    origin: String,
    path: String,
    model: String,
    policy_version: String,
    /// The exact bytes that will be sent. Frozen at preview time.
    payload: Vec<u8>,
    /// The exact state the payload was built from. Frozen at preview time.
    source_snapshot: Vec<u8>,
    created: Instant,
}

/// The values a grant is checked against when it is consumed.
///
/// Every field has to match. A mismatch means the world moved while the
/// user was reading the preview, and the grant is dead.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Binding {
    session_id: String,
    run_generation: u64,
    config_generation: u64,
    scenario: DecisionScenario,
    model: String,
    policy_version: String,
    origin: String,
    path: String,
    payload: Vec<u8>,
    source_snapshot: Vec<u8>,
}

/// The circuit breaker's state.
#[derive(Debug, Default)]
struct Breaker {
    consecutive_failures: u32,
    open_until: Option<Instant>,
    probe_in_flight: bool,
    /// Set when the credential was rejected. It stays blocked until a new
    /// credential is loaded into a new service.
    auth_blocked: bool,
}

/// The run-level decision service.
///
/// One instance per run. It holds the credential, the pending grants, the
/// per-turn quotas and the circuit breaker. Everything it knows dies with
/// the run.
pub struct DecisionService {
    config: DecisionConfig,
    client: DecisionClient,
    consent: Arc<dyn DecisionConsent>,
    session_id: String,
    run_generation: u64,
    pending: Mutex<HashMap<String, PendingDecision>>,
    /// The snapshots that were already asked about in this run.
    asked: Mutex<HashSet<String>>,
    /// The scenarios the user refused in this run.
    denied: Mutex<HashSet<DecisionScenario>>,
    calls_used: AtomicU64,
    inflight: AtomicU64,
    /// Milliseconds spent waiting for the service in this run.
    transport_wait_ms: AtomicU64,
    breaker: Mutex<Breaker>,
    request_seq: AtomicU64,
}

impl DecisionService {
    /// A service for one run.
    pub fn new(
        config: DecisionConfig,
        client: DecisionClient,
        consent: Arc<dyn DecisionConsent>,
        session_id: impl Into<String>,
        run_generation: u64,
    ) -> Self {
        Self {
            config,
            client,
            consent,
            session_id: session_id.into(),
            run_generation,
            pending: Mutex::new(HashMap::new()),
            asked: Mutex::new(HashSet::new()),
            denied: Mutex::new(HashSet::new()),
            calls_used: AtomicU64::new(0),
            inflight: AtomicU64::new(0),
            transport_wait_ms: AtomicU64::new(0),
            breaker: Mutex::new(Breaker::default()),
            request_seq: AtomicU64::new(0),
        }
    }

    /// The service pointed at the configured provider's pinned destination.
    ///
    /// The destination follows the configuration, and the configuration
    /// names exactly one provider. A failure never falls back to the other
    /// one: sending the same content to a second receiver the user did
    /// not approve for it is not a fallback.
    pub fn with_official_destination(
        config: DecisionConfig,
        consent: Arc<dyn DecisionConsent>,
        session_id: impl Into<String>,
        run_generation: u64,
    ) -> Self {
        let client = DecisionClient::for_provider(config.settings.provider);
        Self::new(config, client, consent, session_id, run_generation)
    }

    /// The handle the agent takes.
    pub fn handle(self: &Arc<Self>) -> kodo_agent::DecisionHandle {
        Arc::clone(self) as kodo_agent::DecisionHandle
    }

    /// The pending grants, for a host that wants to show them.
    pub fn pending_count(&self) -> usize {
        Self::lock(&self.pending).len()
    }

    fn lock<'a, T>(mutex: &'a Mutex<T>) -> MutexGuard<'a, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn next_request_id(&self) -> String {
        let seq = self.request_seq.fetch_add(1, Ordering::SeqCst);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        format!("jev-{}-{nanos}-{seq}", std::process::id())
    }

    /// The binding a grant minted right now would be checked against.
    fn current_binding(
        &self,
        scenario: DecisionScenario,
        payload: Vec<u8>,
        source_snapshot: Vec<u8>,
    ) -> Binding {
        Binding {
            session_id: self.session_id.clone(),
            run_generation: self.run_generation,
            config_generation: current_config_generation(),
            scenario,
            model: self.config.settings.model.clone(),
            policy_version: self.config.settings.policy_version.clone(),
            origin: self.client.origin().to_owned(),
            path: self.client.path().to_owned(),
            payload,
            source_snapshot,
        }
    }

    /// Consume a grant.
    ///
    /// The grant is removed first: a second attempt finds nothing. Every
    /// mismatch afterwards is a hard rejection, because a grant that does
    /// not match the world it was minted in must not be usable at all.
    fn consume(
        &self,
        request_id: &str,
        expected: &Binding,
    ) -> Result<PendingDecision, DecisionFailure> {
        let pending = Self::lock(&self.pending)
            .remove(request_id)
            .ok_or_else(|| DecisionFailure::new("consent_expired", "the grant is gone"))?;
        debug_assert_eq!(pending.request_id, request_id);
        if pending.created.elapsed() > CONSENT_TTL {
            return Err(DecisionFailure::new(
                "consent_expired",
                "the grant is older than the consent window",
            ));
        }
        if pending.config_generation != expected.config_generation {
            return Err(DecisionFailure::new(
                "config_changed",
                "the configuration changed after the preview",
            ));
        }
        if pending.session_id != expected.session_id {
            return Err(DecisionFailure::new(
                "consent_expired",
                "the grant belongs to another session",
            ));
        }
        if pending.run_generation != expected.run_generation {
            return Err(DecisionFailure::new(
                "consent_expired",
                "the grant belongs to another run",
            ));
        }
        if pending.scenario != expected.scenario {
            return Err(DecisionFailure::new(
                "snapshot_changed",
                "the grant belongs to another scenario",
            ));
        }
        if pending.model != expected.model
            || pending.policy_version != expected.policy_version
            || pending.origin != expected.origin
            || pending.path != expected.path
        {
            return Err(DecisionFailure::new(
                "config_changed",
                "the destination or the model changed after the preview",
            ));
        }
        if pending.payload != expected.payload {
            return Err(DecisionFailure::new(
                "snapshot_changed",
                "the request changed after the preview",
            ));
        }
        if pending.source_snapshot != expected.source_snapshot {
            return Err(DecisionFailure::new(
                "snapshot_changed",
                "the source changed after the preview",
            ));
        }
        Ok(pending)
    }

    /// Take one of the two in-flight slots. `false` means the caller must
    /// skip: the service never queues behind itself.
    fn acquire_slot(&self) -> bool {
        let mut current = self.inflight.load(Ordering::SeqCst);
        loop {
            if current >= MAX_INFLIGHT as u64 {
                return false;
            }
            match self.inflight.compare_exchange(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    fn release_slot(&self) {
        let _ = self
            .inflight
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                Some(value.saturating_sub(1))
            });
    }
}

impl DecisionAccess for DecisionService {
    fn evaluate(&self, input: &DecisionInput, alive: &dyn Fn() -> bool) -> DecisionOutcome {
        let scenario = input.scenario();
        let started = Instant::now();

        let mut receipt = DecisionReceipt {
            request_id: String::new(),
            mode: self.config.settings.mode,
            provider: self.config.settings.provider,
            model: self.config.settings.model.clone(),
            policy_version: self.config.settings.policy_version.clone(),
            duration_ms: 0,
            consent_wait_ms: 0,
            input_tokens: None,
        };

        // 1. The configuration itself. An unusable record keeps the feature
        //    off; it never falls back to "on".
        if let Some(error) = &self.config.config_error {
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Skipped(kodo_agent::decision::DecisionSkip::new(
                    "config_invalid",
                    error.clone(),
                )),
            };
        }
        if self.config.settings.mode == DecisionMode::Off {
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Skipped(kodo_agent::decision::DecisionSkip::new(
                    "disabled",
                    "the decision component is off",
                )),
            };
        }
        if !self.config.settings.scenario_enabled(scenario) {
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Skipped(kodo_agent::decision::DecisionSkip::new(
                    "ineligible",
                    "the scenario is not enabled",
                )),
            };
        }

        // 2. A credential is required. Observe sends data too.
        let Some(credential) = self.config.credential.clone() else {
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Skipped(kodo_agent::decision::DecisionSkip::new(
                    "missing_credential",
                    "no credential is stored",
                )),
            };
        };

        // 3. The circuit breaker.
        {
            let mut breaker = Self::lock(&self.breaker);
            if breaker.auth_blocked {
                return DecisionOutcome {
                    receipt,
                    result: DecisionResult::Fallback(DecisionFailure::new(
                        "authentication",
                        "the stored credential was rejected; update it in the settings",
                    )),
                };
            }
            if let Some(until) = breaker.open_until {
                if Instant::now() < until {
                    return DecisionOutcome {
                        receipt,
                        result: DecisionResult::Fallback(DecisionFailure::new(
                            "circuit_open",
                            "the decision service is temporarily disabled after repeated failures",
                        )),
                    };
                }
                // The cooldown is over: let exactly one probe through.
                if breaker.probe_in_flight {
                    return DecisionOutcome {
                        receipt,
                        result: DecisionResult::Fallback(DecisionFailure::new(
                            "circuit_open",
                            "the decision service is running a recovery probe",
                        )),
                    };
                }
                breaker.probe_in_flight = true;
            }
        }

        // 4. The per-turn quotas.
        let max_calls = if self.config.settings.max_calls_per_turn == 0 {
            DEFAULT_MAX_CALLS_PER_TURN
        } else {
            self.config.settings.max_calls_per_turn
        };
        if self.calls_used.load(Ordering::SeqCst) >= max_calls as u64 {
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Fallback(DecisionFailure::new(
                    "budget_exhausted",
                    "the per-turn decision budget is used up",
                )),
            };
        }
        if self.transport_wait_ms.load(Ordering::SeqCst)
            >= self.config.settings.max_decision_time_ms_per_turn
        {
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Fallback(DecisionFailure::new(
                    "budget_exhausted",
                    "the per-turn decision time budget is used up",
                )),
            };
        }

        // 5. Build the minimal state and the frozen request.
        let plan = match input {
            DecisionInput::TaskRoute(TaskRouteInput { message, mode, .. }) => {
                plan_task_route(&TaskRouteInput {
                    message: message.clone(),
                    mode: mode.clone(),
                    baseline: kodo_agent::TaskType::Feature,
                })
            }
            DecisionInput::FailureTriage(triage) => plan_failure_triage(triage),
        };
        let source_snapshot = serde_json::to_vec(&plan.state).unwrap_or_else(|_| Vec::new());
        if source_snapshot.len() > self.config.settings.max_state_bytes {
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Fallback(DecisionFailure::new(
                    "payload_limit",
                    "the state is larger than the configured limit",
                )),
            };
        }
        // The screen is a screen, not a proof: it catches secret-shaped
        // material with the shared redactor, and it refuses to send rather
        // than to guess.
        if looks_sensitive(&String::from_utf8_lossy(&source_snapshot)) {
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Fallback(DecisionFailure::new(
                    "sensitive_input",
                    "the state looks like it carries secret material",
                )),
            };
        }
        // The provider decides the wire format; the state it carries is the
        // same minimal state in both cases.
        let body = match build_provider_request(
            self.config.settings.provider,
            &self.config.settings.model,
            &plan,
        ) {
            Ok(body) => body,
            Err(error) => {
                return DecisionOutcome {
                    receipt,
                    result: DecisionResult::Fallback(DecisionFailure::new(
                        "request_invalid",
                        error,
                    )),
                }
            }
        };
        let payload = body.bytes.clone();
        if payload.len() > self.config.settings.max_request_bytes {
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Fallback(DecisionFailure::new(
                    "payload_limit",
                    "the request is larger than the configured limit",
                )),
            };
        }

        // 6. The same snapshot is asked about once per run, and a scenario
        //    the user refused is not asked again in this run.
        let snapshot_key = format!(
            "{}:{}",
            scenario.label(),
            String::from_utf8_lossy(&source_snapshot)
        );
        // The refusal is checked first: it is the reason the user would
        // recognise ("I said no to this").
        if Self::lock(&self.denied).contains(&scenario) {
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Skipped(kodo_agent::decision::DecisionSkip::new(
                    "consent_denied",
                    "the user refused this scenario in this run",
                )),
            };
        }
        if Self::lock(&self.asked).contains(&snapshot_key) {
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Skipped(kodo_agent::decision::DecisionSkip::new(
                    "already_asked",
                    "this exact question was already asked in this run",
                )),
            };
        }

        // 7. Take a slot before the user is asked: a run that cannot be
        //    served must not open a prompt.
        if !self.acquire_slot() {
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Fallback(DecisionFailure::new(
                    "busy",
                    "too many decision requests are already in flight",
                )),
            };
        }
        let slot_taken = true;
        let release = || {
            if slot_taken {
                self.release_slot();
            }
        };

        // 8. The consent.
        let request_id = self.next_request_id();
        receipt.request_id = request_id.clone();
        let preview = DecisionPreview {
            request_id: request_id.clone(),
            scenario,
            mode: self.config.settings.mode,
            origin: self.client.origin().to_owned(),
            path: self.client.path().to_owned(),
            model: self.config.settings.model.clone(),
            policy_version: self.config.settings.policy_version.clone(),
            purpose: purpose_for(scenario),
            request_body: body.text.clone(),
            questions: questions_preview(&plan),
            request_bytes: payload.len(),
            state_bytes: source_snapshot.len(),
            expires_in_seconds: CONSENT_TTL.as_secs(),
        };
        let pending = PendingDecision {
            request_id: request_id.clone(),
            session_id: self.session_id.clone(),
            run_generation: self.run_generation,
            config_generation: current_config_generation(),
            scenario,
            origin: self.client.origin().to_owned(),
            path: self.client.path().to_owned(),
            model: self.config.settings.model.clone(),
            policy_version: self.config.settings.policy_version.clone(),
            payload: payload.clone(),
            source_snapshot: source_snapshot.clone(),
            created: Instant::now(),
        };
        Self::lock(&self.pending).insert(request_id.clone(), pending);
        Self::lock(&self.asked).insert(snapshot_key.clone());

        let consent_started = Instant::now();
        let answer = self.consent.request(&preview);
        receipt.consent_wait_ms = consent_started.elapsed().as_millis() as u64;
        match answer {
            DecisionConsentAnswer::Skip => {
                Self::lock(&self.pending).remove(&request_id);
                Self::lock(&self.denied).insert(scenario);
                release();
                receipt.duration_ms = started.elapsed().as_millis() as u64;
                return DecisionOutcome {
                    receipt,
                    result: DecisionResult::Skipped(kodo_agent::decision::DecisionSkip::new(
                        "consent_denied",
                        "the user did not allow this request",
                    )),
                };
            }
            DecisionConsentAnswer::Cancelled => {
                Self::lock(&self.pending).remove(&request_id);
                release();
                receipt.duration_ms = started.elapsed().as_millis() as u64;
                return DecisionOutcome {
                    receipt,
                    result: DecisionResult::Cancelled,
                };
            }
            DecisionConsentAnswer::Allow => {}
        }

        // 9. Consume the grant. From here on the bytes are frozen.
        let binding = self.current_binding(scenario, payload, source_snapshot);
        let pending = match self.consume(&request_id, &binding) {
            Ok(pending) => pending,
            Err(failure) => {
                release();
                receipt.duration_ms = started.elapsed().as_millis() as u64;
                return DecisionOutcome {
                    receipt,
                    result: DecisionResult::Fallback(failure),
                };
            }
        };

        // 10. Re-check the world before the request goes out.
        if !alive() {
            release();
            receipt.duration_ms = started.elapsed().as_millis() as u64;
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Cancelled,
            };
        }
        if current_config_generation() != pending.config_generation
            || self.config.settings.mode == DecisionMode::Off
        {
            release();
            receipt.duration_ms = started.elapsed().as_millis() as u64;
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Fallback(DecisionFailure::new(
                    "config_changed",
                    "the decision component was turned off or reconfigured",
                )),
            };
        }
        let _ = self.calls_used.fetch_add(1, Ordering::SeqCst);

        // 11. Send it.
        let transport_started = Instant::now();
        let response = self.client.post(
            &pending.payload,
            &credential,
            self.config.settings.request_timeout(),
            self.config.settings.max_response_bytes,
        );
        let waited = transport_started.elapsed().as_millis() as u64;
        let _ = self.transport_wait_ms.fetch_add(waited, Ordering::SeqCst);
        release();

        let response = match response {
            Ok(response) => {
                let mut breaker = Self::lock(&self.breaker);
                breaker.consecutive_failures = 0;
                breaker.open_until = None;
                breaker.probe_in_flight = false;
                response
            }
            Err(failure) => {
                self.record_transport_failure(&failure);
                receipt.duration_ms = started.elapsed().as_millis() as u64;
                return DecisionOutcome {
                    receipt,
                    result: DecisionResult::Fallback(failure),
                };
            }
        };

        // 12. Decode the answer. The provider decides the wire format; the
        //     internal shape is the same, so the validation order below is
        //     identical for every provider.
        let decoded = match self.config.settings.provider {
            DecisionProvider::TypeSafe => {
                let parsed: Result<kodo_agent::decision::DecisionResponse, DecisionFailure> =
                    serde_json::from_value(response).map_err(|error| {
                        DecisionFailure::new("invalid_response", format!("{error}"))
                    });
                parsed.map(|parsed| (parsed, None))
            }
            DecisionProvider::OpenRouter => {
                parse_openrouter_response(&response, &self.config.settings.model, &plan)
                    .map(|(parsed, answer)| (parsed, Some(answer)))
            }
        };
        let (response, answer) = match decoded {
            Ok(decoded) => decoded,
            Err(failure) => {
                receipt.duration_ms = started.elapsed().as_millis() as u64;
                return DecisionOutcome {
                    receipt,
                    result: DecisionResult::Fallback(failure),
                };
            }
        };

        // An answer without a probability distribution is recorded and
        // never adopted: the thresholds assume real probabilities.
        if let Some(OpenRouterAnswer::Uncalibrated { choice }) = answer {
            receipt.duration_ms = started.elapsed().as_millis() as u64;
            return DecisionOutcome {
                receipt,
                result: DecisionResult::Uncalibrated { choice },
            };
        }

        let request = kodo_agent::decision::build_request(&self.config.settings.model, &plan);
        let validated = match validate_response(&request, &response, scenario) {
            Ok(validated) => validated,
            Err(failure) => {
                receipt.duration_ms = started.elapsed().as_millis() as u64;
                return DecisionOutcome {
                    receipt,
                    result: DecisionResult::Fallback(failure),
                };
            }
        };
        receipt.input_tokens = validated.input_tokens;
        receipt.duration_ms = started.elapsed().as_millis() as u64;
        DecisionOutcome {
            receipt,
            result: DecisionResult::Suggested(validated),
        }
    }
}

impl DecisionService {
    /// Count a transport or service failure against the circuit breaker.
    ///
    /// A refusal, an ineligible scenario and a low-confidence answer are
    /// not failures — they never reach this function.
    fn record_transport_failure(&self, failure: &DecisionFailure) {
        let mut breaker = Self::lock(&self.breaker);
        if failure.code == "authentication" {
            // The credential is rejected: stop asking until it changes.
            breaker.auth_blocked = true;
            breaker.consecutive_failures = 0;
            return;
        }
        if !matches!(
            failure.code,
            "timeout" | "network" | "rate_limited" | "unavailable" | "redirect"
        ) {
            return;
        }
        breaker.consecutive_failures = breaker.consecutive_failures.saturating_add(1);
        if breaker.consecutive_failures >= BREAKER_THRESHOLD {
            breaker.open_until = Some(Instant::now() + BREAKER_COOLDOWN);
            breaker.probe_in_flight = false;
        }
    }
}

/// The questions and their answer space, as the consent preview shows them.
///
/// The option order is the same one the OpenRouter path maps its letters
/// through, so the preview tells the user exactly what the model will be
/// asked to choose between.
fn questions_preview(plan: &kodo_agent::decision::DecisionPlan) -> String {
    let options: Vec<serde_json::Value> = plan
        .question
        .options
        .iter()
        .map(|option| {
            serde_json::json!({
                "key": option,
                "meaning": plan.question.criteria.get(option).cloned().unwrap_or_default(),
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        plan.question_id.clone(): {
            "type": plan.question.kind,
            "instructions": plan.question.instructions,
            "options": options,
        }
    }))
    .unwrap_or_default()
}

/// A short, local description of what a request is for.
fn purpose_for(scenario: DecisionScenario) -> String {
    match scenario {
        DecisionScenario::TaskRoute => {
            "判断这次任务应归入哪一类工作流程（只影响任务分类，不改变权限与预算）".to_owned()
        }
        DecisionScenario::FailureTriage => {
            "判断这次失败是产品问题、环境问题还是短暂故障（不解除任何拒绝，也不增加修复次数）"
                .to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kodo_agent::decision::{DecisionSkip, FailureTriageInput, ValidatedDecision};

    /// A consent host that answers from a script.
    struct ScriptedConsent {
        answers: Mutex<Vec<DecisionConsentAnswer>>,
        seen: Mutex<Vec<DecisionPreview>>,
    }

    impl ScriptedConsent {
        fn new(answers: Vec<DecisionConsentAnswer>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers),
                seen: Mutex::new(Vec::new()),
            })
        }
    }

    impl DecisionConsent for ScriptedConsent {
        fn request(&self, preview: &DecisionPreview) -> DecisionConsentAnswer {
            self.seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(preview.clone());
            let mut answers = self
                .answers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if answers.is_empty() {
                DecisionConsentAnswer::Skip
            } else {
                answers.remove(0)
            }
        }
    }

    fn config(mode: DecisionMode, credential: Option<&str>) -> DecisionConfig {
        DecisionConfig {
            settings: DecisionSettings {
                mode,
                task_route_enabled: true,
                failure_triage_enabled: true,
                ..DecisionSettings::default()
            },
            credential: credential.map(|value| value.to_owned()),
            config_generation: current_config_generation(),
            config_error: None,
        }
    }

    fn task_route_input() -> DecisionInput {
        DecisionInput::TaskRoute(TaskRouteInput {
            message: "把那个东西弄好".to_owned(),
            mode: "code".to_owned(),
            baseline: kodo_agent::TaskType::Feature,
        })
    }

    fn service(config: DecisionConfig, consent: Arc<dyn DecisionConsent>) -> Arc<DecisionService> {
        Arc::new(DecisionService::new(
            config,
            DecisionClient::for_local_endpoint(DecisionProvider::TypeSafe, "http://127.0.0.1:1"),
            consent,
            "session-1",
            7,
        ))
    }

    #[test]
    fn off_never_asks_the_host() {
        let consent = ScriptedConsent::new(vec![DecisionConsentAnswer::Allow]);
        let service = service(config(DecisionMode::Off, Some("secret")), consent.clone());
        let outcome = service.evaluate(&task_route_input(), &|| true);
        assert!(matches!(
            outcome.result,
            DecisionResult::Skipped(DecisionSkip {
                code: "disabled",
                ..
            })
        ));
        assert!(consent.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_missing_credential_is_skipped() {
        let consent = ScriptedConsent::new(vec![DecisionConsentAnswer::Allow]);
        let service = service(config(DecisionMode::Observe, None), consent.clone());
        let outcome = service.evaluate(&task_route_input(), &|| true);
        assert!(matches!(
            outcome.result,
            DecisionResult::Skipped(DecisionSkip {
                code: "missing_credential",
                ..
            })
        ));
        assert!(consent.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_disabled_scenario_is_skipped() {
        let consent = ScriptedConsent::new(vec![DecisionConsentAnswer::Allow]);
        let mut cfg = config(DecisionMode::Observe, Some("secret"));
        cfg.settings.failure_triage_enabled = false;
        let service = service(cfg, consent.clone());
        let outcome = service.evaluate(
            &DecisionInput::FailureTriage(FailureTriageInput {
                command_category: "rust_check".to_owned(),
                exit_code: Some(1),
                known_flags: vec![],
                error_excerpt: "boom".to_owned(),
            }),
            &|| true,
        );
        assert!(matches!(
            outcome.result,
            DecisionResult::Skipped(DecisionSkip {
                code: "ineligible",
                ..
            })
        ));
    }

    #[test]
    fn an_invalid_configuration_keeps_the_feature_off() {
        let consent = ScriptedConsent::new(vec![DecisionConsentAnswer::Allow]);
        let mut cfg = config(DecisionMode::Observe, Some("secret"));
        cfg.config_error = Some("jev-decision: unknown mode `maybe`".to_owned());
        let service = service(cfg, consent.clone());
        let outcome = service.evaluate(&task_route_input(), &|| true);
        assert!(matches!(
            outcome.result,
            DecisionResult::Skipped(DecisionSkip {
                code: "config_invalid",
                ..
            })
        ));
        assert!(consent.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_refusal_is_remembered_for_the_rest_of_the_run() {
        let consent = ScriptedConsent::new(vec![
            DecisionConsentAnswer::Skip,
            DecisionConsentAnswer::Allow,
        ]);
        let service = service(
            config(DecisionMode::Observe, Some("secret")),
            consent.clone(),
        );
        let first = service.evaluate(&task_route_input(), &|| true);
        assert!(matches!(
            first.result,
            DecisionResult::Skipped(DecisionSkip {
                code: "consent_denied",
                ..
            })
        ));
        // The second attempt must not open a prompt at all.
        let second = service.evaluate(&task_route_input(), &|| true);
        assert!(matches!(
            second.result,
            DecisionResult::Skipped(DecisionSkip {
                code: "consent_denied",
                ..
            })
        ));
        assert_eq!(consent.seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_preview_shows_the_exact_bytes() {
        let consent = ScriptedConsent::new(vec![DecisionConsentAnswer::Skip]);
        let service = service(
            config(DecisionMode::Observe, Some("secret")),
            consent.clone(),
        );
        service.evaluate(&task_route_input(), &|| true);
        let seen = consent.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let preview = &seen[0];
        assert!(
            preview.request_body.contains("把那个东西弄好"),
            "{preview:?}"
        );
        assert!(preview.questions.contains("task_type"), "{preview:?}");
        assert_eq!(preview.request_bytes, preview.request_body.len());
        assert!(preview.request_bytes > 0);
        assert_eq!(preview.expires_in_seconds, 300);
    }

    #[test]
    fn a_cancelled_consent_stops_the_round() {
        let consent = ScriptedConsent::new(vec![DecisionConsentAnswer::Cancelled]);
        let service = service(config(DecisionMode::Observe, Some("secret")), consent);
        let outcome = service.evaluate(&task_route_input(), &|| true);
        assert!(matches!(outcome.result, DecisionResult::Cancelled));
    }

    #[test]
    fn a_grant_is_consumed_once() {
        let consent = ScriptedConsent::new(vec![DecisionConsentAnswer::Allow]);
        let service = service(config(DecisionMode::Observe, Some("secret")), consent);
        let pending = PendingDecision {
            request_id: "req-1".to_owned(),
            session_id: "session-1".to_owned(),
            run_generation: 7,
            config_generation: current_config_generation(),
            scenario: DecisionScenario::TaskRoute,
            origin: "http://127.0.0.1:1".to_owned(),
            path: "/v1/systemone".to_owned(),
            model: "jev-1.13.0".to_owned(),
            policy_version: "kodo-jev-v1".to_owned(),
            payload: b"{}".to_vec(),
            source_snapshot: b"{}".to_vec(),
            created: Instant::now(),
        };
        DecisionService::lock(&service.pending).insert("req-1".to_owned(), pending);
        let binding =
            service.current_binding(DecisionScenario::TaskRoute, b"{}".to_vec(), b"{}".to_vec());
        assert!(service.consume("req-1", &binding).is_ok());
        // The second attempt finds nothing.
        assert!(service.consume("req-1", &binding).is_err());
    }

    #[test]
    fn a_changed_payload_invalidates_the_grant() {
        let consent = ScriptedConsent::new(vec![DecisionConsentAnswer::Allow]);
        let service = service(config(DecisionMode::Observe, Some("secret")), consent);
        let pending = PendingDecision {
            request_id: "req-1".to_owned(),
            session_id: "session-1".to_owned(),
            run_generation: 7,
            config_generation: current_config_generation(),
            scenario: DecisionScenario::TaskRoute,
            origin: "http://127.0.0.1:1".to_owned(),
            path: "/v1/systemone".to_owned(),
            model: "jev-1.13.0".to_owned(),
            policy_version: "kodo-jev-v1".to_owned(),
            payload: b"{\"a\":1}".to_vec(),
            source_snapshot: b"{}".to_vec(),
            created: Instant::now(),
        };
        DecisionService::lock(&service.pending).insert("req-1".to_owned(), pending);
        let binding = service.current_binding(
            DecisionScenario::TaskRoute,
            b"{\"a\":2}".to_vec(),
            b"{}".to_vec(),
        );
        let error = service.consume("req-1", &binding).unwrap_err();
        assert_eq!(error.code, "snapshot_changed");
    }

    #[test]
    fn another_sessions_grant_is_refused() {
        let consent = ScriptedConsent::new(vec![DecisionConsentAnswer::Allow]);
        let service = service(config(DecisionMode::Observe, Some("secret")), consent);
        let pending = PendingDecision {
            request_id: "req-1".to_owned(),
            session_id: "another-session".to_owned(),
            run_generation: 7,
            config_generation: current_config_generation(),
            scenario: DecisionScenario::TaskRoute,
            origin: "http://127.0.0.1:1".to_owned(),
            path: "/v1/systemone".to_owned(),
            model: "jev-1.13.0".to_owned(),
            policy_version: "kodo-jev-v1".to_owned(),
            payload: b"{}".to_vec(),
            source_snapshot: b"{}".to_vec(),
            created: Instant::now(),
        };
        DecisionService::lock(&service.pending).insert("req-1".to_owned(), pending);
        let binding =
            service.current_binding(DecisionScenario::TaskRoute, b"{}".to_vec(), b"{}".to_vec());
        let error = service.consume("req-1", &binding).unwrap_err();
        assert_eq!(error.code, "consent_expired");
    }

    #[test]
    fn a_changed_configuration_invalidates_the_grant() {
        let consent = ScriptedConsent::new(vec![DecisionConsentAnswer::Allow]);
        let service = service(config(DecisionMode::Observe, Some("secret")), consent);
        let pending = PendingDecision {
            request_id: "req-1".to_owned(),
            session_id: "session-1".to_owned(),
            run_generation: 7,
            config_generation: current_config_generation(),
            scenario: DecisionScenario::TaskRoute,
            origin: "http://127.0.0.1:1".to_owned(),
            path: "/v1/systemone".to_owned(),
            model: "jev-1.13.0".to_owned(),
            policy_version: "kodo-jev-v1".to_owned(),
            payload: b"{}".to_vec(),
            source_snapshot: b"{}".to_vec(),
            created: Instant::now(),
        };
        DecisionService::lock(&service.pending).insert("req-1".to_owned(), pending);
        let mut binding =
            service.current_binding(DecisionScenario::TaskRoute, b"{}".to_vec(), b"{}".to_vec());
        binding.config_generation = current_config_generation() + 99;
        let error = service.consume("req-1", &binding).unwrap_err();
        assert_eq!(error.code, "config_changed");
    }

    #[test]
    fn the_per_turn_budget_stops_further_requests() {
        let consent = ScriptedConsent::new(vec![
            DecisionConsentAnswer::Allow,
            DecisionConsentAnswer::Allow,
            DecisionConsentAnswer::Allow,
            DecisionConsentAnswer::Allow,
        ]);
        let mut cfg = config(DecisionMode::Observe, Some("secret"));
        cfg.settings.max_calls_per_turn = 1;
        let service = service(cfg, consent);
        // The first request fails to reach the (unreachable) endpoint, but it
        // still counts against the budget.
        let _ = service.evaluate(&task_route_input(), &|| true);
        let second = service.evaluate(
            &DecisionInput::FailureTriage(FailureTriageInput {
                command_category: "rust_check".to_owned(),
                exit_code: Some(1),
                known_flags: vec![],
                error_excerpt: "boom".to_owned(),
            }),
            &|| true,
        );
        assert!(matches!(
            second.result,
            DecisionResult::Fallback(DecisionFailure {
                code: "budget_exhausted",
                ..
            })
        ));
    }

    #[test]
    fn a_secret_shaped_state_is_never_sent() {
        let consent = ScriptedConsent::new(vec![DecisionConsentAnswer::Allow]);
        let service = service(
            config(DecisionMode::Observe, Some("secret")),
            consent.clone(),
        );
        let outcome = service.evaluate(
            &DecisionInput::FailureTriage(FailureTriageInput {
                command_category: "generic".to_owned(),
                exit_code: Some(1),
                known_flags: vec![],
                error_excerpt: "Authorization: Bearer abcdefghijklmnop".to_owned(),
            }),
            &|| true,
        );
        assert!(matches!(
            outcome.result,
            DecisionResult::Fallback(DecisionFailure {
                code: "sensitive_input",
                ..
            })
        ));
        assert!(consent.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn an_oversized_state_is_refused_before_the_prompt() {
        let consent = ScriptedConsent::new(vec![DecisionConsentAnswer::Allow]);
        let mut cfg = config(DecisionMode::Observe, Some("secret"));
        cfg.settings.max_state_bytes = 256;
        let service = service(cfg, consent.clone());
        let outcome = service.evaluate(
            &DecisionInput::FailureTriage(FailureTriageInput {
                command_category: "generic".to_owned(),
                exit_code: Some(1),
                known_flags: vec![],
                error_excerpt: "x".repeat(1024),
            }),
            &|| true,
        );
        assert!(matches!(
            outcome.result,
            DecisionResult::Fallback(DecisionFailure {
                code: "payload_limit",
                ..
            })
        ));
        assert!(consent.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn the_same_snapshot_is_asked_once_per_run() {
        let consent = ScriptedConsent::new(vec![
            DecisionConsentAnswer::Allow,
            DecisionConsentAnswer::Allow,
        ]);
        let service = service(
            config(DecisionMode::Observe, Some("secret")),
            consent.clone(),
        );
        let _ = service.evaluate(&task_route_input(), &|| true);
        let second = service.evaluate(&task_route_input(), &|| true);
        assert!(matches!(
            second.result,
            DecisionResult::Skipped(DecisionSkip {
                code: "already_asked",
                ..
            })
        ));
        assert_eq!(consent.seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_cancelled_run_never_sends() {
        let consent = ScriptedConsent::new(vec![DecisionConsentAnswer::Allow]);
        let service = service(config(DecisionMode::Observe, Some("secret")), consent);
        let outcome = service.evaluate(&task_route_input(), &|| false);
        assert!(matches!(outcome.result, DecisionResult::Cancelled));
    }

    #[test]
    fn a_saved_record_can_be_read_back() {
        // The settings log is line-based: a record written as more than
        // one line is cut at the first newline and comes back as a
        // truncated object. The round trip is the contract.
        let dir = std::env::temp_dir().join(format!("kodo-jev-roundtrip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("settings.log");
        let raw = r#"{"schemaVersion":1,"provider":"openRouter","mode":"observe",
            "model":"typesafe/jev-1.14","scenarios":{"taskRoute":true,"failureTriage":false},
            "requestTimeoutMs":1500,"maxCallsPerTurn":3,"maxRequestBytes":32768,
            "maxStateBytes":16384,"maxResponseBytes":65536,
            "maxDecisionTimeMsPerTurn":4500,"policyVersion":"kodo-jev-v1"}"#;
        save_decision_settings_to(&path, raw).expect("save");

        // The stored value is one line.
        let text = std::fs::read_to_string(&path).expect("read the log");
        let stored = text
            .lines()
            .find(|line| line.starts_with("jev-decision="))
            .expect("the record must be one line");
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("jev-decision="))
                .count(),
            1,
            "the record must occupy exactly one line"
        );
        assert!(!stored["jev-decision=".len()..].contains('\n'));

        // And it parses back into the same configuration.
        let config = load_decision_config_from(Some(&path), None);
        assert!(config.config_error.is_none(), "{:?}", config.config_error);
        assert_eq!(config.settings.mode, DecisionMode::Observe);
        assert_eq!(config.settings.provider, DecisionProvider::OpenRouter);
        assert_eq!(config.settings.model, "typesafe/jev-1.14");
        assert!(config.settings.task_route_enabled);
        assert!(!config.settings.failure_triage_enabled);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pretty_printed_record_is_normalized_before_it_is_written() {
        let dir = std::env::temp_dir().join(format!("kodo-jev-pretty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("settings.log");
        let pretty = "{\n  \"schemaVersion\": 1,\n  \"mode\": \"observe\"\n}";
        save_decision_settings_to(&path, pretty).expect("save");
        let config = load_decision_config_from(Some(&path), None);
        assert!(config.config_error.is_none(), "{:?}", config.config_error);
        assert_eq!(config.settings.mode, DecisionMode::Observe);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_damaged_record_says_how_to_repair_it() {
        // The settings log is line-based: a record written as more than
        // one line comes back as a truncated object. The message has to
        // say what to do, not just repeat the parser's complaint.
        let dir = std::env::temp_dir().join(format!("kodo-jev-damaged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("settings.log");
        // What the old pretty-printed writer left behind.
        std::fs::write(
            &path,
            "jev-decision={\n  \"schemaVersion\": 1,\n  \"mode\": \"observe\"\n}\n",
        )
        .expect("write the damaged record");
        let config = load_decision_config_from(Some(&path), None);
        let error = config
            .config_error
            .expect("the damaged record must be reported");
        assert!(error.contains("save the configuration again"), "{error}");
        // The feature stays off rather than falling back to "on".
        assert_eq!(config.settings.mode, DecisionMode::Off);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_openrouter_default_model_is_the_typesafe_one() {
        assert_eq!(
            DecisionProvider::OpenRouter.default_model(),
            "typesafe/jev-1.14"
        );
        assert_eq!(DecisionProvider::TypeSafe.default_model(), "jev-1.13.0");
    }

    #[test]
    fn clearing_one_providers_credential_keeps_the_other_one() {
        // The two providers have separate keys; clearing one must not take
        // the other with it.
        let dir = std::env::temp_dir().join(format!("kodo-jev-providers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("credentials.log");
        kodo_core::settings::write_secret(
            &path,
            kodo_agent::decision::CREDENTIAL_NAMESPACE,
            DecisionProvider::TypeSafe.credential_id(),
            "type-safe-key",
        )
        .expect("type safe credential");
        kodo_core::settings::write_secret(
            &path,
            kodo_agent::decision::CREDENTIAL_NAMESPACE,
            DecisionProvider::OpenRouter.credential_id(),
            "openrouter-key",
        )
        .expect("openrouter credential");

        // `clear_decision_credential` reads the real store, so exercise the
        // namespace rebuild directly with the same keep-set shape.
        let keep = vec![(
            DecisionProvider::OpenRouter.credential_id().to_owned(),
            "openrouter-key".to_owned(),
        )];
        kodo_core::settings::clear_secrets(
            &path,
            kodo_agent::decision::CREDENTIAL_NAMESPACE,
            &keep,
        )
        .expect("clear one provider");

        assert_eq!(
            kodo_core::settings::read_secret(
                &path,
                kodo_agent::decision::CREDENTIAL_NAMESPACE,
                DecisionProvider::TypeSafe.credential_id(),
            ),
            None,
            "the cleared provider's key must be gone"
        );
        assert_eq!(
            kodo_core::settings::read_secret(
                &path,
                kodo_agent::decision::CREDENTIAL_NAMESPACE,
                DecisionProvider::OpenRouter.credential_id(),
            )
            .as_deref(),
            Some("openrouter-key"),
            "the other provider's key must survive"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn clearing_the_decision_credential_leaves_the_others_alone() {
        let dir = std::env::temp_dir().join(format!("kodo-jev-credentials-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("credentials.log");
        kodo_core::settings::write_credential(&path, "provider-a", "provider-secret")
            .expect("provider credential");
        kodo_core::settings::write_secret(&path, "mcp", "srv", "{\"env\":{}}")
            .expect("mcp credential");
        kodo_core::settings::write_secret(
            &path,
            kodo_agent::decision::CREDENTIAL_NAMESPACE,
            kodo_agent::decision::CREDENTIAL_ID,
            "jev-secret",
        )
        .expect("jev credential");

        kodo_core::settings::clear_secrets(&path, kodo_agent::decision::CREDENTIAL_NAMESPACE, &[])
            .expect("clear the decision namespace");

        assert_eq!(
            kodo_core::settings::read_credential(&path, "provider-a").as_deref(),
            Some("provider-secret"),
            "another provider's credential must survive"
        );
        assert_eq!(
            kodo_core::settings::read_secret(&path, "mcp", "srv").as_deref(),
            Some("{\"env\":{}}"),
            "an MCP credential must survive"
        );
        assert_eq!(
            kodo_core::settings::read_secret(
                &path,
                kodo_agent::decision::CREDENTIAL_NAMESPACE,
                kodo_agent::decision::CREDENTIAL_ID,
            ),
            None,
            "the decision credential must be gone"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_decision_record_never_reaches_the_memory_outbox() {
        // The record rides on a reasoning item. The outbox bridge drops
        // every reasoning kind, so a decision record can never be recalled
        // into a later turn.
        use kodo_core::session::{Item, ItemKind, Phase};
        let item = Item {
            id: 1,
            at: 0,
            duration_ms: Some(0),
            status: kodo_core::session::Status::Done,
            kind: ItemKind::Reasoning {
                summary: "决策辅助建议：refactor；未采纳".to_owned(),
                phase: "analyze".to_owned(),
                diagnostics: Some("{\"schema\":\"decision.v1\"}".to_owned()),
            },
        };
        assert!(
            crate::memory_outbox::event_for_item(&item, Phase::Completed).is_none(),
            "a decision record must not become a memory event"
        );
    }

    #[test]
    fn the_secret_is_masked_for_the_settings_screen() {
        assert_eq!(mask_secret("sk-abcdefgh"), "••••efgh");
        assert_eq!(mask_secret(""), "");
    }

    #[test]
    fn a_validated_decision_carries_no_request_content() {
        let decision = ValidatedDecision {
            scenario: DecisionScenario::TaskRoute,
            model: "jev-1.13.0".to_owned(),
            question_id: "task_type".to_owned(),
            choice: "refactor".to_owned(),
            probabilities: [("refactor".to_owned(), 0.95)].into_iter().collect(),
            confidence: Some(0.9),
            input_tokens: Some(120),
        };
        assert_eq!(decision.choice, "refactor");
    }
}
