//! In-process happy-path / error-path tests for the
//! `invoke_outbound_tool` dispatcher. No daemon, no live Google;
//! exercises argument validation + agent_id routing.

use std::sync::Arc;

use nexo_plugin_google::plugin::{GooglePlugin, GooglePluginConfig};
use serde_json::json;

async fn boot(agent_id: &str) -> (Arc<GooglePlugin>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(GooglePlugin::new());
    p.on_configure(vec![GooglePluginConfig {
        agent_id: agent_id.into(),
        workspace_dir: dir.path().to_string_lossy().into_owned(),
        client_id: "cid".into(),
        client_secret: "cs".into(),
        scopes: vec!["gmail.readonly".into()],
        token_file: "google_tokens.json".into(),
        redirect_port: 0,
    }])
    .await
    .unwrap();
    (p, dir)
}

#[tokio::test]
async fn google_auth_status_unauthenticated_when_no_tokens_on_disk() {
    let (p, _dir) = boot("agent_x").await;
    let out = p
        .invoke_outbound_tool("google_auth_status", json!({}), "agent_x")
        .await
        .unwrap();
    assert_eq!(out["authenticated"], json!(false));
    assert!(out["reason"].as_str().unwrap().contains("no tokens"));
}

#[tokio::test]
async fn unknown_tool_name_errors() {
    let (p, _dir) = boot("agent_x").await;
    let err = p
        .invoke_outbound_tool("google_bogus", json!({}), "agent_x")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown tool"));
}

#[tokio::test]
async fn unknown_agent_id_errors() {
    let (p, _dir) = boot("agent_x").await;
    let err = p
        .invoke_outbound_tool("google_auth_status", json!({}), "ghost")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("ghost"));
    assert!(err.to_string().contains("not configured"));
}

#[tokio::test]
async fn google_call_requires_method_arg() {
    let (p, _dir) = boot("agent_x").await;
    let err = p
        .invoke_outbound_tool(
            "google_call",
            json!({ "url": "https://gmail.googleapis.com/foo" }),
            "agent_x",
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("`method`"));
}

#[tokio::test]
async fn google_call_requires_url_arg() {
    let (p, _dir) = boot("agent_x").await;
    let err = p
        .invoke_outbound_tool(
            "google_call",
            json!({ "method": "GET" }),
            "agent_x",
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("`url`"));
}

#[tokio::test]
async fn google_call_rejects_non_https_urls() {
    let (p, _dir) = boot("agent_x").await;
    let err = p
        .invoke_outbound_tool(
            "google_call",
            json!({ "method": "GET", "url": "http://gmail.googleapis.com/foo" }),
            "agent_x",
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("https://"));
}

#[tokio::test]
async fn google_call_rejects_non_googleapis_hosts() {
    let (p, _dir) = boot("agent_x").await;
    let err = p
        .invoke_outbound_tool(
            "google_call",
            json!({ "method": "GET", "url": "https://example.com/" }),
            "agent_x",
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("googleapis.com"));
}

#[tokio::test]
async fn google_auth_revoke_succeeds_with_no_tokens() {
    let (p, _dir) = boot("agent_x").await;
    let out = p
        .invoke_outbound_tool("google_auth_revoke", json!({}), "agent_x")
        .await
        .unwrap();
    assert_eq!(out["ok"], json!(true));
}
