//! Subprocess plugin runtime.
//!
//! Holds one `Arc<GoogleAuthClient>` per agent in a DashMap, keyed
//! by `agent_id`. The daemon's `[plugin.configure]` JSON-RPC delivers
//! the per-agent config list at boot; the four `google_*` tools
//! dispatch through `invoke_outbound_tool` which reads `agent_id`
//! from the JSON-RPC `params`.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::client::{GoogleAuthClient, GoogleAuthConfig};

/// Opaque per-agent config delivered by `plugin.configure`. Mirrors
/// the YAML shape declared in the manifest's `[plugin.config_schema]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GooglePluginConfig {
    /// Agent identifier the OAuth client belongs to. Used as DashMap key.
    pub agent_id: String,
    /// Workspace directory. Token file resolves here when relative.
    pub workspace_dir: String,
    pub client_id: String,
    pub client_secret: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default = "default_token_file")]
    pub token_file: String,
    #[serde(default = "default_redirect_port")]
    pub redirect_port: u16,
}

fn default_token_file() -> String {
    "google_tokens.json".to_string()
}
fn default_redirect_port() -> u16 {
    8765
}

/// Process-wide google plugin state. Built once at boot; refreshed
/// incrementally by every `plugin.configure` call.
///
/// Full-replace semantics on `on_configure`: a fresh list replaces
/// the entire DashMap so removing an agent's `google_auth:` block
/// drops its in-memory client. Mirrors email's tenant-set behaviour.
pub struct GooglePlugin {
    clients: DashMap<String, Arc<GoogleAuthClient>>,
}

impl Default for GooglePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl GooglePlugin {
    pub fn new() -> Self {
        Self {
            clients: DashMap::new(),
        }
    }

    /// Number of agents currently configured. Used by observability +
    /// tests.
    pub fn agent_count(&self) -> usize {
        self.clients.len()
    }

    /// Replace the agent → client map with a fresh set built from
    /// the supplied configs. `load_from_disk` runs in parallel so
    /// existing tokens come back into memory before the first tool
    /// call.
    pub async fn on_configure(&self, configs: Vec<GooglePluginConfig>) -> Result<()> {
        // Build the new map first; only swap once every client is
        // constructed. Failed disk loads warn-only — the agent can
        // still re-auth via `google_auth_start`.
        let next: DashMap<String, Arc<GoogleAuthClient>> = DashMap::new();
        for cfg in configs {
            let agent_id = cfg.agent_id.clone();
            let workspace = PathBuf::from(&cfg.workspace_dir);
            let client_cfg = GoogleAuthConfig {
                client_id: cfg.client_id,
                client_secret: cfg.client_secret,
                scopes: cfg.scopes,
                token_file: cfg.token_file,
                redirect_port: cfg.redirect_port,
            };
            let client = GoogleAuthClient::new(client_cfg, &workspace);
            if let Err(e) = client.load_from_disk().await {
                tracing::warn!(
                    target = "nexo_plugin_google",
                    agent = %agent_id,
                    error = %e,
                    "tokens load failed; agent will need to re-consent"
                );
            }
            next.insert(agent_id, client);
        }

        self.clients.clear();
        for (k, v) in next.into_iter() {
            self.clients.insert(k, v);
        }

        tracing::info!(
            target = "nexo_plugin_google",
            agents = self.clients.len(),
            "google plugin reconfigured"
        );
        Ok(())
    }

    /// Resolve the per-agent `Arc<GoogleAuthClient>`.
    pub fn client_for(&self, agent_id: &str) -> Result<Arc<GoogleAuthClient>> {
        self.clients
            .get(agent_id)
            .map(|r| Arc::clone(r.value()))
            .ok_or_else(|| {
                anyhow!(
                    "agent `{agent_id}` is not configured for google_auth (no plugin.configure \
                     entry — daemon did not declare it OR not_configured)"
                )
            })
    }

    /// Dispatch a tool call routed via the daemon's outbound RPC
    /// (`outbound_tool.invoke` OR `tool.invoke` — `rpc_method` is
    /// caller-provided). `agent_id` is required + comes from the
    /// JSON-RPC `params`.
    pub async fn invoke_outbound_tool(
        &self,
        tool_name: &str,
        args: Value,
        agent_id: &str,
    ) -> Result<Value> {
        let client = self.client_for(agent_id)?;
        match tool_name {
            "google_auth_start" => self.tool_auth_start(&client).await,
            "google_auth_status" => Ok(client.snapshot().await),
            "google_call" => self.tool_call(&client, &args).await,
            "google_auth_revoke" => self.tool_revoke(&client).await,
            other => Err(anyhow!("unknown tool `{other}`")),
        }
    }

