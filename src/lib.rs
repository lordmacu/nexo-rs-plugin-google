//! `nexo-plugin-google` — Google APIs tool plugin for Nexo agents.
//!
//! Phase 94 close-out of the canonical plugin-extraction lineage
//! (browser, telegram, whatsapp, email, google). Provides four
//! agent-callable tools wrapping the Google OAuth 2.0 installed-app
//! flow + authenticated `*.googleapis.com` requests:
//!
//!   * `google_auth_start`  — begin consent (returns auth URL).
//!   * `google_auth_status` — report token state.
//!   * `google_call`        — authenticated HTTP call.
//!   * `google_auth_revoke` — revoke + wipe local tokens.
//!
//! Subprocess shape: one process covers every agent with
//! `google_auth:` configured in `agents.yaml`. Per-agent state
//! (refresh_token, access_token, scopes) lives in
//! `<workspace>/<token_file>` keyed by agent_id at the daemon's
//! `plugin.configure` call.

pub mod auto_discovery;
pub mod client;
pub mod cli;
pub mod env_config;
pub mod plugin;
pub mod runtime_handle;
pub mod tools;

pub use client::{
    canonicalize_scopes, DeviceChallenge, GoogleAuthClient, GoogleAuthConfig, GoogleTokens,
    SecretSources,
};
pub use plugin::{GooglePlugin, GooglePluginConfig};

pub use cli::{Cli, Command, OauthOnceArgs};
