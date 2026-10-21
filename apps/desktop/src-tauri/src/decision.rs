//! The desktop's decision-assistance bridge.
//!
//! The desktop is the only surface that can show a send preview and wait
//! for an answer, so it is the only place a decision request may be
//! approved. The bridge keeps three things apart:
//!
//! - the **settings**, stored in the ordinary local stores and edited from
//!   the settings screen;
//! - the **pending consent**, which lives in memory for the length of one
//!   request and is answered by a request id the frontend received in a
//!   preview event;
//! - the **run service**, which owns the credential, the quotas and the
//!   transport for exactly one run.
//!
//! The frontend never sends a request body, a destination or a session id
//! it made up: it only answers "may this request that is already on screen
//! go out?".

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kodo_agent::decision::DecisionProvider;
use kodo_shell::decision::{
    DecisionConsent, DecisionConsentAnswer, DecisionPreview, DecisionService,
};
use kodo_shell::{
    clear_decision_credential, decision_settings_view, save_decision_credential,
    save_decision_settings, DecisionSettingsView,
};
use tauri::State;

use crate::view::RunEvent;

/// The decision settings, as the settings screen needs them.
#[tauri::command]
pub fn decision_get_settings() -> DecisionSettingsView {
    decision_settings_view()
}

/// Store the decision settings.
///
/// The record is validated before it is written: an invalid mode, an
/// unknown policy version or an out-of-range quota is refused with a
/// message the screen can show, and the stored configuration is left
/// alone.
#[tauri::command]
pub fn decision_save_settings(settings: String) -> Result<u64, String> {
    save_decision_settings(&settings)
}

/// Store a provider's decision credential.
///
/// The provider names which key this is: every provider's key lives under
/// its own id, so one service's key is never sent to another.
#[tauri::command]
pub fn decision_set_credential(provider: String, secret: String) -> Result<u64, String> {
    let provider = DecisionProvider::parse(&provider)
        .ok_or_else(|| format!("unknown decision provider `{provider}`"))?;
    save_decision_credential(provider, &secret)
}

/// Remove one provider's decision credential. The other providers' keys and
/// every other credential family are untouched.
#[tauri::command]
pub fn decision_clear_credential(provider: String) -> Result<u64, String> {
    let provider = DecisionProvider::parse(&provider)
        .ok_or_else(|| format!("unknown decision provider `{provider}`"))?;
    clear_decision_credential(provider)
}

/// Answer a pending send preview.
///
/// The request id is the only thing the frontend supplies. The session and
/// the run come from the backend's own record, and the grant is consumed
/// once — a second answer, or an answer for a request that is already
/// gone, fails.
#[tauri::command]
pub fn decision_confirm_preview(
    decisions: State<'_, DecisionRegistry>,
    request_id: String,
    approved: bool,
) -> Result<(), String> {
    if decisions.resolve(&request_id, approved) {
        Ok(())
    } else {
        Err("the preview has expired".into())
    }
}

/// The preview a running conversation is still waiting on, in the form the
/// GUI needs to put the dialog back after the user returns to it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPreviewView {
    pub session: String,
    pub request_id: String,
    pub run_generation: u64,
    pub scenario: String,
    pub mode: String,
    pub origin: String,
    pub path: String,
    pub model: String,
    pub purpose: String,
    pub request_body: String,
    pub questions: String,
    pub request_bytes: usize,
    pub state_bytes: usize,
    pub expires_in_seconds: u64,
}

/// Recover the preview a live conversation is still waiting on.
///
/// The run event that carried the preview was addressed to the conversation
/// that was open at the time. A user who switched away must see the same
/// dialog when they come back, or the run would wait for an answer nobody
/// knows about.
#[tauri::command]
pub fn decision_pending_preview(
    decisions: State<'_, DecisionRegistry>,
    session: String,
) -> Option<PendingPreviewView> {
    decisions.pending_preview(&session)
}

/// One consent waiting for an answer.
struct PendingConsent {
    session_id: String,
    run_generation: u64,
    /// Kept whole so a conversation opened later can show the same bytes.
    preview: DecisionPreview,
    /// The shell grants expire after the preview's advertised lifetime; the
    /// wait must not outlive them.
    expires_at: Instant,
    sender: mpsc::Sender<DecisionConsentAnswer>,
}

