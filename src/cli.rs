//! CLI subcommand wiring for `nexo-plugin-google`.
//!
//! Two interactive subcommands plus the long-lived plugin dispatch
//! mode (covered by `main.rs` when no subcommand is given):
//!
//!   * `--print-manifest` — emit the bundled `nexo-plugin.toml` to
//!     stdout. The daemon's discovery walker calls this during boot
//!     to register the plugin without spawning the JSON-RPC loop.
//!   * `--oauth-once <agent_id>` — run the OAuth consent flow once
//!     (loopback by default; `--device` switches to device-code;
//!     `--remote` auto-detects WSL2 / SSH and prefers device-code).
//!
//! Designed so the setup wizard's `nexo-setup` Rust crate can spawn
//! this binary instead of linking the plugin crate directly — keeps
//! the workspace daemon and setup tooling free of the OAuth machinery.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};

use crate::client::{GoogleAuthClient, GoogleAuthConfig};

#[derive(Debug, Parser)]
#[command(
    name = "nexo-plugin-google",
    version,
    about = "Google APIs (Gmail/Calendar/Drive) tool plugin for Nexo agents.",
    long_about = "Subprocess binary loaded by the nexo-rs daemon via discovery. \
Without a subcommand, boots the long-lived JSON-RPC dispatch loop on stdin/stdout. \
Use --print-manifest to dump the bundled manifest, or --oauth-once <agent_id> to \
run the OAuth consent flow interactively (setup wizard path)."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Emit the bundled `nexo-plugin.toml` and exit. Used by the
    /// daemon's discovery walker to register the plugin without
    /// spawning the JSON-RPC loop.
    #[command(name = "--print-manifest")]
    PrintManifest,

    /// Run the OAuth consent flow once + persist the resulting
    /// tokens. Setup wizard spawns this from its Google service
    /// step.
    #[command(name = "--oauth-once")]
    OauthOnce(OauthOnceArgs),
}

#[derive(Debug, clap::Args)]
pub struct OauthOnceArgs {
    /// Agent identifier the tokens belong to. Used for logging +
    /// future per-agent workspace resolution. Required positional.
    pub agent_id: String,

    /// Path to the file holding the OAuth client_id. Daemon-side
    /// secret store typically lands these at
    /// `secrets/<agent>_google_client_id.txt`.
    #[arg(long, value_name = "PATH")]
    pub client_id_file: PathBuf,

    /// Path to the file holding the OAuth client_secret.
    #[arg(long, value_name = "PATH")]
    pub client_secret_file: PathBuf,

    /// Absolute path where tokens should land on success.
    #[arg(long, value_name = "PATH")]
    pub token_file: PathBuf,

    /// Comma-separated OAuth scopes (short forms accepted —
    /// `gmail.readonly` is expanded to the full canonical URL).
    #[arg(long, value_name = "SCOPES")]
    pub scopes: String,

    /// Loopback callback port — must match an "Authorized redirect
    /// URI" entry in the OAuth client config:
    /// `http://127.0.0.1:<port>/callback`.
    #[arg(long, default_value_t = 8765, value_name = "PORT")]
    pub redirect_port: u16,

    /// Workspace directory. Tokens persist here when `--token-file`
    /// is relative.
    #[arg(long, value_name = "PATH")]
    pub workspace_dir: PathBuf,

    /// Use device-code flow (operator types `user_code` at
    /// `verification_url` on a second device). Default is loopback
    /// (browser hits 127.0.0.1).
    #[arg(long)]
    pub device: bool,

    /// Treat the current shell as remote (SSH / WSL2 / container)
    /// and prefer device-code over loopback. Mirrors OpenClaw's
    /// `shouldUseManualOAuthFlow(isRemote)`.
    #[arg(long)]
    pub remote: bool,
}

