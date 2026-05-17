//! Subprocess plugin runtime — multi-instance × multi-account.
//!
//! Operators describe their Google accounts in a single
//! `google-auth.yaml` mirroring the legacy in-tree shape:
//!
//! ```yaml
//! accounts:
//!   - id: ana@gmail.com
//!     agent_id: ana
//!     client_id_path:     ./secrets/ana_client_id.txt
//!     client_secret_path: ./secrets/ana_client_secret.txt
//!     token_path:         ./secrets/ana_token.json
//!     scopes: [gmail.readonly, calendar]
//!     redirect_port: 8765
//! ```
//!
//! The plugin holds one `Arc<GoogleAuthClient>` per `accounts[].id`
//! and a per-agent lookup table mapping `agent_id → [account_id, ...]`
//! so tools dispatch with optional `account:` arg (defaulting to the
//! first account for the calling agent). Mirrors the email plugin's
//! tenant × account fanout.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::client::{GoogleAuthClient, GoogleAuthConfig, SecretSources};

/// Operator-facing top-level config (mirrors `google-auth.yaml`).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GoogleAuthFile {
    #[serde(default)]
    pub accounts: Vec<GoogleAccount>,
}

/// Single Google account binding declared in `google-auth.yaml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GoogleAccount {
    /// Account identifier, conventionally the email address. Unique
    /// across the file. Tools accept it via the optional
    /// `account` arg.
    pub id: String,
    /// Agent that owns this account. Multiple accounts MAY share
    /// the same `agent_id` (multi-account-per-agent).
    pub agent_id: String,
    /// Path to a file holding the OAuth client_id.
    pub client_id_path: PathBuf,
    /// Path to a file holding the OAuth client_secret.
    pub client_secret_path: PathBuf,
    /// Where to persist tokens. Absolute or relative to CWD.
    pub token_path: PathBuf,
    /// Granted OAuth scopes. Short forms (`gmail.readonly`) get
    /// expanded by `canonicalize_scopes` inside the client.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Loopback callback port for `google_auth_start`. Default 8765.
    #[serde(default = "default_redirect_port")]
    pub redirect_port: u16,
}

fn default_redirect_port() -> u16 {
    8765
}

/// Plugin-side process-wide state.
pub struct GooglePlugin {
    /// account_id → client.
    accounts: DashMap<String, Arc<GoogleAuthClient>>,
    /// agent_id → ordered list of account_ids the agent owns. The
    /// first entry is the default account when the LLM omits the
    /// `account` arg.
    by_agent: DashMap<String, Vec<String>>,
}