#[derive(Default)]
struct RegistryInner {
    /// request id → the consent waiting for an answer.
    pending: Mutex<HashMap<String, PendingConsent>>,
    /// session id → (run generation, the run's decision service).
    runs: Mutex<HashMap<String, (u64, Arc<DecisionService>)>>,
}

/// The registry the IPC commands talk to.
#[derive(Clone, Default)]
pub struct DecisionRegistry {
    inner: Arc<RegistryInner>,
}

impl DecisionRegistry {
    /// Remember a run's decision service so the consent commands can find
    /// it. The run generation is kept with it: a consent from an earlier
    /// run must not be answerable while a later run owns the session.
    pub fn register_run(
        &self,
        session_id: &str,
        run_generation: u64,
        service: Arc<DecisionService>,
    ) {
        self.inner
            .runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session_id.to_owned(), (run_generation, service));
    }

    /// Forget a run's decision service. Called when the run ends so a
    /// later run cannot reuse the old one.
    pub fn release_run(&self, session_id: &str) {
        self.inner
            .runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session_id);
    }

    /// Drop every pending consent of a session. Used when the run is
    /// stopped: a closed window must not be able to revive a request.
    pub fn clear_session(&self, session_id: &str) {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_, pending| pending.session_id != session_id);
    }

    /// The preview this session's current run is waiting on.
    ///
    /// A preview from a run that no longer owns the session is not returned:
    /// re-opening a conversation must never revive an answer from a finished
    /// run.
    pub fn pending_preview(&self, session_id: &str) -> Option<PendingPreviewView> {
        let (preview, expires_at, run_generation) = {
            let pending = self
                .inner
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let entry = pending
                .values()
                .find(|pending| pending.session_id == session_id)?;
            (
                entry.preview.clone(),
                entry.expires_at,
                entry.run_generation,
            )
        };
        let current = self
            .inner
            .runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session_id)
            .map(|(generation, _)| *generation);
        if current != Some(run_generation) {
            return None;
        }
        let remaining = expires_at.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        Some(PendingPreviewView {
            session: session_id.to_owned(),
            request_id: preview.request_id,
            run_generation,
            scenario: preview.scenario.label().to_owned(),
            mode: preview.mode.label().to_owned(),
            origin: preview.origin,
            path: preview.path,
            model: preview.model,
            purpose: preview.purpose,
            request_body: preview.request_body,
            questions: preview.questions,
            request_bytes: preview.request_bytes,
            state_bytes: preview.state_bytes,
            expires_in_seconds: remaining.as_secs(),
        })
    }

    /// Answer a pending consent.
    ///
    /// The request id is the only thing the frontend supplies. The session
    /// and the run generation come from the record the backend kept, so a
    /// frontend cannot answer another conversation's request.
    pub fn resolve(&self, request_id: &str, approved: bool) -> bool {
        let pending = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(request_id);
        let Some(pending) = pending else {
            return false;
        };
        // The run that asked must still own the session. A grant from a
        // finished run cannot be revived by a later answer.
        let current = self
            .inner
            .runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&pending.session_id)
            .map(|(generation, _)| *generation);
        if current != Some(pending.run_generation) {
            return false;
        }
        let answer = if approved {
            DecisionConsentAnswer::Allow
        } else {
            DecisionConsentAnswer::Skip
        };
        pending.sender.send(answer).is_ok()
    }

    /// How many consents are waiting. Used by the tests and the status
    /// read-out.
    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

/// The desktop's consent host.
///
/// It shows the preview through the existing run-event bridge and waits
/// for the answer, exactly like the command-approval rendezvous does. The
/// wait is cancellation-aware: a stopped run must not keep the thread
/// parked.
pub struct DesktopDecisionConsent {
    registry: DecisionRegistry,
    emit: Box<dyn Fn(RunEvent) + Send + Sync>,
    session_id: String,
    run_generation: u64,
    alive: Arc<AtomicBool>,
}