    async fn tool_auth_start(&self, client: &Arc<GoogleAuthClient>) -> Result<Value> {
        let (url, _join) = client.start_auth_flow().await?;
        let redirect_port = client.config().redirect_port;
        Ok(json!({
            "ok": true,
            "url": url,
            "instructions": "Open this URL in a browser you're logged into \
                your Google account with, approve the scopes, then call \
                google_auth_status to confirm.",
            "redirect_uri": format!("http://127.0.0.1:{redirect_port}/callback"),
        }))
    }

    async fn tool_call(&self, client: &Arc<GoogleAuthClient>, args: &Value) -> Result<Value> {
        let method = args["method"]
            .as_str()
            .ok_or_else(|| anyhow!("google_call requires `method`"))?;
        let url = args["url"]
            .as_str()
            .ok_or_else(|| anyhow!("google_call requires `url`"))?;
        if !url.starts_with("https://") || !url.contains("googleapis.com") {
            return Err(anyhow!(
                "google_call only accepts https://*.googleapis.com URLs — got `{url}`"
            ));
        }
        let body = args
            .get("body")
            .filter(|b| !b.is_null())
            .cloned();
        let resp = client.authorized_call(method, url, body).await?;
        Ok(json!({ "ok": true, "response": resp }))
    }

    async fn tool_revoke(&self, client: &Arc<GoogleAuthClient>) -> Result<Value> {
        client.revoke().await?;
        Ok(json!({ "ok": true, "message": "tokens revoked + wiped" }))
    }

    // ── Admin handlers ──────────────────────────────────────────

    /// Admin RPC: `nexo/admin/google/oauth_status` →
    /// per-agent OAuth snapshot.
    pub async fn admin_oauth_status(&self, agent_id: &str) -> Result<Value> {
        let client = self.client_for(agent_id)?;
        Ok(client.snapshot().await)
    }

    /// Admin RPC: `nexo/admin/google/oauth_revoke` → revoke + wipe.
    pub async fn admin_oauth_revoke(&self, agent_id: &str) -> Result<Value> {
        let client = self.client_for(agent_id)?;
        client.revoke().await?;
        Ok(json!({ "ok": true, "agent_id": agent_id }))
    }

    /// Admin RPC: `nexo/admin/google/list_tokens` → snapshot per
    /// configured agent.
    pub async fn admin_list_tokens(&self) -> Result<Value> {
        let mut out: Vec<Value> = Vec::with_capacity(self.clients.len());
        // Collect keys first to avoid holding dashmap iter across `.await`.
        let agent_ids: Vec<String> = self
            .clients
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        for agent_id in agent_ids {
            let Some(client_arc) = self.clients.get(&agent_id).map(|r| Arc::clone(r.value())) else {
                continue;
            };
            let snap = client_arc.snapshot().await;
            out.push(json!({
                "agent_id": agent_id,
                "status": snap,
            }));
        }
        Ok(json!({ "agents": out }))
    }
}