/// Detect remote shells where the loopback listener can't be
/// reached by the operator's browser (SSH session, WSL2). Mirrors
/// OpenClaw's `shouldUseManualOAuthFlow(isRemote)` heuristic.
pub fn should_use_device_flow(args: &OauthOnceArgs) -> bool {
    if args.device {
        return true;
    }
    if args.remote {
        return true;
    }
    if std::env::var_os("SSH_CONNECTION").is_some()
        || std::env::var_os("SSH_TTY").is_some()
        || std::env::var_os("SSH_CLIENT").is_some()
    {
        return true;
    }
    if is_wsl2() {
        return true;
    }
    false
}

fn is_wsl2() -> bool {
    if let Ok(s) = std::fs::read_to_string("/proc/sys/kernel/osrelease") {
        return s.to_lowercase().contains("microsoft");
    }
    false
}

/// Run the OAuth consent flow once. Returns when tokens are
/// persisted to disk OR an error surfaces. Stdout / stderr stream
/// operator-facing prompts.
pub async fn run_oauth_once(args: OauthOnceArgs) -> Result<()> {
    let client_id = tokio::fs::read_to_string(&args.client_id_file)
        .await
        .with_context(|| {
            format!(
                "reading client_id file at {}",
                args.client_id_file.display()
            )
        })?
        .trim()
        .to_string();
    let client_secret = tokio::fs::read_to_string(&args.client_secret_file)
        .await
        .with_context(|| {
            format!(
                "reading client_secret file at {}",
                args.client_secret_file.display()
            )
        })?
        .trim()
        .to_string();

    if client_id.is_empty() {
        bail!("client_id file empty: {}", args.client_id_file.display());
    }
    if client_secret.is_empty() {
        bail!(
            "client_secret file empty: {}",
            args.client_secret_file.display()
        );
    }

    let scopes: Vec<String> = args
        .scopes
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if scopes.is_empty() {
        bail!("--scopes must list at least one OAuth scope");
    }

    let token_file_str = args.token_file.to_string_lossy().into_owned();
    let cfg = GoogleAuthConfig {
        client_id,
        client_secret,
        scopes,
        token_file: token_file_str,
        redirect_port: args.redirect_port,
    };

    let client = GoogleAuthClient::new(cfg, &args.workspace_dir);

    if should_use_device_flow(&args) {
        run_device_flow(&client, &args.agent_id).await
    } else {
        run_loopback_flow(&client, &args.agent_id, args.redirect_port).await
    }
}

async fn run_loopback_flow(
    client: &Arc<GoogleAuthClient>,
    agent_id: &str,
    redirect_port: u16,
) -> Result<()> {
    let (url, handle) = client
        .start_auth_flow()
        .await
        .context("starting loopback OAuth flow")?;

    println!();
    println!("╭─ Google OAuth consent (loopback) ─────────────────────────────╮");
    println!("│ 1. Abrí este link en cualquier browser (laptop/celular):       │");
    println!("╰────────────────────────────────────────────────────────────────╯");
    println!();
    println!("  {url}");
    println!();
    println!("2. Login con tu cuenta de Google → click Allow.");
    println!();
    println!(
        "3. El browser te va a redirigir a `http://127.0.0.1:{redirect_port}/callback?…` y \
         vas a ver `Authorisation received ✓`."
    );
    println!();
    println!("Esperando callback en 127.0.0.1:{redirect_port}…");

    let tokens = handle
        .await
        .map_err(|e| anyhow!("oauth flow join error: {e}"))??;

    println!();
    println!(
        "✔ Tokens persistidos en {} (agent `{agent_id}`).",
        client.token_path().display()
    );
    println!(
        "   refresh_token: {}",
        if tokens.refresh_token.is_some() {
            "yes (persistente)"
        } else {
            "no (sólo access_token; correr de nuevo con prompt=consent)"
        }
    );
    Ok(())
}

