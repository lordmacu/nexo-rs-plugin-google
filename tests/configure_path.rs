//! plugin.configure round-trip in-process: build GooglePlugin,
//! deliver Vec<GooglePluginConfig>, verify the DashMap reflects
//! the requested set with full-replace semantics.

use std::sync::Arc;

use nexo_plugin_google::plugin::{GooglePlugin, GooglePluginConfig};

fn cfg(id: &str, workspace: &str) -> GooglePluginConfig {
    GooglePluginConfig {
        agent_id: id.into(),
        workspace_dir: workspace.into(),
        client_id: "cid".into(),
        client_secret: "cs".into(),
        scopes: vec!["gmail.readonly".into()],
        token_file: "google_tokens.json".into(),
        redirect_port: 0,
    }
}

#[tokio::test]
async fn empty_config_leaves_dashmap_empty() {
    let p = Arc::new(GooglePlugin::new());
    p.on_configure(Vec::new()).await.unwrap();
    assert_eq!(p.agent_count(), 0);
}

#[tokio::test]
async fn single_agent_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(GooglePlugin::new());
    p.on_configure(vec![cfg("solo", &dir.path().to_string_lossy())])
        .await
        .unwrap();
    assert_eq!(p.agent_count(), 1);
    let client = p.client_for("solo").unwrap();
    assert_eq!(client.config().client_id, "cid");
}

#[tokio::test]
async fn full_replace_drops_old_agents_and_inserts_new() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(GooglePlugin::new());

    p.on_configure(vec![
        cfg("a", &dir.path().join("a").to_string_lossy()),
        cfg("b", &dir.path().join("b").to_string_lossy()),
        cfg("c", &dir.path().join("c").to_string_lossy()),
    ])
    .await
    .unwrap();
    assert_eq!(p.agent_count(), 3);

    p.on_configure(vec![cfg("a", &dir.path().join("a").to_string_lossy())])
        .await
        .unwrap();
    assert_eq!(p.agent_count(), 1);
    assert!(p.client_for("a").is_ok());
    assert!(p.client_for("b").is_err());
    assert!(p.client_for("c").is_err());
}

#[tokio::test]
async fn workspace_relative_token_file_resolves_under_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(GooglePlugin::new());
    p.on_configure(vec![cfg("solo", &dir.path().to_string_lossy())])
        .await
        .unwrap();
    let client = p.client_for("solo").unwrap();
    let token_path = client.token_path();
    assert!(token_path.starts_with(dir.path()));
    assert!(token_path.ends_with("google_tokens.json"));
}

#[tokio::test]
async fn absolute_token_file_used_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let abs_token = dir.path().join("absolute_tokens.json");
    let mut config = cfg("solo", &dir.path().to_string_lossy());
    config.token_file = abs_token.to_string_lossy().into_owned();

    let p = Arc::new(GooglePlugin::new());
    p.on_configure(vec![config]).await.unwrap();
    let client = p.client_for("solo").unwrap();
    assert_eq!(client.token_path(), abs_token);
}
