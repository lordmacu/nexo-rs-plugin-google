//! Subprocess-side env config loader.
//!
//! Daemon discovery walker spawns the binary with these env vars
//! (generic, no plugin-specific seeding code in the daemon —
//! agnostic per Wave 4 email precedent):
//!
//!   * `NEXO_BROKER_URL`            — broker URL.
//!   * `NEXO_BROKER_KIND`           — `nats` | `local` | `stdio_bridge`.
//!   * `NEXO_CONFIG_DIR`            — absolute path to the operator
//!                                     config dir (Phase 94 v0.2.1
//!                                     agnostic addition). Plugin
//!                                     reads
//!                                     `$NEXO_CONFIG_DIR/plugins/google-auth.yaml`
//!                                     here.
//!   * `NEXO_PLUGIN_GOOGLE_CONFIG_PATH` — explicit override path
//!                                     for the google-auth.yaml file
//!                                     (rarely needed).

use std::path::PathBuf;

use crate::plugin::GoogleAuthFile;

#[derive(Debug, Default)]
pub struct GoogleEnvConfig {
    pub broker_url: String,
    pub broker_kind: String,
    /// Initial accounts (loaded from yaml if a path was discoverable).
    pub initial: Option<GoogleAuthFile>,
    /// Path the YAML was read from (when `initial` is `Some`). Useful
    /// for diagnostics + future hot-reload watchers.
    pub config_path: Option<PathBuf>,
}

/// Resolve broker env + load the initial `google-auth.yaml` (best
/// effort). Errors only when the YAML exists but fails to parse.
pub fn google_config_from_env() -> anyhow::Result<GoogleEnvConfig> {
    let broker_url = std::env::var("NEXO_BROKER_URL").unwrap_or_default();
    let broker_kind = std::env::var("NEXO_BROKER_KIND").unwrap_or_else(|_| {
        if broker_url.starts_with("nats://") {
            "nats".to_string()
        } else {
            "local".to_string()
        }
    });

    let candidate = resolve_config_path();
    let (initial, config_path) = match candidate.as_ref() {
        Some(path) if path.exists() => {
            let bytes = std::fs::read(path)
                .map_err(|e| anyhow::anyhow!("reading google config at {}: {e}", path.display()))?;
            let parsed: GoogleAuthFile = serde_yaml::from_slice(&bytes).map_err(|e| {
                anyhow::anyhow!("parsing google-auth.yaml at {}: {e}", path.display())
            })?;
            (Some(parsed), Some(path.clone()))
        }
        _ => (None, candidate),
    };

    Ok(GoogleEnvConfig {
        broker_url,
        broker_kind,
        initial,
        config_path,
    })
}

/// Resolution order:
///   1. `NEXO_PLUGIN_GOOGLE_CONFIG_PATH` if set.
///   2. `$NEXO_CONFIG_DIR/plugins/google-auth.yaml`.
///   3. `./config/plugins/google-auth.yaml` (CWD-relative fallback
///      for local development).
fn resolve_config_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("NEXO_PLUGIN_GOOGLE_CONFIG_PATH") {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    if let Ok(cfg_dir) = std::env::var("NEXO_CONFIG_DIR") {
        if !cfg_dir.is_empty() {
            return Some(PathBuf::from(cfg_dir).join("plugins/google-auth.yaml"));
        }
    }
    Some(PathBuf::from("./config/plugins/google-auth.yaml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn clean_env() {
        for k in [
            "NEXO_BROKER_URL",
            "NEXO_BROKER_KIND",
            "NEXO_CONFIG_DIR",
            "NEXO_PLUGIN_GOOGLE_CONFIG_PATH",
        ] {
            unsafe {
                std::env::remove_var(k);
            }
        }
    }

    #[test]
    #[serial]
    fn defaults_when_env_empty_and_no_file() {
        clean_env();
        // Force a CWD-fallback miss by pointing NEXO_CONFIG_DIR at a
        // bogus path so the resolver yields Some but exists=false.
        unsafe {
            std::env::set_var("NEXO_CONFIG_DIR", "/nonexistent-nexo-test-dir");
        }
        let cfg = google_config_from_env().unwrap();
        assert!(cfg.initial.is_none());
        assert_eq!(cfg.broker_kind, "local");
        clean_env();
    }

    #[test]
    #[serial]
    fn nats_url_implies_nats_kind() {
        clean_env();
        unsafe {
            std::env::set_var("NEXO_BROKER_URL", "nats://localhost:4222");
            std::env::set_var("NEXO_CONFIG_DIR", "/nonexistent-nexo-test-dir");
        }
        let cfg = google_config_from_env().unwrap();
        assert_eq!(cfg.broker_kind, "nats");
        clean_env();
    }

    #[test]
    #[serial]
    fn reads_google_auth_yaml_when_config_dir_points_at_real_file() {
        clean_env();
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(
            plugins.join("google-auth.yaml"),
            "accounts:\n  - id: ana@gmail.com\n    agent_id: ana\n    client_id_path: /tmp/cid\n    client_secret_path: /tmp/cs\n    token_path: /tmp/tok.json\n    scopes:\n      - gmail.readonly\n",
        )
        .unwrap();

        unsafe {
            std::env::set_var("NEXO_CONFIG_DIR", dir.path().to_string_lossy().into_owned());
        }
        let cfg = google_config_from_env().unwrap();
        let parsed = cfg.initial.expect("should load yaml");
        assert_eq!(parsed.accounts.len(), 1);
        assert_eq!(parsed.accounts[0].id, "ana@gmail.com");
        assert_eq!(parsed.accounts[0].agent_id, "ana");
        clean_env();
    }

    #[test]
    #[serial]
    fn explicit_override_path_takes_precedence() {
        clean_env();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("override.yaml");
        std::fs::write(
            &path,
            "accounts:\n  - id: x@y\n    agent_id: x\n    client_id_path: /tmp/c\n    client_secret_path: /tmp/c\n    token_path: /tmp/t\n    scopes: []\n",
        )
        .unwrap();
        unsafe {
            std::env::set_var(
                "NEXO_PLUGIN_GOOGLE_CONFIG_PATH",
                path.to_string_lossy().into_owned(),
            );
            std::env::set_var("NEXO_CONFIG_DIR", "/nonexistent-nexo-test-dir");
        }
        let cfg = google_config_from_env().unwrap();
        let parsed = cfg.initial.expect("override path should win");
        assert_eq!(parsed.accounts[0].id, "x@y");
        clean_env();
    }
}
