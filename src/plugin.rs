//! Phase 94 placeholder — `GooglePlugin` lands in step 13.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GooglePluginConfig {
    pub agent_id: String,
    pub workspace_dir: String,
    pub client_id: String,
    pub client_secret: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub token_file: String,
    #[serde(default)]
    pub redirect_port: u16,
}

#[derive(Debug, Default)]
pub struct GooglePlugin;
