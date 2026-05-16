//! Bundled `nexo-plugin.toml` parses cleanly and declares the
//! expected 4 outbound tools + all required manifest sections.

use nexo_plugin_manifest::manifest::{ConfigShape, PluginManifest};

const MANIFEST: &str = include_str!("../nexo-plugin.toml");

#[test]
fn manifest_parses_as_v2() {
    let parsed: PluginManifest =
        toml::from_str(MANIFEST).expect("nexo-plugin.toml parses as PluginManifest");
    assert_eq!(parsed.manifest_version, 2, "must be v2 canonical");
    assert_eq!(parsed.plugin.id, "google");
    assert_eq!(parsed.plugin.version.to_string(), "0.2.1");
}

#[test]
fn extends_tools_declares_exactly_four() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let tools = &parsed.plugin.extends.tools;
    assert_eq!(tools.len(), 4);
    assert_eq!(
        tools,
        &vec![
            "google_auth_start".to_string(),
            "google_auth_status".to_string(),
            "google_call".to_string(),
            "google_auth_revoke".to_string(),
        ]
    );
}

#[test]
fn expose_list_matches_extends_tools() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let exposed = &parsed.plugin.tools.expose;
    assert_eq!(exposed.len(), 4);
    for name in &parsed.plugin.extends.tools {
        assert!(
            exposed.contains(name),
            "{name} missing from tools.expose list"
        );
    }
}

#[test]
fn admin_section_present_with_correct_prefix() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let admin = parsed
        .plugin
        .admin
        .as_ref()
        .expect("[plugin.admin] required");
    assert_eq!(admin.method_prefix, "nexo/admin/google/");
    assert_eq!(admin.broker_topic_prefix, "plugin.google.admin");
}

#[test]
fn config_schema_is_object_shape_with_accounts_array() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let cs = parsed
        .plugin
        .config_schema
        .as_ref()
        .expect("[plugin.config_schema] required");
    assert_eq!(cs.shape, ConfigShape::Object);
    // The schema string must declare an `accounts` array — that's
    // the contract operators rely on when authoring
    // `google-auth.yaml`.
    assert!(
        cs.schema.contains("\"accounts\""),
        "schema must declare an accounts array"
    );
}

#[test]
fn credentials_schema_opted_out() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let cs = parsed
        .plugin
        .credentials_schema
        .as_ref()
        .expect("[plugin.credentials_schema] required (even when opting out)");
    assert!(
        !cs.enabled,
        "google plugin reads OAuth file refs directly; no RemoteCredentialStore"
    );
}

#[test]
fn dashboard_layout_workspace_walk() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let dash = parsed
        .plugin
        .dashboard
        .as_ref()
        .expect("[plugin.dashboard] required");
    // serde-flattened on shape; assert layout subdir.
    let serialised = toml::to_string(dash).unwrap();
    assert!(
        serialised.contains("workspace_walk"),
        "dashboard.layout must be workspace_walk: {serialised}"
    );
    assert!(
        serialised.contains("google_tokens.json"),
        "dashboard.auth_check must reference google_tokens.json: {serialised}"
    );
}

#[test]
fn entrypoint_command_matches_bin_name() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    assert_eq!(
        parsed.plugin.entrypoint.command.as_deref(),
        Some("nexo-plugin-google")
    );
}

#[test]
fn capabilities_broker_subscribes_to_outbound_and_admin() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let broker = parsed
        .plugin
        .capabilities
        .broker
        .as_ref()
        .expect("[plugin.capabilities.broker] required");
    assert!(broker.subscribe.iter().any(|t| t == "plugin.outbound.google"));
    assert!(broker
        .subscribe
        .iter()
        .any(|t| t == "plugin.outbound.google.>"));
    assert!(broker.subscribe.iter().any(|t| t == "plugin.google.admin.>"));
}