impl Default for GooglePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl GooglePlugin {
    pub fn new() -> Self {
        Self {
            accounts: DashMap::new(),
            by_agent: DashMap::new(),
        }
    }

    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }

    pub fn agent_count(&self) -> usize {
        self.by_agent.len()
    }

    /// Replace the in-memory state from a parsed `google-auth.yaml`.
    /// Full-replace semantics: prior accounts not present in the
    /// new payload are dropped. File-path reads happen lazily here
    /// (so a rotation between configures is honoured), and the
    /// resulting client carries `SecretSources` for lazy refresh
    /// on subsequent mtime changes.
    pub async fn on_configure(&self, file: GoogleAuthFile) -> Result<()> {
        let next_accounts: DashMap<String, Arc<GoogleAuthClient>> = DashMap::new();
        let next_by_agent: DashMap<String, Vec<String>> = DashMap::new();

        for acct in file.accounts {
            let client_id = read_trim(&acct.client_id_path).with_context(|| {
                format!(
                    "account `{}`: reading client_id from {}",
                    acct.id,
                    acct.client_id_path.display()
                )
            })?;
            let client_secret = read_trim(&acct.client_secret_path).with_context(|| {
                format!(
                    "account `{}`: reading client_secret from {}",
                    acct.id,
                    acct.client_secret_path.display()
                )
            })?;

            let token_file = acct.token_path.to_string_lossy().into_owned();
            let workspace_dir = acct
                .token_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));

            let cfg = GoogleAuthConfig {
                client_id,
                client_secret,
                scopes: acct.scopes.clone(),
                token_file,
                redirect_port: acct.redirect_port,
            };
            let sources = SecretSources {
                client_id_path: acct.client_id_path.clone(),
                client_secret_path: acct.client_secret_path.clone(),
            };
            let client = GoogleAuthClient::new_with_sources(cfg, &workspace_dir, Some(sources));
            if let Err(e) = client.load_from_disk().await {
                tracing::warn!(
                    target = "nexo_plugin_google",
                    account = %acct.id,
                    error = %e,
                    "tokens load failed; account will need to re-consent"
                );
            }

            next_accounts.insert(acct.id.clone(), client);
            next_by_agent
                .entry(acct.agent_id.clone())
                .or_default()
                .push(acct.id.clone());
        }

        self.accounts.clear();
        for (k, v) in next_accounts.into_iter() {
            self.accounts.insert(k, v);
        }
        self.by_agent.clear();
        for (k, v) in next_by_agent.into_iter() {
            self.by_agent.insert(k, v);
        }

        tracing::info!(
            target = "nexo_plugin_google",
            accounts = self.accounts.len(),
            agents = self.by_agent.len(),
            "google plugin reconfigured"
        );
        Ok(())
    }

    /// Resolve the per-account `Arc<GoogleAuthClient>`. Public so
    /// admin handlers can introspect.
    pub fn client_by_account(&self, account_id: &str) -> Result<Arc<GoogleAuthClient>> {
        self.accounts
            .get(account_id)
            .map(|r| Arc::clone(r.value()))
            .ok_or_else(|| anyhow!("account `{account_id}` is not configured"))
    }

    /// Resolve the per-agent default account. Returns the first
    /// account in the agent's list; errors if the agent has no
    /// accounts.
    pub fn default_account_for(&self, agent_id: &str) -> Result<String> {
        let list = self.by_agent.get(agent_id).ok_or_else(|| {
            anyhow!(
                "agent `{agent_id}` is not configured for google_auth \
                     (no entry in `google-auth.yaml::accounts[].agent_id`)"
            )
        })?;
        list.first()
            .cloned()
            .ok_or_else(|| anyhow!("agent `{agent_id}` is configured but has zero accounts"))
    }

    /// Pick the account this call targets:
    ///   1. `args.account` (operator-supplied);
    ///   2. else fall back to the agent's default (first) account.
    fn resolve_account(&self, args: &Value, agent_id: &str) -> Result<String> {
        if let Some(explicit) = args.get("account").and_then(|v| v.as_str()) {
            return Ok(explicit.to_string());
        }
        self.default_account_for(agent_id)
    }

    pub fn accounts_for_agent(&self, agent_id: &str) -> Vec<String> {
        self.by_agent
            .get(agent_id)
            .map(|r| r.clone())
            .unwrap_or_default()
    }

    /// Dispatch a `tool.invoke` call. The daemon's RemoteToolHandler
    /// passes the LLM's `args` payload plus `agent_id` from the
    /// per-agent context.
    pub async fn invoke_outbound_tool(
        &self,
        tool_name: &str,
        args: Value,
        agent_id: &str,
    ) -> Result<Value> {
        let account_id = self.resolve_account(&args, agent_id)?;
        let client = self.client_by_account(&account_id)?;
        match tool_name {
            "google_auth_start" => self.tool_auth_start(&client, &account_id).await,
            "google_auth_status" => {
                let mut snap = client.snapshot().await;
                if let Some(map) = snap.as_object_mut() {
                    map.insert("account".into(), json!(account_id));
                }
                Ok(snap)
            }
            "google_call" => self.tool_call(&client, &args, &account_id).await,
            "google_auth_revoke" => self.tool_revoke(&client, &account_id).await,
            other => Err(anyhow!("unknown tool `{other}`")),
        }
    }

    async fn tool_auth_start(
        &self,
        client: &Arc<GoogleAuthClient>,
        account_id: &str,
    ) -> Result<Value> {
        let (url, _join) = client.start_auth_flow().await?;
        let redirect_port = client.config().redirect_port;
        Ok(json!({
            "ok": true,
            "account": account_id,
            "url": url,
            "instructions": "Open this URL in a browser you're logged into \
                your Google account with, approve the scopes, then call \
                google_auth_status to confirm.",
            "redirect_uri": format!("http://127.0.0.1:{redirect_port}/callback"),
        }))
    }

    async fn tool_call(
        &self,
        client: &Arc<GoogleAuthClient>,
        args: &Value,
        account_id: &str,
    ) -> Result<Value> {
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
        let body = args.get("body").filter(|b| !b.is_null()).cloned();
        let resp = client.authorized_call(method, url, body).await?;
        Ok(json!({ "ok": true, "account": account_id, "response": resp }))
    }

    async fn tool_revoke(&self, client: &Arc<GoogleAuthClient>, account_id: &str) -> Result<Value> {
        client.revoke().await?;
        Ok(json!({
            "ok": true,
            "account": account_id,
            "message": "tokens revoked + wiped"
        }))
    }

    // ── Admin handlers ──────────────────────────────────────────

    pub async fn admin_oauth_status(&self, agent_id: &str, account: Option<&str>) -> Result<Value> {
        let account_id = match account {
            Some(a) => a.to_string(),
            None => self.default_account_for(agent_id)?,
        };
        let client = self.client_by_account(&account_id)?;
        let mut snap = client.snapshot().await;
        if let Some(map) = snap.as_object_mut() {
            map.insert("account".into(), json!(account_id));
        }
        Ok(snap)
    }

    pub async fn admin_oauth_revoke(&self, agent_id: &str, account: Option<&str>) -> Result<Value> {
        let account_id = match account {
            Some(a) => a.to_string(),
            None => self.default_account_for(agent_id)?,
        };
        let client = self.client_by_account(&account_id)?;
        client.revoke().await?;
        Ok(json!({ "ok": true, "agent_id": agent_id, "account": account_id }))
    }

    pub async fn admin_list_tokens(&self) -> Result<Value> {
        let mut out: Vec<Value> = Vec::with_capacity(self.accounts.len());
        let account_ids: Vec<String> = self.accounts.iter().map(|e| e.key().clone()).collect();
        for account_id in account_ids {
            let Some(client_arc) = self
                .accounts
                .get(&account_id)
                .map(|r| Arc::clone(r.value()))
            else {
                continue;
            };
            let snap = client_arc.snapshot().await;
            out.push(json!({
                "account": account_id,
                "status": snap,
            }));
        }
        // Also expose agent → accounts mapping so the admin UI can
        // render multi-account UX.
        let mut agents: Vec<Value> = Vec::with_capacity(self.by_agent.len());
        for entry in self.by_agent.iter() {
            agents.push(json!({
                "agent_id": entry.key(),
                "accounts": entry.value(),
            }));
        }
        Ok(json!({ "accounts": out, "agents": agents }))
    }
}

