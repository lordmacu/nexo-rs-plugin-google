//! Subprocess + CLI entrypoint for `nexo-plugin-google` (Phase 94).
//!
//! Dispatch:
//!   * `nexo-plugin-google --print-manifest` → echo bundled manifest.
//!   * `nexo-plugin-google --oauth-once <agent_id> [flags]` →
//!     interactive OAuth consent flow (loopback default; `--device`
//!     switches to device-code; `--remote` auto-detects WSL2/SSH).
//!   * no subcommand → boot the long-lived JSON-RPC dispatch loop
//!     against stdin/stdout (daemon-spawned mode). Wired in step 16.

use clap::Parser;

use nexo_plugin_google::cli::{Cli, Command};

const MANIFEST: &str = include_str!("../nexo-plugin.toml");

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Command::PrintManifest) => {
            print!("{}", MANIFEST);
            Ok(())
        }
        Some(Command::OauthOnce(args)) => {
            nexo_plugin_google::cli::run_oauth_once(args).await
        }
        None => {
            // Phase 94 step 16 — long-lived JSON-RPC dispatch loop.
            // Placeholder while plugin runtime is wired up.
            eprintln!(
                "nexo-plugin-google: JSON-RPC dispatch loop not yet implemented \
                 (Phase 94 step 16). Run with --print-manifest or --oauth-once."
            );
            std::process::exit(78); // EX_CONFIG
        }
    }
}
