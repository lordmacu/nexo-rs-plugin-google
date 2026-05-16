//! Integration smoke-tests for the binary's CLI surface.
//!
//! Spawns the built `nexo-plugin-google` binary directly + verifies
//! the externally-observable CLI contract:
//!   * `--print-manifest` emits a TOML that parses as `PluginManifest`.
//!   * `--oauth-once --help` exits 0.
//!   * `--oauth-once` missing required args exits non-zero.

use std::process::Command;

use nexo_plugin_manifest::manifest::PluginManifest;

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_nexo-plugin-google")
}

#[test]
fn print_manifest_emits_valid_v2_manifest() {
    let out = Command::new(bin_path())
        .arg("--print-manifest")
        .output()
        .expect("spawn binary");
    assert!(
        out.status.success(),
        "exit={} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8 stdout");
    let parsed: PluginManifest =
        toml::from_str(&stdout).expect("emitted manifest parses as PluginManifest");
    assert_eq!(parsed.manifest_version, 2);
    assert_eq!(parsed.plugin.id, "google");
    assert_eq!(parsed.plugin.extends.tools.len(), 4);
}

#[test]
fn print_manifest_then_short_circuit_exit() {
    // Even when stdout is piped through a small consumer, the bin
    // exits with code 0 without entering the JSON-RPC dispatch loop.
    let out = Command::new(bin_path())
        .arg("--print-manifest")
        .output()
        .expect("spawn binary");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn oauth_once_help_exits_zero() {
    let out = Command::new(bin_path())
        .args(["--oauth-once", "--help"])
        .output()
        .expect("spawn binary");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--client-id-file"));
    assert!(stdout.contains("--client-secret-file"));
    assert!(stdout.contains("--token-file"));
    assert!(stdout.contains("--scopes"));
    assert!(stdout.contains("--workspace-dir"));
}

#[test]
fn oauth_once_missing_required_flag_exits_nonzero() {
    let out = Command::new(bin_path())
        .args(["--oauth-once", "agent_x", "--client-id-file", "/tmp/cid"])
        .output()
        .expect("spawn binary");
    assert!(
        !out.status.success(),
        "expected non-zero exit on missing flags"
    );
}

// `no_subcommand_exits_nonzero_with_hint` removed in step 16 —
// bare invocation now enters the long-lived JSON-RPC dispatch loop
// (see `e2e_handshake.rs::initialize_reply_lists_four_google_tools`
// for the new contract).
