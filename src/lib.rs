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
//! Multi-instance × multi-account: one subprocess holds N
//! `Arc<GoogleAuthClient>`s keyed by account id (operator-chosen,
//! conventionally an email address). Each account binds to an
//! `agent_id`; an agent MAY own multiple accounts (default + work,
//! etc.). Tools accept an optional `account` arg; absent → the
//! agent's first account.

pub mod auto_discovery;
pub mod cli;
pub mod client;
pub mod env_config;
pub mod plugin;
pub mod runtime_handle;
pub mod tools;

pub use client::{
    canonicalize_scopes, DeviceChallenge, GoogleAuthClient, GoogleAuthConfig, GoogleTokens,
    SecretSources,
};
pub use plugin::{GoogleAccount, GoogleAuthFile, GooglePlugin};

pub use cli::{Cli, Command, OauthOnceArgs};
