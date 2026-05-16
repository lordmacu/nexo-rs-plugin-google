//! Subprocess + CLI entrypoint for `nexo-plugin-google` (Phase 94).
//!
//! Dispatch rules:
//!   * `nexo-plugin-google --print-manifest` → echo bundled manifest.
//!   * `nexo-plugin-google --oauth-once <agent_id> [flags]` →
//!     run the one-shot OAuth consent flow (loopback default,
//!     `--device` switches to device-code, `--remote` auto-detects
//!     WSL2/SSH and prefers device-code).
//!   * no subcommand → boot the long-lived JSON-RPC dispatch loop
//!     against stdin/stdout (daemon-spawned mode).

fn main() -> anyhow::Result<()> {
    // Implementation lands in subsequent steps. Stub keeps the bin
    // compilable while scaffolding.
    eprintln!("nexo-plugin-google scaffold; subcommand wiring pending");
    Ok(())
}
