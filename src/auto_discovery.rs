//! Phase 81.33.b.real Stage 4 — auto-discovery broker handler for
//! admin RPC commands. Daemon receives `nexo/admin/google/<verb>`
//! from operators and forwards to `plugin.google.admin.<verb>` —
//! this module owns the in-process side of that contract.
//!
//! Other auto-discovery stages (pairing/http/metrics) are NOT
//! wired for google in v0.2.0:
//!   * Pairing: google has its own `--oauth-once` CLI subcommand;
//!     no broker-pairing surface needed.
//!   * HTTP: no long-lived HTTP routes mounted (the loopback
//!     listener lives inside `--oauth-once`'s lifetime, not as a
//!     daemon-routed endpoint).
//!   * Metrics: no Prometheus counters today (follow-up).
//!
//! All handler functions are `async` + return a `serde_json::Value`
//! shaped as `{ ok: true, result: <...> }` on success or
//! `{ ok: false, error: <msg> }` on failure — matches the
//! email plugin's contract so daemon-side
//! `PluginAdminRouter::forward_request` decodes uniformly.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::plugin::GooglePlugin;
use crate::runtime_handle;

async fn current_plugin() -> Option<Arc<GooglePlugin>> {
    runtime_handle::runtime_handle()
        .read()
        .await
        .as_ref()
        .map(Arc::clone)
}

/// `plugin.google.admin.*` dispatcher. Inspects `request.method` +
/// routes to the matching admin verb. Mirrors email's
/// `admin_handle` shape.
pub async fn admin_handle(request: &Value) -> Value {
    let method = request
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    let Some(plugin) = current_plugin().await else {
        return json!({
            "ok": false,
            "error": "google plugin not yet booted",
        });
    };

    match method {
        "nexo/admin/google/bot_info" => json!({
            "ok": true,
            "result": {
                "plugin": "google",
                "version": env!("CARGO_PKG_VERSION"),
                "configured_agents": plugin.agent_count(),
            },
        }),

        "nexo/admin/google/oauth_status" => {
            let Some(agent_id) = params.get("agent_id").and_then(|v| v.as_str()) else {
                return json!({
                    "ok": false,
                    "error": "missing required param `agent_id`",
                });
            };
            match plugin.admin_oauth_status(agent_id).await {
                Ok(snap) => json!({ "ok": true, "result": snap }),
                Err(e) => json!({ "ok": false, "error": format!("{e}") }),
            }
        }

        "nexo/admin/google/oauth_revoke" => {
            let Some(agent_id) = params.get("agent_id").and_then(|v| v.as_str()) else {
                return json!({
                    "ok": false,
                    "error": "missing required param `agent_id`",
                });
            };
            match plugin.admin_oauth_revoke(agent_id).await {
                Ok(v) => json!({ "ok": true, "result": v }),
                Err(e) => json!({ "ok": false, "error": format!("{e}") }),
            }
        }

        "nexo/admin/google/list_tokens" => match plugin.admin_list_tokens().await {
            Ok(v) => json!({ "ok": true, "result": v }),
            Err(e) => json!({ "ok": false, "error": format!("{e}") }),
        },

        other => json!({
            "ok": false,
            "error": format!("unknown admin method: {other}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{GooglePlugin, GooglePluginConfig};
    use serial_test::serial;

    async fn boot_with_one_agent() -> Arc<GooglePlugin> {
        let dir = tempfile::tempdir().unwrap();
        let p = Arc::new(GooglePlugin::new());
        p.on_configure(vec![GooglePluginConfig {
            agent_id: "agent_x".into(),
            workspace_dir: dir.path().to_string_lossy().into_owned(),
            client_id: "cid".into(),
            client_secret: "cs".into(),
            scopes: vec!["gmail.readonly".into()],
            token_file: "google_tokens.json".into(),
            redirect_port: 0,
        }])
        .await
        .unwrap();
        // intentionally leak dir handle until test exits
        std::mem::forget(dir);
        runtime_handle::set_runtime_handle(p.clone()).await;
        p
    }

    #[tokio::test]
    #[serial]
    async fn admin_bot_info_returns_metadata() {
        let _p = boot_with_one_agent().await;
        let r = admin_handle(&json!({
            "method": "nexo/admin/google/bot_info",
            "params": {},
        }))
        .await;
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["result"]["plugin"], json!("google"));
        assert_eq!(r["result"]["configured_agents"], json!(1));
    }

    #[tokio::test]
    #[serial]
    async fn admin_oauth_status_routes_to_plugin() {
        let _p = boot_with_one_agent().await;
        let r = admin_handle(&json!({
            "method": "nexo/admin/google/oauth_status",
            "params": { "agent_id": "agent_x" },
        }))
        .await;
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["result"]["authenticated"], json!(false));
    }

    #[tokio::test]
    #[serial]
    async fn admin_oauth_status_missing_agent_id_errors() {
        let _p = boot_with_one_agent().await;
        let r = admin_handle(&json!({
            "method": "nexo/admin/google/oauth_status",
            "params": {},
        }))
        .await;
        assert_eq!(r["ok"], json!(false));
    }

    #[tokio::test]
    #[serial]
    async fn admin_list_tokens_returns_per_agent_list() {
        let _p = boot_with_one_agent().await;
        let r = admin_handle(&json!({
            "method": "nexo/admin/google/list_tokens",
            "params": {},
        }))
        .await;
        assert_eq!(r["ok"], json!(true));
        let agents = r["result"]["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
    }

    #[tokio::test]
    #[serial]
    async fn admin_unknown_method_returns_err() {
        let _p = boot_with_one_agent().await;
        let r = admin_handle(&json!({
            "method": "nexo/admin/google/does_not_exist",
            "params": {},
        }))
        .await;
        assert_eq!(r["ok"], json!(false));
    }
}