impl DesktopDecisionConsent {
    pub fn new(
        registry: DecisionRegistry,
        emit: Box<dyn Fn(RunEvent) + Send + Sync>,
        session_id: String,
        run_generation: u64,
        alive: Arc<AtomicBool>,
    ) -> Self {
        Self {
            registry,
            emit,
            session_id,
            run_generation,
            alive,
        }
    }
}

impl DecisionConsent for DesktopDecisionConsent {
    fn request(&self, preview: &DecisionPreview) -> DecisionConsentAnswer {
        let (sender, receiver) = mpsc::channel();
        let expires_at = Instant::now() + Duration::from_secs(preview.expires_in_seconds);
        self.registry
            .inner
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                preview.request_id.clone(),
                PendingConsent {
                    session_id: self.session_id.clone(),
                    run_generation: self.run_generation,
                    preview: preview.clone(),
                    expires_at,
                    sender,
                },
            );
        (self.emit)(RunEvent::DecisionPreview {
            session: self.session_id.clone(),
            request_id: preview.request_id.clone(),
            run_generation: self.run_generation,
            scenario: preview.scenario.label().to_owned(),
            mode: preview.mode.label().to_owned(),
            origin: preview.origin.clone(),
            path: preview.path.clone(),
            model: preview.model.clone(),
            purpose: preview.purpose.clone(),
            request_body: preview.request_body.clone(),
            questions: preview.questions.clone(),
            request_bytes: preview.request_bytes,
            state_bytes: preview.state_bytes,
            expires_in_seconds: preview.expires_in_seconds,
        });

