//! Phase 94 placeholder — full client.rs port lands in step 6.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GoogleAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub token_file: String,
    #[serde(default)]
    pub redirect_port: u16,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GoogleTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SecretSources {
    pub client_id_path: PathBuf,
    pub client_secret_path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct DeviceChallenge {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Default)]
pub struct GoogleAuthClient;

pub fn canonicalize_scopes(input: &[String]) -> Vec<String> {
    input.to_vec()
}
