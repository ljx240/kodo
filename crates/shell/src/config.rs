//! Load shared Kodo settings (provider, permission, generation knobs) the same
//! way the desktop shell does. Settings live in `settings.log` and secrets in
//! `credentials.log` under the app config dir — one store, every client.

use kodo_agent::mcp::{merge_secrets, McpSecrets, McpServerConfig};
use kodo_agent::{AgentMode, Permission, Provider};
use kodo_core::settings;

/// Per-process CLI flags that beat the stored settings.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    pub permission: Option<Permission>,
    pub mode: Option<AgentMode>,
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ShellConfig {
    pub provider: Option<Provider>,
    pub permission: Permission,
    pub mode: AgentMode,
    pub fallback_to_local: bool,
    pub max_output_tokens: u32,
    pub extended_thinking: bool,
    pub local_trace: bool,
    /// Enabled and disabled servers alike; `run()` connects only the enabled
    /// ones. Secret values are merged in from `credentials.log`.
    pub mcp_servers: Vec<McpServerConfig>,
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

        Self::from_sources(
            overrides,
            &read_setting,
            settings::credentials_path().as_deref(),
        )
    }

    fn from_sources(
        overrides: Overrides,
        read_setting: &dyn Fn(&str) -> Option<String>,
        creds: Option<&std::path::Path>,
    ) -> Self {
        let permission = overrides
            .permission
            .unwrap_or_else(|| permission_from_settings(read_setting("permission")));

        let mode = overrides
            .mode
            .unwrap_or_else(|| mode_from_settings(read_setting("agent-mode")));

        let provider = load_provider(read_setting, overrides.model.as_deref(), creds);

        let max_output_tokens = read_setting("max-output-tokens")
            .and_then(|raw| raw.parse::<u32>().ok())
            .unwrap_or(4096);
        let extended_thinking = read_setting("extended-thinking").as_deref() == Some("true");
        // fallback-behavior=fail means no silent offline answers when a key exists.
        let fallback_to_local = read_setting("fallback-behavior").as_deref() != Some("fail");

        let mcp_servers = load_mcp_servers(read_setting, creds);

        Self {
            provider,
            permission,
            mode,
            fallback_to_local,
            max_output_tokens,
            extended_thinking,
            local_trace: read_setting("local-detailed-traces").as_deref() == Some("true"),
            mcp_servers,
        }
    }
}

/// The `mcp-servers` settings array with each server's secret values merged
/// in from `credentials.log`. Missing settings, missing HOME or an unparsable
/// blob all degrade to "no server" or "no secret" rather than failing.
fn load_mcp_servers(
    read_setting: &dyn Fn(&str) -> Option<String>,
    creds: Option<&std::path::Path>,
) -> Vec<McpServerConfig> {
    let raw = match read_setting("mcp-servers") {
        Some(raw) if !raw.trim().is_empty() => raw,
        _ => return Vec::new(),
    };
    let list: Vec<McpServerConfig> = match serde_json::from_str(&raw) {
        Ok(list) => list,
        Err(_) => return Vec::new(),
    };
    list.into_iter()
        .map(|config| {
            let blob = creds
                .and_then(|path| settings::read_secret(path, "mcp", &config.id))
                .and_then(|raw| serde_json::from_str::<McpSecrets>(&raw).ok())
                .unwrap_or_default();
            merge_secrets(&config, &blob)
        })
        .collect()
}

/// Secure-by-default: missing or unknown permission means Ask.
pub fn permission_from_settings(value: Option<String>) -> Permission {
    value
        .map(|raw| Permission::parse(&raw))
        .unwrap_or(Permission::Ask)
}

/// Default mode is Code — Kodo stays a coding agent unless the user switches.
pub fn mode_from_settings(value: Option<String>) -> AgentMode {
    value
        .map(|raw| AgentMode::parse(&raw))
        .unwrap_or(AgentMode::Code)
}

/// Empty restores provider presets; a custom window must fit a positive u32.
pub fn parse_context_window(raw: &str) -> Result<Option<u32>, &'static str> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    raw.parse::<u32>()
        .ok()
        .filter(|window| *window > 0)
        .map(Some)
        .ok_or("context window must be an integer between 1 and 4294967295 tokens")
}