fn read_trim(path: &Path) -> std::io::Result<String> {
    let raw = std::fs::read_to_string(path)?;
    Ok(raw.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, agent: &str, dir: &Path) -> (GoogleAccount, PathBuf, PathBuf) {
        let cid_path = dir.join(format!("{id}_cid.txt"));
        let cs_path = dir.join(format!("{id}_cs.txt"));
        std::fs::write(&cid_path, "test-cid").unwrap();
        std::fs::write(&cs_path, "test-cs").unwrap();
        let acct = GoogleAccount {
            id: id.into(),
            agent_id: agent.into(),
            client_id_path: cid_path.clone(),
            client_secret_path: cs_path.clone(),
            token_path: dir.join(format!("{id}_token.json")),
            scopes: vec!["gmail.readonly".into()],
            redirect_port: 0,
        };
        (acct, cid_path, cs_path)
    }

    #[tokio::test]
    async fn on_configure_loads_two_accounts_one_agent() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = GooglePlugin::new();

        let (a1, _, _) = account("ana@gmail.com", "ana", dir.path());
        let (a2, _, _) = account("ana@work.com", "ana", dir.path());

        plugin
            .on_configure(GoogleAuthFile {
                accounts: vec![a1, a2],
            })
            .await
            .unwrap();

        assert_eq!(plugin.account_count(), 2);
        assert_eq!(plugin.agent_count(), 1);
        let accounts_for_ana = plugin.accounts_for_agent("ana");
        assert_eq!(accounts_for_ana.len(), 2);
        assert!(accounts_for_ana.contains(&"ana@gmail.com".to_string()));
        assert!(accounts_for_ana.contains(&"ana@work.com".to_string()));
    }

    #[tokio::test]
    async fn full_replace_on_second_configure() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = GooglePlugin::new();
        let (a, _, _) = account("a@gmail.com", "ana", dir.path());
        let (b, _, _) = account("b@gmail.com", "bob", dir.path());

        plugin
            .on_configure(GoogleAuthFile {
                accounts: vec![a, b],
            })
            .await
            .unwrap();
        assert_eq!(plugin.account_count(), 2);

        let (a, _, _) = account("a@gmail.com", "ana", dir.path());
        plugin
            .on_configure(GoogleAuthFile { accounts: vec![a] })
            .await
            .unwrap();
        assert_eq!(plugin.account_count(), 1);
        assert!(plugin.accounts_for_agent("bob").is_empty());
    }

    #[tokio::test]
    async fn invoke_resolves_default_account_for_agent() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = GooglePlugin::new();
        let (a, _, _) = account("ana@gmail.com", "ana", dir.path());
        plugin
            .on_configure(GoogleAuthFile { accounts: vec![a] })
            .await
            .unwrap();

        let out = plugin
            .invoke_outbound_tool("google_auth_status", json!({}), "ana")
            .await
            .unwrap();
        assert_eq!(out["account"], json!("ana@gmail.com"));
        assert_eq!(out["authenticated"], json!(false));
    }

    #[tokio::test]
    async fn invoke_picks_explicit_account_arg() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = GooglePlugin::new();
        let (a, _, _) = account("ana@gmail.com", "ana", dir.path());
        let (b, _, _) = account("ana@work.com", "ana", dir.path());
        plugin
            .on_configure(GoogleAuthFile {
                accounts: vec![a, b],
            })
            .await
            .unwrap();

        let out = plugin
            .invoke_outbound_tool(
                "google_auth_status",
                json!({ "account": "ana@work.com" }),
                "ana",
            )
            .await
            .unwrap();
        assert_eq!(out["account"], json!("ana@work.com"));
    }

    #[tokio::test]
    async fn invoke_unknown_agent_errors() {
        let plugin = GooglePlugin::new();
        let err = plugin
            .invoke_outbound_tool("google_auth_status", json!({}), "ghost")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("ghost"));
        assert!(err.to_string().contains("not configured"));
    }

    #[tokio::test]
    async fn invoke_unknown_account_errors() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = GooglePlugin::new();
        let (a, _, _) = account("ana@gmail.com", "ana", dir.path());
        plugin
            .on_configure(GoogleAuthFile { accounts: vec![a] })
            .await
            .unwrap();

        let err = plugin
            .invoke_outbound_tool(
                "google_auth_status",
                json!({ "account": "nobody@gmail.com" }),
                "ana",
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("nobody@gmail.com"));
    }

    #[tokio::test]
    async fn google_call_rejects_non_googleapis_host() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = GooglePlugin::new();
        let (a, _, _) = account("a@gmail.com", "ana", dir.path());
        plugin
            .on_configure(GoogleAuthFile { accounts: vec![a] })
            .await
            .unwrap();
        let err = plugin
            .invoke_outbound_tool(
                "google_call",
                json!({ "method": "GET", "url": "https://evil.example.com/" }),
                "ana",
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("googleapis.com"));
    }

    #[tokio::test]
    async fn admin_list_tokens_returns_per_account_and_per_agent() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = GooglePlugin::new();
        let (a, _, _) = account("a@gmail.com", "ana", dir.path());
        let (b, _, _) = account("b@gmail.com", "ana", dir.path());
        plugin
            .on_configure(GoogleAuthFile {
                accounts: vec![a, b],
            })
            .await
            .unwrap();
        let listing = plugin.admin_list_tokens().await.unwrap();
        assert_eq!(listing["accounts"].as_array().unwrap().len(), 2);
        let agents = listing["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["agent_id"], json!("ana"));
        assert_eq!(agents[0]["accounts"].as_array().unwrap().len(), 2);
    }
}
