# Changelog

## 0.2.1 — 2026-05-16

Fixes runtime gaps discovered after the 0.2.0 release. **0.2.0 has
been yanked**; operators must `cargo install nexo-plugin-google@0.2.1`
or later.

### Fixed

- **Plugin was unreachable from a real daemon (showstopper).** The
  0.2.0 `[plugin.config_schema]` declared per-agent `agent_id` /
  `workspace_dir` / `client_id` / `client_secret` shape, but
  daemons surface operator config at
  `<config_dir>/plugins/<plugin_id>.yaml`. Operators write
  `google-auth.yaml`, so `entries["google"]` stayed empty,
  `plugin.configure` delivered `null`, and every tool returned
  "not configured". v0.2.1 redefines the schema (`shape =
  "object"`) to mirror the legacy `google-auth.yaml` operators
  already maintain; the plugin also reads the file directly from
  `$NEXO_CONFIG_DIR/plugins/google-auth.yaml` at boot so the
  first tool call works regardless of `plugin.configure` timing.

- **Multi-account-per-agent restored.** v0.2.0 keyed the
  in-memory state by `agent_id` alone, dropping a parity feature
  from the in-tree code: one agent owning multiple Google
  accounts (e.g. personal + work). v0.2.1 keys clients by
  `account_id` and maintains an `agent → [account_id, ...]`
  lookup. Tools accept an optional `account` argument; absent
  resolves to the agent's first declared account. Admin RPCs
  also accept `account`.

- **Lazy refresh of rotated client_id / client_secret restored.**
  v0.2.0 constructed clients via `GoogleAuthClient::new`, which
  drops the `SecretSources` channel that the in-tree code used
  to pick up mtime-driven rotations. v0.2.1 calls
  `new_with_sources(...)` with the operator's file paths, so a
  fresh `chmod 600` rewrite of `secrets/*_google_client_id.txt`
  is honoured on the next tool call without restarting the
  subprocess.

- **`[plugin.credentials_schema]` opt-out.** The 0.2.0 manifest
  set `enabled = true` but the plugin shipped no
  `on_credentials_list / issue / resolve_bytes / reload`
  handlers. Daemons that probed `plugin.credentials.*` got
  `-32601 method not found`. v0.2.1 sets `enabled = false`; the
  plugin authenticates through OAuth file references inside
  `google-auth.yaml`, so the `RemoteCredentialStore` indirection
  isn't needed.

### Added

- New `[plugin.config_schema]` exposes `accounts[]` array with
  `id` + `agent_id` + `client_id_path` + `client_secret_path` +
  `token_path` + `scopes` + `redirect_port`.
- `GoogleAuthFile` + `GoogleAccount` public types.
- `account_count()` / `agent_count()` / `accounts_for_agent()`
  / `default_account_for()` / `client_by_account()` helpers on
  `GooglePlugin`.
- Admin RPCs (`oauth_status`, `oauth_revoke`) accept an
  optional `account` param.
- `admin_list_tokens()` now returns both `accounts: [...]` AND
  `agents: [...]` so admin UIs can render the multi-account
  fanout.

### Daemon-side dep

Requires the host daemon (`nexo-rs`) to seed `NEXO_CONFIG_DIR`
into every subprocess plugin's env. Shipped together in the
proyecto Phase 94 v0.2.1 wave. Older daemons fall back to
`./config/plugins/google-auth.yaml` (CWD-relative).

## 0.2.0 — 2026-05-16 — YANKED

Initial standalone-subprocess release. See README.md for the
architecture overview. **Yanked due to runtime config delivery
gap; use 0.2.1.**
