# nexo-plugin-google

Google APIs (Gmail / Calendar / Drive) tool plugin for
[`nexo-rs`](https://github.com/lordmacu/nexo-rs) agents. Subprocess
binary discovered + spawned by the daemon via
`[plugin.entrypoint]`; loaded out-of-tree so the daemon never links
the OAuth machinery directly.

Phase 94 close-out of the Phase 81 canonical plugin extraction
lineage (browser → telegram → whatsapp → email → google).

## What it ships

Four agent-callable tools — all routed through the canonical
`tool.invoke` JSON-RPC path with `agent_id` per call, so per-agent
token state is correctly partitioned inside one subprocess:

| Tool | Purpose |
|------|---------|
| `google_auth_start` | Begin OAuth consent; returns the URL the user must open. |
| `google_auth_status` | Report authenticated / expires_in_secs / has_refresh / scopes. Safe to poll. |
| `google_call` | Authenticated `{method, url, body?}` request against any `*.googleapis.com` host. Refreshes access token transparently. |
| `google_auth_revoke` | Revoke the on-file refresh_token + wipe local tokens. Forces re-auth. |

Plus admin RPCs under `nexo/admin/google/`: `oauth_status`,
`oauth_revoke`, `list_tokens`, `bot_info` — forwarded via the
broker per `[plugin.admin]`.

## Install

```bash
cargo install nexo-plugin-google
```

The binary lands at `$HOME/.cargo/bin/nexo-plugin-google`. The
daemon's discovery walker probes it with `--print-manifest` and
auto-registers without any extra configuration.

## CLI subcommands

```text
nexo-plugin-google [SUBCOMMAND]

  --print-manifest                       emit bundled manifest + exit
  --oauth-once <agent_id> [--device]     run OAuth consent flow once
                                          (setup wizard / one-shot)
  (no subcommand)                        long-lived JSON-RPC dispatch
                                          on stdin/stdout
                                          (daemon-spawned mode)
```

`--oauth-once` flags:

```text
  --client-id-file <PATH>       path to file holding the OAuth client_id
  --client-secret-file <PATH>   path to file holding the OAuth client_secret
  --token-file <PATH>           where to persist tokens on success (mode 0o600)
  --scopes <SCOPES>             comma-separated; short forms expanded
  --workspace-dir <PATH>        workspace root; relative token_file resolves here
  --redirect-port <PORT>        loopback callback port; default 8765
  --device                      use device-code flow (no local browser)
  --remote                      auto-detect SSH/WSL2 + prefer device-code
```

## Architecture

- **Single subprocess covers every agent with `google_auth:` configured.**
  Per-agent state lives in
  `DashMap<agent_id, Arc<GoogleAuthClient>>` inside the process,
  keyed by the `agent_id` the daemon includes in each
  `tool.invoke` call. Token files persist at
  `<workspace>/<token_file>` (default `google_tokens.json`).
- **Config delivered via daemon's `plugin.configure` JSON-RPC**
  (Phase 93 opaque-config contract). The plugin's manifest
  `[plugin.config_schema] shape = "array"` declares the per-agent
  shape: `agent_id`, `workspace_dir`, `client_id`, `client_secret`,
  `scopes`, `token_file`, `redirect_port`.
- **`[plugin.extends].tools` declares the four `google_*` names**,
  and the manifest's `[plugin.tools].expose` namespace-allowlists
  them per the per-plugin tool-namespace rule. Schemas ship at
  handshake via the initialize-reply (`PluginAdapter::declare_tools`).

## OAuth flow

Two consent paths share `GoogleAuthClient`:

1. **Loopback** (default `--oauth-once`): bind `127.0.0.1:<port>`,
   print the consent URL, block on the redirect, exchange the
   code for tokens, persist.
2. **Device-code** (`--oauth-once --device` or `--remote` heuristic):
   POST to `oauth2.googleapis.com/device/code`, print the
   `user_code` + `verification_url`, poll until approval.

Either way the resulting tokens land in the workspace at
`google_tokens.json` (mode 0o600 on Unix); subsequent
`google_call` invocations transparently refresh the access token
when it's within 60s of expiring.

## Manifest sections

- `[plugin]` — id, version, name, description, min_nexo_version.
- `[plugin.entrypoint]` — `command = "nexo-plugin-google"`.
- `[plugin.requires]` — `nexo_capabilities = ["broker"]`.
- `[plugin.capabilities.broker]` — subscribe allowlist:
  `plugin.outbound.google[.>]`, `plugin.google.admin.>`.
- `[plugin.tools]` — `expose` namespace.
- `[plugin.extends]` — `tools` lifted into daemon's
  `RemoteToolHandler` registry.
- `[plugin.admin]` — admin RPC prefix.
- `[plugin.config_schema]` — `shape = "array"`, per-agent shape.
- `[plugin.credentials_schema]` — `enabled = true`, accounts shape array.
- `[plugin.dashboard]` — workspace_walk subdir, auth_check probes
  `google_tokens.json`.

## License

Dual-licensed under MIT OR Apache-2.0. See `LICENSE-MIT` and
`LICENSE-APACHE`.

## Status

Phase 94 shipped 2026-05-16. 10/11 sub-phases ✅; the 11th
(this publish) ships with 0.2.0. Open follow-ups: PKCE migration,
multi-account-per-tenant shape, `[plugin.http]` route mount,
`[plugin.metrics]` Prometheus scrape, in-tree
`crates/plugins/google/` deletion (blocked on `nexo-poller`
migration to the published lib).

Source: [github.com/lordmacu/nexo-rs-plugin-google](https://github.com/lordmacu/nexo-rs-plugin-google).
