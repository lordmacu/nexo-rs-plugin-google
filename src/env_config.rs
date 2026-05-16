//! Subprocess-side env config loader.
//!
//! Daemon discovery walker spawns the binary with these env vars
//! (set generically by the daemon's plugin spawner; no
//! google-specific seeding code in the daemon — agnostic per Wave 4
//! email precedent):
//!
//!   * `NEXO_BROKER_URL`           — broker URL (e.g. `nats://…`).
//!   * `NEXO_BROKER_KIND`          — `nats` | `local` | `stdio_bridge`.
//!   * `NEXO_PLUGIN_GOOGLE_CONFIG_PATH` — optional path to a fallback
//!                                   YAML used when the daemon's
//!                                   `plugin.configure` JSON-RPC
//!                                   has not been received yet
//!                                   (parity with email plugin).
//!
//! The subprocess primarily receives its per-agent config list via
//! `plugin.configure`; this fallback path keeps `cargo install
//! nexo-plugin-google` usable from a one-shot CLI for diagnostics.

use std::path::PathBuf;

use crate::plugin::GooglePluginConfig;

/// Parsed env contract.
#[derive(Debug, Default)]
pub struct GoogleEnvConfig {
    /// `NEXO_BROKER_URL`. Empty string when unset — the binary is
    /// being used outside the daemon (e.g. `--oauth-once`).
    pub broker_url: String,
    /// `NEXO_BROKER_KIND`. Defaults to `nats` when broker_url starts
    /// with `nats://`, else `local`.
    pub broker_kind: String,
    /// Initial set of agent configs (if `NEXO_PLUGIN_GOOGLE_CONFIG_PATH`
    /// pointed at a YAML and parse succeeded). `plugin.configure`
    /// overrides this set later.
    pub initial_configs: Vec<GooglePluginConfig>,
}

/// Read env vars + optional fallback YAML. Errors only on YAML
/// parse failure when the path is set + non-empty.
pub fn google_config_from_env() -> anyhow::Result<GoogleEnvConfig> {
    let broker_url = std::env::var("NEXO_BROKER_URL").unwrap_or_default();
    let broker_kind = std::env::var("NEXO_BROKER_KIND").unwrap_or_else(|_| {
        if broker_url.starts_with("nats://") {
            "nats".to_string()
        } else {
            "local".to_string()
        }
    });

    let initial_configs = match std::env::var("NEXO_PLUGIN_GOOGLE_CONFIG_PATH") {
        Ok(path) if !path.is_empty() => {
            let path = PathBuf::from(path);
            let bytes = std::fs::read(&path).map_err(|e| {
                anyhow::anyhow!("reading NEXO_PLUGIN_GOOGLE_CONFIG_PATH={}: {e}", path.display())
            })?;
            let parsed: Vec<GooglePluginConfig> = serde_yaml::from_slice(&bytes).map_err(|e| {
                anyhow::anyhow!("parsing NEXO_PLUGIN_GOOGLE_CONFIG_PATH yaml: {e}")
            })?;
            parsed
        }
        _ => Vec::new(),
    };

    Ok(GoogleEnvConfig {
        broker_url,
        broker_kind,
        initial_configs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn defaults_when_env_empty() {
        unsafe {
            std::env::remove_var("NEXO_BROKER_URL");
            std::env::remove_var("NEXO_BROKER_KIND");
            std::env::remove_var("NEXO_PLUGIN_GOOGLE_CONFIG_PATH");
        }
        let cfg = google_config_from_env().unwrap();
        assert_eq!(cfg.broker_url, "");
        assert_eq!(cfg.broker_kind, "local");
        assert!(cfg.initial_configs.is_empty());
    }

    #[test]
    #[serial]
    fn nats_url_implies_nats_kind() {
        unsafe {
            std::env::set_var("NEXO_BROKER_URL", "nats://localhost:4222");
            std::env::remove_var("NEXO_BROKER_KIND");
            std::env::remove_var("NEXO_PLUGIN_GOOGLE_CONFIG_PATH");
        }
        let cfg = google_config_from_env().unwrap();
        assert_eq!(cfg.broker_kind, "nats");
        unsafe {
            std::env::remove_var("NEXO_BROKER_URL");
        }
    }

    #[test]
    #[serial]
    fn yaml_fallback_path_loads_configs() {
        let dir = tempfile::tempdir().unwrap();
        let yaml_path = dir.path().join("g.yaml");
        std::fs::write(
            &yaml_path,
            "- agent_id: agent_a\n  workspace_dir: /tmp/a\n  client_id: cid\n  \
             client_secret: cs\n  scopes: [\"gmail.readonly\"]\n",
        )
        .unwrap();
        unsafe {
            std::env::set_var(
                "NEXO_PLUGIN_GOOGLE_CONFIG_PATH",
                yaml_path.to_string_lossy().into_owned(),
            );
        }
        let cfg = google_config_from_env().unwrap();
        assert_eq!(cfg.initial_configs.len(), 1);
        assert_eq!(cfg.initial_configs[0].agent_id, "agent_a");
        unsafe {
            std::env::remove_var("NEXO_PLUGIN_GOOGLE_CONFIG_PATH");
        }
    }
}
