//! plugin.configure round-trip — multi-account × multi-agent.

use std::path::Path;
use std::sync::Arc;

use nexo_plugin_google::plugin::{GoogleAccount, GoogleAuthFile, GooglePlugin};

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

#[tokio::test]
async fn empty_accounts_leaves_state_empty() {
    let p = Arc::new(GooglePlugin::new());
    p.on_configure(GoogleAuthFile::default()).await.unwrap();
    assert_eq!(p.account_count(), 0);
    assert_eq!(p.agent_count(), 0);
}

#[tokio::test]
async fn single_account_single_agent_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(GooglePlugin::new());
    let a = account("solo@gmail.com", "solo", dir.path());
    p.on_configure(GoogleAuthFile { accounts: vec![a] }).await.unwrap();
    assert_eq!(p.account_count(), 1);
    let default_acct = p.default_account_for("solo").unwrap();
    assert_eq!(default_acct, "solo@gmail.com");
    let client = p.client_by_account(&default_acct).unwrap();
    assert_eq!(client.config().client_id, "test-cid");
}

#[tokio::test]
async fn full_replace_drops_removed_accounts() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(GooglePlugin::new());
    p.on_configure(GoogleAuthFile {
        accounts: vec![
            account("a@gmail.com", "ana", dir.path()),
            account("b@gmail.com", "bob", dir.path()),
            account("c@gmail.com", "cat", dir.path()),
        ],
    })
    .await
    .unwrap();
    assert_eq!(p.account_count(), 3);

    p.on_configure(GoogleAuthFile {
        accounts: vec![account("a@gmail.com", "ana", dir.path())],
    })
    .await
    .unwrap();
    assert_eq!(p.account_count(), 1);
    assert!(p.client_by_account("a@gmail.com").is_ok());
    assert!(p.client_by_account("b@gmail.com").is_err());
    assert!(p.default_account_for("bob").is_err());
}

#[tokio::test]
async fn multi_account_per_agent_preserves_order() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(GooglePlugin::new());
    let a1 = account("ana@gmail.com", "ana", dir.path());
    let a2 = account("ana@work.com", "ana", dir.path());
    p.on_configure(GoogleAuthFile {
        accounts: vec![a1, a2],
    })
    .await
    .unwrap();

    assert_eq!(p.account_count(), 2);
    assert_eq!(p.agent_count(), 1);
    // Default account is the FIRST entry the operator declared.
    assert_eq!(p.default_account_for("ana").unwrap(), "ana@gmail.com");
    let listing = p.accounts_for_agent("ana");
    assert_eq!(listing, vec!["ana@gmail.com", "ana@work.com"]);
}

#[tokio::test]
async fn absolute_token_path_used_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(GooglePlugin::new());
    let mut a = account("solo@gmail.com", "solo", dir.path());
    let abs_token = dir.path().join("absolute_tokens.json");
    a.token_path = abs_token.clone();
    p.on_configure(GoogleAuthFile { accounts: vec![a] })
        .await
        .unwrap();
    let client = p.client_by_account("solo@gmail.com").unwrap();
    assert_eq!(client.token_path(), abs_token);
}