        // The wait is a poll, not one blocking receive: a stopped run has
        // to release this thread promptly. It also ends with the grant: a
        // preview nobody answers must not park the run forever.
        let answer = loop {
            if !self.alive.load(Ordering::SeqCst) {
                break DecisionConsentAnswer::Cancelled;
            }
            let now = Instant::now();
            if now >= expires_at {
                break DecisionConsentAnswer::Skip;
            }
            let wait = expires_at
                .saturating_duration_since(now)
                .min(Duration::from_millis(100));
            match receiver.recv_timeout(wait) {
                Ok(answer) => break answer,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break DecisionConsentAnswer::Skip,
            }
        };
        self.registry
            .inner
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&preview.request_id);
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kodo_agent::decision::{DecisionClient, DecisionMode, DecisionScenario, DecisionSettings};
    use kodo_shell::decision::{DecisionConfig, DecisionPreview};

    fn preview(expires_in_seconds: u64) -> DecisionPreview {
        DecisionPreview {
            request_id: "req-1".to_owned(),
            scenario: DecisionScenario::TaskRoute,
            mode: DecisionMode::Observe,
            origin: "https://example.invalid".to_owned(),
            path: "/v1/systemone".to_owned(),
            model: "jev-1.13.0".to_owned(),
            policy_version: "kodo-jev-v1".to_owned(),
            purpose: "test".to_owned(),
            request_body: "{}".to_owned(),
            questions: "[]".to_owned(),
            request_bytes: 2,
            state_bytes: 2,
            expires_in_seconds,
        }
    }

    fn config() -> DecisionConfig {
        DecisionConfig {
            settings: DecisionSettings {
                mode: DecisionMode::Observe,
                task_route_enabled: true,
                ..DecisionSettings::default()
            },
            credential: Some("synthetic".to_owned()),
            config_generation: kodo_shell::decision::current_config_generation(),
            config_error: None,
            recovered: false,
        }
    }

    #[test]
    fn an_unknown_request_id_cannot_be_answered() {
        let registry = DecisionRegistry::default();
        assert!(!registry.resolve("nope", true));
    }

    #[test]
    fn a_consent_is_answered_once() {
        let registry = DecisionRegistry::default();
        registry.register_run("s", 4, test_service());
        let (sender, receiver) = mpsc::channel();
        registry.inner.pending.lock().unwrap().insert(
            "req-1".to_owned(),
            PendingConsent {
                session_id: "s".to_owned(),
                run_generation: 4,
                preview: preview(300),
                expires_at: Instant::now() + Duration::from_secs(300),
                sender,
            },
        );
        assert!(registry.resolve("req-1", true));
        assert_eq!(receiver.recv().unwrap(), DecisionConsentAnswer::Allow);
        // A second answer finds nothing.
        assert!(!registry.resolve("req-1", true));
    }

    #[test]
    fn clearing_a_session_drops_its_consents() {
        let registry = DecisionRegistry::default();
        let (sender, _receiver) = mpsc::channel();
        registry.inner.pending.lock().unwrap().insert(
            "req-1".to_owned(),
            PendingConsent {
                session_id: "s".to_owned(),
                run_generation: 1,
                preview: preview(300),
                expires_at: Instant::now() + Duration::from_secs(300),
                sender,
            },
        );
        registry.clear_session("s");
        assert_eq!(registry.pending_count(), 0);
    }

    #[test]
    fn a_pending_preview_is_visible_only_to_its_current_run() {
        let registry = DecisionRegistry::default();
        registry.register_run("s", 4, test_service());
        let (sender, _receiver) = mpsc::channel();
        registry.inner.pending.lock().unwrap().insert(
            "req-1".to_owned(),
            PendingConsent {
                session_id: "s".to_owned(),
                run_generation: 4,
                preview: preview(300),
                expires_at: Instant::now() + Duration::from_secs(300),
                sender,
            },
        );
        let visible = registry
            .pending_preview("s")
            .expect("the current run's preview is visible");
        assert_eq!(visible.request_id, "req-1");
        assert_eq!(visible.scenario, "taskRoute");
        assert!(visible.expires_in_seconds > 0);
        assert!(registry.pending_preview("another-session").is_none());

        // A newer run owns the session now: the old preview must not come back.
        registry.register_run("s", 5, test_service());
        assert!(registry.pending_preview("s").is_none());
    }

    #[test]
    fn an_expired_preview_is_skipped_without_an_answer() {
        let registry = DecisionRegistry::default();
        registry.register_run("s", 1, test_service());
        let emitted = Arc::new(AtomicBool::new(false));
        let emitted_by_consent = emitted.clone();
        let consent = DesktopDecisionConsent::new(
            registry.clone(),
            Box::new(move |_| emitted_by_consent.store(true, Ordering::SeqCst)),
            "s".to_owned(),
            1,
            Arc::new(AtomicBool::new(true)),
        );
        let answer = consent.request(&preview(0));
        assert_eq!(answer, DecisionConsentAnswer::Skip);
        assert!(emitted.load(Ordering::SeqCst), "the preview is still shown");
        assert_eq!(registry.pending_count(), 0);
        assert!(registry.pending_preview("s").is_none());
    }

    fn test_service() -> Arc<DecisionService> {
        Arc::new(DecisionService::new(
            config(),
            DecisionClient::for_local_endpoint(DecisionProvider::TypeSafe, "http://127.0.0.1:1"),
            Arc::new(DecisionConsentForTest),
            "s",
            1,
        ))
    }

    #[test]
    fn a_consent_from_a_finished_run_cannot_be_answered() {
        let registry = DecisionRegistry::default();
        registry.register_run("s", 9, test_service());
        let (sender, _receiver) = mpsc::channel();
        registry.inner.pending.lock().unwrap().insert(
            "req-1".to_owned(),
            PendingConsent {
                session_id: "s".to_owned(),
                run_generation: 4,
                preview: preview(300),
                expires_at: Instant::now() + Duration::from_secs(300),
                sender,
            },
        );
        assert!(!registry.resolve("req-1", true));
    }

    #[test]
    fn a_run_service_is_registered_and_released() {
        let registry = DecisionRegistry::default();
        let service = test_service();
        registry.register_run("s", 3, service);
        assert_eq!(registry.runs_for_test("s"), Some(3));
        registry.release_run("s");
        assert_eq!(registry.runs_for_test("s"), None);
    }

    impl DecisionRegistry {
        /// The run generation that owns a session, for the tests.
        #[cfg(test)]
        pub fn runs_for_test(&self, session_id: &str) -> Option<u64> {
            self.inner
                .runs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(session_id)
                .map(|(generation, _)| *generation)
        }
    }

    struct DecisionConsentForTest;

    impl DecisionConsent for DecisionConsentForTest {
        fn request(&self, _preview: &DecisionPreview) -> DecisionConsentAnswer {
            DecisionConsentAnswer::Skip
        }
    }
}
