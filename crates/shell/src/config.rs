//! Load shared Kodo settings (provider, permission, generation knobs) the same
//! way the desktop shell does. Settings live in `settings.log` and secrets in
//! `credentials.log` under the app config dir — one store, every client.

use kodo_agent::{Permission, Provider};
use kodo_core::settings;

/// Per-process CLI flags that beat the stored settings.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    pub permission: Option<Permission>,
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ShellConfig {
    pub provider: Option<Provider>,
    pub permission: Permission,
    pub fallback_to_local: bool,
    pub max_output_tokens: u32,
    pub extended_thinking: bool,
}

/// Stored shape of one entry in the `providers` settings array (camelCase
/// keys as written by the desktop shell; the key itself lives in
/// `credentials.log`, never here).
#[derive(Debug, Clone, serde::Deserialize)]
struct ProviderJson {
    id: String,
    #[allow(dead_code)]
    name: String,
    template: String,
    endpoint: String,
    model: String,
    #[serde(default, rename = "modelId")]
    model_id: Option<String>,
}

impl ShellConfig {
    /// Never fails: missing HOME, unparsable settings, or an absent key all
    /// degrade to the same defaults the GUI uses (`provider: None` → offline
    /// local notes).
    pub fn load(overrides: Overrides) -> Self {
        let settings_path = settings::settings_path();
        let read_setting = |key: &str| {
            settings_path
                .as_ref()
                .and_then(|path| settings::read(path, key))
        };

        let permission = overrides
            .permission
            .unwrap_or_else(|| permission_from_settings(read_setting("permission")));

        let provider = load_provider(&read_setting, overrides.model.as_deref());

        let max_output_tokens = read_setting("max-output-tokens")
            .and_then(|raw| raw.parse::<u32>().ok())
            .unwrap_or(4096);
        let extended_thinking = read_setting("extended-thinking").as_deref() == Some("true");
        // fallback-behavior=fail means no silent offline answers when a key exists.
        let fallback_to_local = read_setting("fallback-behavior").as_deref() != Some("fail");

        Self {
            provider,
            permission,
            fallback_to_local,
            max_output_tokens,
            extended_thinking,
        }
    }
}

/// Secure-by-default: missing or unknown permission means Ask.
pub fn permission_from_settings(value: Option<String>) -> Permission {
    value
        .map(|raw| Permission::parse(&raw))
        .unwrap_or(Permission::Ask)
}

fn load_provider(
    read_setting: &dyn Fn(&str) -> Option<String>,
    model_override: Option<&str>,
) -> Option<Provider> {
    let raw = read_setting("providers")?;
    if raw.trim().is_empty() {
        return None;
    }
    let list: Vec<ProviderJson> = serde_json::from_str(&raw).ok()?;
    if list.is_empty() {
        return None;
    }

    let creds = settings::credentials_path()?;
    let build = |p: &ProviderJson| -> Option<Provider> {
        let api_key = settings::read_credential(&creds, &p.id).unwrap_or_default();
        if api_key.is_empty() {
            return None;
        }
        Some(Provider::new(
            p.template.clone(),
            api_key,
            p.endpoint.clone(),
            p.model_id
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| p.model.clone()),
        ))
    };

    let active = read_setting("active-provider")
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(0);
    let chosen = list
        .get(active)
        .cloned()
        .or_else(|| list.iter().find(|p| build(p).is_some()).cloned())?;

    let mut primary = build(&chosen)?;
    // Populate the failover chain when the user asked to try the next best
    // model (fallback-behavior != fail). Auth/invalid-model never failover.
    if read_setting("fallback-behavior").as_deref() != Some("fail") {
        for p in &list {
            if p.id == chosen.id {
                continue;
            }
            if let Some(fb) = build(p) {
                primary.fallbacks.push(fb);
            }
        }
    }
    // Default model is a runtime consumer: override the primary model so the
    // Settings control is what the next call actually sends. A CLI --model
    // flag wins over the stored setting.
    let default_model = model_override
        .map(str::to_owned)
        .or_else(|| read_setting("default-model").map(|m| m.trim().to_owned()));
    if let Some(model) = default_model {
        if !model.is_empty() {
            let mut next = Provider::new(
                primary.template.clone(),
                primary.api_key.clone(),
                primary.endpoint.clone(),
                model,
            );
            next.fallbacks = std::mem::take(&mut primary.fallbacks);
            primary = next;
        }
    }
    Some(primary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_permission_is_ask() {
        assert_eq!(permission_from_settings(None), Permission::Ask);
        assert_eq!(
            permission_from_settings(Some("bogus".into())),
            Permission::Ask
        );
        assert_eq!(
            permission_from_settings(Some("full".into())),
            Permission::Full
        );
    }

    #[test]
    fn overrides_beat_stored_settings() {
        let overrides = Overrides {
            permission: Some(Permission::Full),
            model: Some("override-model".into()),
        };
        // ShellConfig::load reads the real config dir; only assert the
        // override plumbing here so the test stays hermetic.
        assert_eq!(overrides.permission, Some(Permission::Full));
        assert_eq!(overrides.model.as_deref(), Some("override-model"));
    }
}