fn load_provider(
    read_setting: &dyn Fn(&str) -> Option<String>,
    model_override: Option<&str>,
    creds: Option<&std::path::Path>,
) -> Option<Provider> {
    let raw = read_setting("providers")?;
    if raw.trim().is_empty() {
        return None;
    }
    let list: Vec<ProviderJson> = serde_json::from_str(&raw).ok()?;
    if list.is_empty() {
        return None;
    }

    let creds = creds?;
    let context_window = read_setting("context-window-tokens")
        .and_then(|raw| parse_context_window(&raw).ok().flatten());
    let build = |p: &ProviderJson| -> Option<Provider> {
        let api_key = settings::read_credential(creds, &p.id).unwrap_or_default();
        if api_key.is_empty() {
            return None;
        }
        let mut provider = Provider::new(
            p.template.clone(),
            api_key,
            p.endpoint.clone(),
            p.model_id
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| p.model.clone()),
        );
        if let Some(window) = context_window {
            let mut caps = provider.capabilities();
            caps.context_window_tokens = window;
            provider.capabilities_override = Some(caps);
        }
        Some(provider)
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
            next.capabilities_override = primary.capabilities_override;
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
    fn context_window_accepts_auto_and_positive_integers_only() {
        assert_eq!(parse_context_window(""), Ok(None));
        assert_eq!(parse_context_window("   "), Ok(None));
        assert_eq!(parse_context_window(" 64000 "), Ok(Some(64_000)));
        assert_eq!(parse_context_window("4294967295"), Ok(Some(u32::MAX)));
        for raw in ["0", "-1", "1.5", "1e5", "unknown", "4294967296"] {
            assert!(parse_context_window(raw).is_err(), "{raw}");
        }
    }

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
    fn missing_mode_is_code() {
        assert_eq!(mode_from_settings(None), AgentMode::Code);
        assert_eq!(mode_from_settings(Some("bogus".into())), AgentMode::Code);
        assert_eq!(mode_from_settings(Some("work".into())), AgentMode::Work);
    }

    #[test]
    fn overrides_beat_stored_settings() {
        let overrides = Overrides {
            permission: Some(Permission::Full),
            mode: Some(AgentMode::Work),
            model: Some("override-model".into()),
        };
        // ShellConfig::load reads the real config dir; only assert the
        // override plumbing here so the test stays hermetic.
        assert_eq!(overrides.permission, Some(Permission::Full));
        assert_eq!(overrides.model.as_deref(), Some("override-model"));
    }

    #[test]
    fn stored_settings_reach_runtime() {
        let dir =
            std::env::temp_dir().join(format!("kodo-settings-consumer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let creds = dir.join("credentials.log");
        settings::write_credential(&creds, "a", "synthetic-a").unwrap();
        settings::write_credential(&creds, "b", "synthetic-b").unwrap();
        let values = std::collections::BTreeMap::from([
            (
                "providers",
                r#"[{"id":"a","name":"A","template":"openai","endpoint":"","model":"original"},{"id":"b","name":"B","template":"openai","endpoint":"","model":"backup"}]"#,
            ),
            ("permission", "auto"),
            ("agent-mode", "work"),
            ("default-model", "configured"),
            ("fallback-behavior", "fail"),
            ("max-output-tokens", "8192"),
            ("context-window-tokens", "64000"),
            ("extended-thinking", "true"),
            ("local-detailed-traces", "true"),
        ]);
        let read = |k: &str| values.get(k).map(|v| v.to_string());
        let config = ShellConfig::from_sources(Overrides::default(), &read, Some(&creds));
        assert_eq!(config.permission, Permission::Auto);
        assert_eq!(config.mode, AgentMode::Work);
        assert_eq!(config.max_output_tokens, 8192);
        assert!(config.extended_thinking && config.local_trace);
        assert!(!config.fallback_to_local);
        let provider = config.provider.unwrap();
        assert_eq!(provider.resolved_model_id(), "configured");
        assert_eq!(provider.context_window(), 64_000);
        assert!(provider.capabilities().native_tools);
        assert!(provider.fallbacks.is_empty());
        let next = |k: &str| {
            if k == "fallback-behavior" {
                Some("next".into())
            } else {
                read(k)
            }
        };
        let config = ShellConfig::from_sources(
            Overrides {
                permission: Some(Permission::Ask),
                mode: Some(AgentMode::Code),
                model: Some("override".into()),
            },
            &next,
            Some(&creds),
        );
        assert_eq!(config.permission, Permission::Ask);
        assert_eq!(config.mode, AgentMode::Code);
        let provider = config.provider.unwrap();
        assert_eq!(provider.resolved_model_id(), "override");
        assert_eq!(provider.context_window(), 64_000);
        assert_eq!(provider.fallbacks.len(), 1);
        assert_eq!(provider.fallbacks[0].context_window(), 64_000);
        for raw in ["", "0", "invalid", "4294967296"] {
            let reset = |key: &str| {
                if key == "context-window-tokens" {
                    Some(raw.to_owned())
                } else {
                    next(key)
                }
            };
            let provider = ShellConfig::from_sources(Overrides::default(), &reset, Some(&creds))
                .provider
                .unwrap();
            assert_eq!(provider.context_window(), 128_000, "{raw}");
            assert_eq!(provider.fallbacks[0].context_window(), 128_000, "{raw}");
        }
        let defaults = ShellConfig::from_sources(Overrides::default(), &|_| None, None);
        assert!(!defaults.local_trace);
        assert_eq!(defaults.max_output_tokens, 4096);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn mcp_servers_merge_secret_values_from_the_credentials_log() {
        let dir = std::env::temp_dir().join(format!("kodo-shell-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let creds = dir.join("credentials.log");
        settings::write_secret(&creds, "mcp", "fs", r#"{"env":{"TOKEN":"t-99"}}"#).expect("blob");

        let raw = r#"[{"id":"fs","name":"FS","enabled":true,"transport":"stdio","command":"npx","args":[],"env":{"TOKEN":""},"url":"","headers":{}}]"#;
        let read = |key: &str| (key == "mcp-servers").then(|| raw.to_owned());
        let list = load_mcp_servers(&read, Some(&creds));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].env.get("TOKEN").map(String::as_str), Some("t-99"));

        // A server whose blob is missing keeps empty values; no key or junk
        // settings degrade to "no servers", matching ShellConfig's contract.
        let orphan =
            r#"[{"id":"gone","name":"Gone","transport":"http","url":"http://127.0.0.1:1/mcp"}]"#;
        let read_orphan = |key: &str| (key == "mcp-servers").then(|| orphan.to_owned());
        let list = load_mcp_servers(&read_orphan, Some(&creds));
        assert_eq!(list.len(), 1);
        assert!(list[0].headers.is_empty() && list[0].env.is_empty());

        assert!(load_mcp_servers(&|_| None, Some(&creds)).is_empty());
        let bad = |key: &str| (key == "mcp-servers").then(|| "{".to_owned());
        assert!(load_mcp_servers(&bad, Some(&creds)).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
