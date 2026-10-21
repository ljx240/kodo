//! The desktop's decision-assistance bridge.
//!
//! The settings screen is where the user enables the feature and is told
//! that Jev requests leave the machine. Per the user's decision on
//! 2026-10-21, the desktop does not ask again for every request: a live run
//! lets it go out, and a stopped run never does. The CLI keeps its own
//! terminal preview.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use kodo_agent::decision::DecisionProvider;
use kodo_shell::decision::{DecisionConsent, DecisionConsentAnswer, DecisionPreview};
use kodo_shell::{
    clear_decision_credential, decision_settings_view, save_decision_credential,
    save_decision_settings, DecisionSettingsView,
};

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

/// The desktop's consent host.
///
/// A request is allowed only while its run is alive. The shell still calls
/// this for every request, so stopping a run cancels a request that was
/// about to leave instead of letting it finish unattended.
pub struct DesktopDecisionConsent {
    alive: Arc<AtomicBool>,
}

impl DesktopDecisionConsent {
    pub fn new(alive: Arc<AtomicBool>) -> Self {
        Self { alive }
    }
}

impl DecisionConsent for DesktopDecisionConsent {
    fn request(&self, _preview: &DecisionPreview) -> DecisionConsentAnswer {
        if self.alive.load(Ordering::SeqCst) {
            DecisionConsentAnswer::Allow
        } else {
            DecisionConsentAnswer::Cancelled
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kodo_agent::decision::{DecisionMode, DecisionScenario};

    fn preview() -> DecisionPreview {
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
            expires_in_seconds: 300,
        }
    }

    fn consent(alive: bool) -> DesktopDecisionConsent {
        DesktopDecisionConsent::new(Arc::new(AtomicBool::new(alive)))
    }

    #[test]
    fn a_live_run_sends_without_a_confirmation() {
        assert_eq!(
            consent(true).request(&preview()),
            DecisionConsentAnswer::Allow
        );
    }

    #[test]
    fn a_stopped_run_cancels_the_request() {
        assert_eq!(
            consent(false).request(&preview()),
            DecisionConsentAnswer::Cancelled
        );
    }
}