async fn run_device_flow(client: &Arc<GoogleAuthClient>, agent_id: &str) -> Result<()> {
    let challenge = client
        .request_device_code()
        .await
        .context("requesting device code")?;

    println!();
    println!("╭─ Google OAuth consent (device-code) ──────────────────────────╮");
    println!("│  Abrí en cualquier navegador:                                  │");
    println!("│    {}", challenge.verification_url);
    println!("│  Código a escribir:  {}", challenge.user_code);
    println!("│  (válido por {}s)", challenge.expires_in);
    println!("╰────────────────────────────────────────────────────────────────╯");
    println!();
    println!("Esperando aprobación…");

    let tokens = client
        .poll_device_token(&challenge)
        .await
        .context("polling device token")?;

    println!();
    println!(
        "✔ Tokens persistidos en {} (agent `{agent_id}`).",
        client.token_path().display()
    );
    println!(
        "   refresh_token: {}",
        if tokens.refresh_token.is_some() {
            "yes (persistente)"
        } else {
            "no (sólo access_token; correr de nuevo con prompt=consent)"
        }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_print_manifest_subcommand() {
        let cli = Cli::try_parse_from(["nexo-plugin-google", "--print-manifest"]).unwrap();
        matches!(cli.command, Some(Command::PrintManifest));
    }

    #[test]
    fn parses_oauth_once_with_required_flags() {
        let cli = Cli::try_parse_from([
            "nexo-plugin-google",
            "--oauth-once",
            "agent_x",
            "--client-id-file",
            "/tmp/cid",
            "--client-secret-file",
            "/tmp/cs",
            "--token-file",
            "/tmp/tok.json",
            "--scopes",
            "gmail.readonly,drive.readonly",
            "--workspace-dir",
            "/tmp/ws",
        ])
        .unwrap();
        match cli.command {
            Some(Command::OauthOnce(args)) => {
                assert_eq!(args.agent_id, "agent_x");
                assert_eq!(args.redirect_port, 8765);
                assert!(!args.device);
                assert!(!args.remote);
            }
            other => panic!("expected OauthOnce; got {other:?}"),
        }
    }

    #[test]
    fn oauth_once_missing_required_flag_fails() {
        let result = Cli::try_parse_from([
            "nexo-plugin-google",
            "--oauth-once",
            "agent_x",
            "--client-id-file",
            "/tmp/cid",
            // missing --client-secret-file
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn device_flow_picked_when_explicit_flag() {
        let args = OauthOnceArgs {
            agent_id: "x".into(),
            client_id_file: PathBuf::from("/tmp"),
            client_secret_file: PathBuf::from("/tmp"),
            token_file: PathBuf::from("/tmp"),
            scopes: String::new(),
            redirect_port: 0,
            workspace_dir: PathBuf::from("/tmp"),
            device: true,
            remote: false,
        };
        assert!(should_use_device_flow(&args));
    }

    #[test]
    fn loopback_picked_when_no_flags_no_env() {
        // Save + clear potentially set env vars so the test is hermetic.
        let preserved = [
            std::env::var("SSH_CONNECTION").ok(),
            std::env::var("SSH_TTY").ok(),
            std::env::var("SSH_CLIENT").ok(),
        ];
        unsafe {
            std::env::remove_var("SSH_CONNECTION");
            std::env::remove_var("SSH_TTY");
            std::env::remove_var("SSH_CLIENT");
        }

        let args = OauthOnceArgs {
            agent_id: "x".into(),
            client_id_file: PathBuf::from("/tmp"),
            client_secret_file: PathBuf::from("/tmp"),
            token_file: PathBuf::from("/tmp"),
            scopes: String::new(),
            redirect_port: 0,
            workspace_dir: PathBuf::from("/tmp"),
            device: false,
            remote: false,
        };
        let picked_device = should_use_device_flow(&args);

        // Restore env vars.
        unsafe {
            if let Some(v) = &preserved[0] {
                std::env::set_var("SSH_CONNECTION", v);
            }
            if let Some(v) = &preserved[1] {
                std::env::set_var("SSH_TTY", v);
            }
            if let Some(v) = &preserved[2] {
                std::env::set_var("SSH_CLIENT", v);
            }
        }

        // If WSL2, device flow is picked even without SSH env.
        // We can't unsafely override /proc; trust the heuristic.
        if !is_wsl2() {
            assert!(!picked_device, "loopback expected when no remote markers");
        }
    }
}
