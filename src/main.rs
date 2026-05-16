//! Subprocess + CLI entrypoint for `nexo-plugin-google` (Phase 94).
//!
//! Dispatch:
//!   * `nexo-plugin-google --print-manifest` → echo bundled manifest.
//!   * `nexo-plugin-google --oauth-once <agent_id> [flags]` →
//!     interactive OAuth consent flow.
//!   * no subcommand → long-lived JSON-RPC dispatch loop against
//!     stdin/stdout (daemon-spawned mode).
//!
//! Configuration delivered via daemon's `plugin.configure` JSON-RPC
//! (Phase 93). Optional fallback YAML at
//! `NEXO_PLUGIN_GOOGLE_CONFIG_PATH` for diagnostic standalone use.

use std::sync::Arc;

use clap::Parser;
use nexo_broker::AnyBroker;
use nexo_microapp_sdk::plugin::{PluginAdapter, ToolInvocation, ToolInvocationError};
use serde_json::{json, Value};

use nexo_plugin_google::auto_discovery;
use nexo_plugin_google::cli::{Cli, Command};
use nexo_plugin_google::env_config::google_config_from_env;
use nexo_plugin_google::plugin::{GooglePlugin, GooglePluginConfig};
use nexo_plugin_google::runtime_handle;
use nexo_plugin_google::tools::tool_defs;

const MANIFEST: &str = include_str!("../nexo-plugin.toml");

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    // Phase 81.20.x F1 — Stage 8 cargo-install ergonomics: respond to
    // `--print-manifest` BEFORE any tracing / broker wiring so the
    // discovery walker just gets manifest bytes on stdout.
    nexo_microapp_sdk::plugin::print_manifest_if_requested(MANIFEST);

    // tracing MUST write to stderr — stdout is reserved for JSON-RPC
    // framing. Without `with_writer(io::stderr)` log lines would
    // corrupt every reply.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // rustls 0.23 requires an explicit process-wide CryptoProvider.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // CLI subcommands short-circuit before broker / adapter boot.
    let cli = Cli::parse();
    match cli.command {
        Some(Command::PrintManifest) => {
            print!("{}", MANIFEST);
            return Ok(());
        }
        Some(Command::OauthOnce(args)) => {
            return nexo_plugin_google::cli::run_oauth_once(args).await;
        }
        None => {
            // Fall through to long-lived JSON-RPC mode.
        }
    }

    let plugin = Arc::new(GooglePlugin::new());
    runtime_handle::set_runtime_handle(plugin.clone()).await;

    // Load fallback env config if NEXO_PLUGIN_GOOGLE_CONFIG_PATH is
    // set. Daemon's `plugin.configure` overrides this later.
    match google_config_from_env() {
        Ok(env_cfg) => {
            if !env_cfg.initial_configs.is_empty() {
                if let Err(e) = plugin.on_configure(env_cfg.initial_configs).await {
                    tracing::warn!(
                        target = "nexo_plugin_google",
                        error = %e,
                        "env-based on_configure failed"
                    );
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                target = "nexo_plugin_google",
                error = %e,
                "env config load failed; relying on plugin.configure"
            );
        }
    }

    // Spawn broker subscribers for admin RPC + outbound (defensive)
    // ONLY when the daemon seeded a broker URL. Stand-alone CLI uses
    // (e.g. operators running --print-manifest) skip this.
    if let Ok(broker_url) = std::env::var("NEXO_BROKER_URL") {
        if !broker_url.is_empty() {
            match boot_broker(&broker_url).await {
                Ok(broker) => spawn_auto_discovery_subscribers(broker),
                Err(e) => tracing::warn!(
                    target = "nexo_plugin_google",
                    error = %e,
                    "broker connect failed; admin RPCs disabled"
                ),
            }
        }
    }

    let plugin_for_configure = plugin.clone();
    let plugin_for_tool = plugin.clone();

    let adapter = PluginAdapter::new(MANIFEST)?
        .declare_tools(tool_defs())
        // Phase 93.4 — host delivers per-agent operator YAML.
        .on_configure(move |value: serde_yaml::Value| {
            let plugin = plugin_for_configure.clone();
            async move {
                let configs: Vec<GooglePluginConfig> = serde_yaml::from_value(value)
                    .map_err(|e| format!("invalid google plugin config: {e}"))?;
                plugin
                    .on_configure(configs)
                    .await
                    .map_err(|e| format!("on_configure: {e}"))
            }
        })
        .on_tool(move |invocation: ToolInvocation| {
            let plugin = plugin_for_tool.clone();
            async move { dispatch_tool(plugin, invocation).await }
        });

    tracing::info!(target = "nexo_plugin_google", "JSON-RPC dispatch loop ready");
    adapter.run_stdio().await?;
    Ok(())
}

