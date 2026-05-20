//! Phase 81.33.b.real Stage 4 + Phase 94 FU#4 — auto-discovery
//! broker handlers for admin RPC + HTTP routes.
//!
//! Other auto-discovery stages NOT yet wired for google:
//!   * Pairing: google has its own `--oauth-once` CLI subcommand;
//!     no broker-pairing surface needed.
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
    let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
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
                "configured_accounts": plugin.account_count(),
            },
        }),

        "nexo/admin/google/oauth_status" => {
            let Some(agent_id) = params.get("agent_id").and_then(|v| v.as_str()) else {
                return json!({
                    "ok": false,
                    "error": "missing required param `agent_id`",
                });
            };
            let account = params.get("account").and_then(|v| v.as_str());
            match plugin.admin_oauth_status(agent_id, account).await {
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
            let account = params.get("account").and_then(|v| v.as_str());
            match plugin.admin_oauth_revoke(agent_id, account).await {
                Ok(v) => json!({ "ok": true, "result": v }),
                Err(e) => json!({ "ok": false, "error": format!("{e}") }),
            }
        }

        "nexo/admin/google/list_tokens" => match plugin.admin_list_tokens().await {
            Ok(v) => json!({ "ok": true, "result": v }),
            Err(e) => json!({ "ok": false, "error": format!("{e}") }),
        },

        // Phase 99 — admin UI (Mode A dynamic). The daemon forwards
        // `describe` here; we build the live OAuth-status screen.
        "nexo/admin/google/admin_ui/describe" => {
            let status = match plugin.admin_list_tokens().await {
                Ok(v) => v,
                Err(e) => json!({ "error": format!("{e}") }),
            };
            json!({ "ok": true, "result": oauth_screen_descriptor(status) })
        }

        // OAuth screen is read-only (status + actions); nothing to
        // persist. Return the ConfigSetResponse envelope so the UI
        // save path is a clean no-op.
        "nexo/admin/google/admin_ui/config_set" => json!({
            "ok": true,
            "result": { "ok": true, "errors": [] },
        }),

        other => json!({
            "ok": false,
            "error": format!("unknown admin method: {other}"),
        }),
    }
}

/// Phase 99 — build the live OAuth-status `ScreenDescriptor`
/// (nexo-tool-meta wire shape) the admin UI renders. A read-only
/// status field + a `revoke` action + a refresh widget, all reusing
/// the plugin's existing `list_tokens` / `oauth_revoke` admin verbs
/// (zero new dispatch surface beyond this descriptor).
fn oauth_screen_descriptor(status: Value) -> Value {
    json!({
        "plugin": "google",
        "screen_id": "oauth",
        "title": "Google OAuth",
        "fields": [
            {
                "key": "status",
                "field_type": "json",
                "label": "Token status",
                "required": false,
                "value": status,
            }
        ],
        "actions": [
            {
                "id": "revoke",
                "label": "Revoke account",
                "method": "nexo/admin/google/oauth_revoke",
                "prompt_fields": [
                    { "key": "agent_id", "field_type": "text", "label": "Agent id", "required": true },
                    { "key": "account", "field_type": "text", "label": "Account (optional)", "required": false }
                ],
                "on_success": "refresh",
            }
        ],
        "refresh": { "method": "nexo/admin/google/list_tokens" },
    })
}

// ── Phase 94 FU#4 — HTTP route handler ─────────────────────────

/// `plugin.google.http.request` broker handler. The daemon's
/// `PluginHttpRouter` (Phase 81.33.b.real Stage 2) maps requests
/// arriving at `/google/<path>` onto this RPC; the plugin's
/// internal router answers + returns the canonical
/// `{status, headers, body_base64}` envelope. Mirrors email's
/// shape.
pub async fn http_request(request: &Value) -> Value {
    let path = request.get("path").and_then(|v| v.as_str()).unwrap_or("/");
    let method = request
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET");
    match (method, path) {
        ("GET", "/google/status") => {
            let body = render_status_snapshot().await;
            respond(
                200,
                "application/json; charset=utf-8",
                body.to_string().as_bytes(),
            )
        }
        ("GET", "/google/health") => respond(
            200,
            "application/json; charset=utf-8",
            br#"{"status":"ok"}"#,
        ),
        _ => respond(
            404,
            "application/json; charset=utf-8",
            br#"{"error":"not found"}"#,
        ),
    }
}

async fn render_status_snapshot() -> Value {
    let Some(plugin) = current_plugin().await else {
        return json!({
            "status": "booting",
            "plugin": "google",
            "version": env!("CARGO_PKG_VERSION"),
        });
    };
    let agent_count = plugin.agent_count();
    let account_count = plugin.account_count();
    // Best-effort: include per-account oauth snapshot. Heavy if
    // many accounts, but operators typically have N <= 20.
    let listing = plugin
        .admin_list_tokens()
        .await
        .unwrap_or_else(|_| json!({}));
    json!({
        "status": "ok",
        "plugin": "google",
        "version": env!("CARGO_PKG_VERSION"),
        "agents": agent_count,
        "accounts": account_count,
        "listing": listing,
    })
}

fn respond(status: u16, content_type: &str, body: &[u8]) -> Value {
    use base64::Engine;
    json!({
        "status": status,
        "headers": [["Content-Type", content_type]],
        "body_base64": base64::engine::general_purpose::STANDARD.encode(body),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{GoogleAccount, GoogleAuthFile, GooglePlugin};
    use serial_test::serial;

    #[test]
    fn oauth_descriptor_has_status_field_revoke_action_and_refresh() {
        let d = oauth_screen_descriptor(json!({"agents": []}));
        assert_eq!(d["screen_id"], "oauth");
        assert_eq!(d["fields"][0]["key"], "status");
        assert_eq!(d["fields"][0]["field_type"], "json");
        // status value is the passed-in token snapshot.
        assert_eq!(d["fields"][0]["value"], json!({"agents": []}));
        assert_eq!(d["actions"][0]["id"], "revoke");
        assert_eq!(d["actions"][0]["method"], "nexo/admin/google/oauth_revoke");
        assert_eq!(d["actions"][0]["prompt_fields"][0]["key"], "agent_id");
        assert_eq!(d["refresh"]["method"], "nexo/admin/google/list_tokens");
    }

    async fn boot_with_one_agent() -> Arc<GooglePlugin> {
        let dir = tempfile::tempdir().unwrap();
        let cid_path = dir.path().join("cid.txt");
        let cs_path = dir.path().join("cs.txt");
        std::fs::write(&cid_path, "test-cid").unwrap();
        std::fs::write(&cs_path, "test-cs").unwrap();
        let p = Arc::new(GooglePlugin::new());
        p.on_configure(GoogleAuthFile {
            accounts: vec![GoogleAccount {
                id: "agent_x@gmail.com".into(),
                agent_id: "agent_x".into(),
                client_id_path: cid_path,
                client_secret_path: cs_path,
                token_path: dir.path().join("tok.json"),
                scopes: vec!["gmail.readonly".into()],
                redirect_port: 0,
            }],
        })
        .await
        .unwrap();
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
