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
use std::time::Duration;

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
/// The provider names which key this is: TypeSafe and OpenRouter keys live
/// under their own ids, so one service's key is never sent to the other.
#[tauri::command]
pub fn decision_set_credential(provider: String, secret: String) -> Result<u64, String> {
    let provider = DecisionProvider::parse(&provider)
        .ok_or_else(|| format!("unknown decision provider `{provider}`"))?;
    save_decision_credential(provider, &secret)
}

/// Remove one provider's decision credential. The other provider's key and
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

/// One consent waiting for an answer.
struct PendingConsent {
    session_id: String,
    run_generation: u64,
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
        // to release this thread promptly.
        let answer = loop {
            if !self.alive.load(Ordering::SeqCst) {
                break DecisionConsentAnswer::Cancelled;
            }
            match receiver.recv_timeout(Duration::from_millis(100)) {
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
    use kodo_agent::decision::{DecisionClient, DecisionMode, DecisionSettings};
    use kodo_shell::decision::DecisionConfig;

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
                sender,
            },
        );
        registry.clear_session("s");
        assert_eq!(registry.pending_count(), 0);
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