async fn boot_broker(broker_url: &str) -> anyhow::Result<AnyBroker> {
    let broker_inner = nexo_config::types::broker::BrokerInner {
        kind: if broker_url.starts_with("nats://") {
            nexo_config::types::broker::BrokerKind::Nats
        } else {
            nexo_config::types::broker::BrokerKind::Local
        },
        url: broker_url.to_string(),
        auth: nexo_config::types::broker::BrokerAuthConfig::default(),
        persistence: nexo_config::types::broker::BrokerPersistenceConfig::default(),
        limits: nexo_config::types::broker::BrokerLimitsConfig::default(),
        fallback: nexo_config::types::broker::BrokerFallbackConfig::default(),
    };
    AnyBroker::from_config(&broker_inner)
        .await
        .map_err(|e| anyhow::anyhow!("broker connect failed: {e}"))
}

async fn dispatch_tool(
    plugin: Arc<GooglePlugin>,
    inv: ToolInvocation,
) -> Result<Value, ToolInvocationError> {
    let agent_id = inv
        .agent_id
        .as_deref()
        .ok_or_else(|| ToolInvocationError::ArgumentInvalid(
            "tool.invoke is missing `agent_id` (daemon must include it)".into(),
        ))?;
    match plugin
        .invoke_outbound_tool(&inv.tool_name, inv.args, agent_id)
        .await
    {
        Ok(v) => Ok(v),
        Err(e) => {
            let msg = format!("{e}");
            // Heuristic mapping of common error shapes onto the
            // -33401..-33405 typed band the daemon decodes.
            if msg.contains("unknown tool") {
                Err(ToolInvocationError::NotFound(msg))
            } else if msg.contains("not configured") {
                Err(ToolInvocationError::Unavailable(msg))
            } else if msg.contains("circuit breaker open") {
                Err(ToolInvocationError::Unavailable(msg))
            } else if msg.contains("requires `method`")
                || msg.contains("requires `url`")
                || msg.contains("only accepts")
            {
                Err(ToolInvocationError::ArgumentInvalid(msg))
            } else {
                Err(ToolInvocationError::ExecutionFailed(msg))
            }
        }
    }
}

/// Phase 81.33.b.real Stage 4 — admin RPC subscriber loop. One
/// task per request-reply topic family; per-task failure isolation
/// so one dropped subscriber doesn't kill the rest.
fn spawn_auto_discovery_subscribers(broker: AnyBroker) {
    spawn_one(broker, "plugin.google.admin.>", |_b, payload| async move {
        auto_discovery::admin_handle(&payload).await
    });
}

fn spawn_one<F, Fut>(broker: AnyBroker, topic: &'static str, handler: F)
where
    F: Fn(AnyBroker, Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Value> + Send + 'static,
{
    use nexo_broker::{BrokerHandle, Event, Message};
    tokio::spawn(async move {
        let mut sub = match broker.subscribe(topic).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    target = "google.auto_discovery",
                    topic,
                    error = %e,
                    "subscribe failed; topic will not receive requests"
                );
                return;
            }
        };
        tracing::info!(target = "google.auto_discovery", topic, "subscriber up");
        while let Some(event) = sub.next().await {
            let Ok(msg) = serde_json::from_value::<Message>(event.payload) else {
                continue;
            };
            let Some(reply_to) = msg.reply_to.clone() else {
                continue;
            };
            let reply_payload = handler(broker.clone(), msg.payload.clone()).await;
            let reply_msg = Message::new(reply_to.clone(), reply_payload);
            let reply_event = Event::new(
                reply_to.clone(),
                "google",
                match serde_json::to_value(&reply_msg) {
                    Ok(v) => v,
                    Err(_) => continue,
                },
            );
            if let Err(e) = broker.publish(&reply_to, reply_event).await {
                tracing::warn!(
                    target = "google.auto_discovery",
                    topic,
                    reply_to = %reply_to,
                    error = %e,
                    "failed to publish reply"
                );
            }
        }
        tracing::debug!(
            target = "google.auto_discovery",
            topic,
            "subscriber stream ended"
        );
    });
}

// Silence the unused-by-binary helper warning for the `json!` import
// when the main entry doesn't use it directly. Keeps the use line
// idempotent for future handler additions.
#[allow(dead_code)]
fn _ensure_json_in_scope() -> Value {
    json!({})
}
