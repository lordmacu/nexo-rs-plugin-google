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
    assert_eq!(parsed.plugin.version.to_string(), "0.2.0");
}

#[test]
fn outbound_tools_declare_exactly_four() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let outbound = &parsed.plugin.tools.outbound;
    assert_eq!(outbound.len(), 4, "expect 4 outbound tool specs");
    let names: Vec<&str> = outbound.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "google_auth_start",
            "google_auth_status",
            "google_call",
            "google_auth_revoke",
        ]
    );
}

#[test]
fn expose_list_matches_outbound_names() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let exposed = &parsed.plugin.tools.expose;
    assert_eq!(exposed.len(), 4);
    for spec in &parsed.plugin.tools.outbound {
        assert!(
            exposed.iter().any(|e| e == &spec.name),
            "{} missing from expose list",
            spec.name
        );
    }
}

#[test]
fn outbound_input_schemas_are_valid_json() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    for spec in &parsed.plugin.tools.outbound {
        let v: serde_json::Value = serde_json::from_str(&spec.input_schema)
            .unwrap_or_else(|e| panic!("{}: input_schema not JSON: {e}", spec.name));
        assert_eq!(
            v["type"], "object",
            "{}: input schema must be type=object",
            spec.name
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
fn config_schema_is_array_shape() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let cs = parsed
        .plugin
        .config_schema
        .as_ref()
        .expect("[plugin.config_schema] required");
    assert_eq!(cs.shape, ConfigShape::Array);
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
