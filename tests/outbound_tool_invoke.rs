//! In-process happy-path / error-path tests for the
//! `invoke_outbound_tool` dispatcher with multi-account fanout.

use std::path::Path;
use std::sync::Arc;

use nexo_plugin_google::plugin::{GoogleAccount, GoogleAuthFile, GooglePlugin};
use serde_json::json;

fn account(id: &str, agent: &str, dir: &Path) -> GoogleAccount {
    let cid_path = dir.join(format!("{id}_cid.txt"));
    let cs_path = dir.join(format!("{id}_cs.txt"));
    std::fs::write(&cid_path, "test-cid").unwrap();
    std::fs::write(&cs_path, "test-cs").unwrap();
    GoogleAccount {
        id: id.into(),
        agent_id: agent.into(),
        client_id_path: cid_path,
        client_secret_path: cs_path,
        token_path: dir.join(format!("{id}_token.json")),
        scopes: vec!["gmail.readonly".into()],
        redirect_port: 0,
    }
}

async fn boot_single_account(agent_id: &str, account_id: &str) -> (Arc<GooglePlugin>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(GooglePlugin::new());
    p.on_configure(GoogleAuthFile {
        accounts: vec![account(account_id, agent_id, dir.path())],
    })
    .await
    .unwrap();
    (p, dir)
}

#[tokio::test]
async fn google_auth_status_unauthenticated_when_no_tokens_on_disk() {
    let (p, _dir) = boot_single_account("agent_x", "agent_x@gmail.com").await;
    let out = p
        .invoke_outbound_tool("google_auth_status", json!({}), "agent_x")
        .await
        .unwrap();
    assert_eq!(out["authenticated"], json!(false));
    assert_eq!(out["account"], json!("agent_x@gmail.com"));
}

#[tokio::test]
async fn explicit_account_arg_picks_account() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(GooglePlugin::new());
    p.on_configure(GoogleAuthFile {
        accounts: vec![
            account("ana@gmail.com", "ana", dir.path()),
            account("ana@work.com", "ana", dir.path()),
        ],
    })
    .await
    .unwrap();

    let out = p
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
async fn unknown_tool_errors() {
    let (p, _dir) = boot_single_account("agent_x", "agent_x@gmail.com").await;
    let err = p
        .invoke_outbound_tool("google_bogus", json!({}), "agent_x")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown tool"));
}

#[tokio::test]
async fn unknown_agent_errors() {
    let (p, _dir) = boot_single_account("agent_x", "agent_x@gmail.com").await;
    let err = p
        .invoke_outbound_tool("google_auth_status", json!({}), "ghost")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("ghost"));
    assert!(err.to_string().contains("not configured"));
}

#[tokio::test]
async fn unknown_account_errors() {
    let (p, _dir) = boot_single_account("agent_x", "agent_x@gmail.com").await;
    let err = p
        .invoke_outbound_tool(
            "google_auth_status",
            json!({ "account": "nobody@gmail.com" }),
            "agent_x",
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("nobody@gmail.com"));
}

#[tokio::test]
async fn google_call_requires_method_arg() {
    let (p, _dir) = boot_single_account("agent_x", "agent_x@gmail.com").await;
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
    let (p, _dir) = boot_single_account("agent_x", "agent_x@gmail.com").await;
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
    let (p, _dir) = boot_single_account("agent_x", "agent_x@gmail.com").await;
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
    let (p, _dir) = boot_single_account("agent_x", "agent_x@gmail.com").await;
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
    let (p, _dir) = boot_single_account("agent_x", "agent_x@gmail.com").await;
    let out = p
        .invoke_outbound_tool("google_auth_revoke", json!({}), "agent_x")
        .await
        .unwrap();
    assert_eq!(out["ok"], json!(true));
    assert_eq!(out["account"], json!("agent_x@gmail.com"));
}