/// Extract `agent_id` from a JSON-RPC params object. Looks first
/// for the canonical top-level field; falls back to a `_meta`
/// envelope so daemon paths that nest metadata still resolve.
pub fn extract_agent_id(params: &Value) -> Result<String> {
    params
        .get("agent_id")
        .and_then(|v| v.as_str())
        .or_else(|| params.get("_meta").and_then(|m| m.get("agent_id")).and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .context(
            "outbound tool call is missing `agent_id` in params \
             (daemon must include it — Phase 94 Stage 5 agnostic infra)",
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(id: &str, ws: &str) -> GooglePluginConfig {
        GooglePluginConfig {
            agent_id: id.into(),
            workspace_dir: ws.into(),
            client_id: "test-cid".into(),
            client_secret: "test-cs".into(),
            scopes: vec!["gmail.readonly".into()],
            token_file: "google_tokens.json".into(),
            redirect_port: 0, // tests don't bind
        }
    }

    #[tokio::test]
    async fn on_configure_populates_dashmap() {
        let dir = tempfile::tempdir().unwrap();
        let p = GooglePlugin::new();
        let configs = vec![
            cfg("agent_a", &dir.path().join("a").to_string_lossy()),
            cfg("agent_b", &dir.path().join("b").to_string_lossy()),
        ];
        p.on_configure(configs).await.unwrap();
        assert_eq!(p.agent_count(), 2);
        assert!(p.client_for("agent_a").is_ok());
        assert!(p.client_for("agent_b").is_ok());
    }

    #[tokio::test]
    async fn on_configure_full_replace_drops_removed_agents() {
        let dir = tempfile::tempdir().unwrap();
        let p = GooglePlugin::new();
        p.on_configure(vec![
            cfg("agent_a", &dir.path().join("a").to_string_lossy()),
            cfg("agent_b", &dir.path().join("b").to_string_lossy()),
        ])
        .await
        .unwrap();
        assert_eq!(p.agent_count(), 2);
        // Second configure with only agent_c → both prior agents dropped.
        p.on_configure(vec![cfg("agent_c", &dir.path().join("c").to_string_lossy())])
            .await
            .unwrap();
        assert_eq!(p.agent_count(), 1);
        assert!(p.client_for("agent_a").is_err());
        assert!(p.client_for("agent_c").is_ok());
    }

    #[tokio::test]
    async fn invoke_outbound_tool_returns_status_for_known_agent() {
        let dir = tempfile::tempdir().unwrap();
        let p = GooglePlugin::new();
        p.on_configure(vec![cfg("agent_x", &dir.path().to_string_lossy())])
            .await
            .unwrap();
        let result = p
            .invoke_outbound_tool("google_auth_status", json!({}), "agent_x")
            .await
            .unwrap();
        // No tokens persisted → authenticated:false snapshot.
        assert_eq!(result["authenticated"], json!(false));
    }

    #[tokio::test]
    async fn invoke_outbound_tool_unknown_agent_errors() {
        let p = GooglePlugin::new();
        let err = p
            .invoke_outbound_tool("google_auth_status", json!({}), "ghost")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("ghost"));
        assert!(err.to_string().contains("not configured"));
    }

    #[tokio::test]
    async fn invoke_outbound_tool_unknown_tool_errors() {
        let dir = tempfile::tempdir().unwrap();
        let p = GooglePlugin::new();
        p.on_configure(vec![cfg("agent_x", &dir.path().to_string_lossy())])
            .await
            .unwrap();
        let err = p
            .invoke_outbound_tool("google_does_not_exist", json!({}), "agent_x")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown tool"));
    }

    #[tokio::test]
    async fn google_call_rejects_non_googleapis_url() {
        let dir = tempfile::tempdir().unwrap();
        let p = GooglePlugin::new();
        p.on_configure(vec![cfg("agent_x", &dir.path().to_string_lossy())])
            .await
            .unwrap();
        let err = p
            .invoke_outbound_tool(
                "google_call",
                json!({ "method": "GET", "url": "https://evil.example.com/" }),
                "agent_x",
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("googleapis.com"));
    }

    #[tokio::test]
    async fn admin_oauth_status_returns_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let p = GooglePlugin::new();
        p.on_configure(vec![cfg("agent_x", &dir.path().to_string_lossy())])
            .await
            .unwrap();
        let snap = p.admin_oauth_status("agent_x").await.unwrap();
        assert_eq!(snap["authenticated"], json!(false));
    }

    #[tokio::test]
    async fn admin_list_tokens_enumerates_configured_agents() {
        let dir = tempfile::tempdir().unwrap();
        let p = GooglePlugin::new();
        p.on_configure(vec![
            cfg("agent_a", &dir.path().join("a").to_string_lossy()),
            cfg("agent_b", &dir.path().join("b").to_string_lossy()),
        ])
        .await
        .unwrap();
        let listing = p.admin_list_tokens().await.unwrap();
        let agents = listing["agents"].as_array().expect("agents array");
        assert_eq!(agents.len(), 2);
        let ids: std::collections::HashSet<&str> = agents
            .iter()
            .map(|e| e["agent_id"].as_str().unwrap())
            .collect();
        assert!(ids.contains("agent_a"));
        assert!(ids.contains("agent_b"));
    }

    #[test]
    fn extract_agent_id_reads_top_level_field() {
        let p = json!({ "agent_id": "agent_x", "tool_name": "google_call" });
        assert_eq!(extract_agent_id(&p).unwrap(), "agent_x");
    }

    #[test]
    fn extract_agent_id_falls_back_to_meta_envelope() {
        let p = json!({ "_meta": { "agent_id": "agent_y" } });
        assert_eq!(extract_agent_id(&p).unwrap(), "agent_y");
    }

    #[test]
    fn extract_agent_id_errors_when_absent() {
        let p = json!({ "tool_name": "google_call" });
        assert!(extract_agent_id(&p).is_err());
    }
}
