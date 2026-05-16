//! Process-wide handle to the active [`crate::plugin::GooglePlugin`].
//!
//! Mirrors `nexo-plugin-email::runtime_handle` so auto-discovery
//! broker handlers can reach the plugin's state without taking
//! ownership at module-load time. Set once at `main` boot via
//! [`set_runtime_handle`]; readers in `auto_discovery` await the
//! `RwLock` whenever a broker request lands.

use std::sync::Arc;

use once_cell::sync::Lazy;
use tokio::sync::RwLock;

use crate::plugin::GooglePlugin;

static HANDLE: Lazy<RwLock<Option<Arc<GooglePlugin>>>> =
    Lazy::new(|| RwLock::new(None));

/// Reader handle — returned `None` while boot is in flight.
pub fn runtime_handle() -> &'static RwLock<Option<Arc<GooglePlugin>>> {
    &HANDLE
}

/// Populate the handle. Called from `main` after the plugin's
/// `on_configure` has built the initial state.
pub async fn set_runtime_handle(plugin: Arc<GooglePlugin>) {
    *HANDLE.write().await = Some(plugin);
}
