# Mission Control Desktop

Native Rust desktop companion to Mission Control — a pixel-art visualization of the OpenClaw AI agent farm running on `ubu-3xdv`. Octopus sprites in themed rooms, thought bubbles on state transitions, a real-time WS connection to the OpenClaw gateway.

> **Status:** in active use. Live WebSocket connection with Ed25519 device pairing, pixel-art office scene with sprite animations, multi-agent chat, Agents / Sessions / Logs nav tabs, approval flow, operator-driven cron runs and session resets. Persistent UI state across restarts.

## Stack

- **Framework:** [Iced 0.14](https://iced.rs) — Declarative/Elm, `wgpu`-rendered
- **Async runtime:** tokio (via Iced's `tokio` feature)
- **Color conversion:** `palette` (OKLCH from `apps/web/src/app/globals.css` → sRGB for Iced)
- **Logging:** `tracing` + `tracing-subscriber`

## Run

Matches the house `./dev` pattern used across Sassy Dog (lupita, velovate, tailoredtip, quickshot). Everything activity-first, colored output, cargo underneath.

```bash
./dev run                        # auto mode (uses env vars as-is), debug build
./dev run --mode mock            # scripted fixture (offline demo)
./dev run --mode ws              # real data via native WebSocket
./dev run --release              # release-optimized
./dev build --release            # stripped release binary at target/release/sassy-mc
./dev test                       # cargo test
./dev lint                       # cargo clippy -- -D warnings
./dev fmt                        # cargo fmt (--check for dry-run)
./dev ci                         # full CI pipeline locally (fmt + clippy + test + release build)
./dev cross-arm64                # build for ubu-3xdv (aarch64-linux-gnu)
./dev version                    # app version, build number, git sha
./dev help                       # usage
```

Activity implementations are thin scripts under `Scripts/`. `Scripts/lib.sh` + `Scripts/config.sh` mirror Lupita's pattern so new contributors coming from another Sassy Dog app land in familiar ground.

### Data source modes

Two paths to OpenClaw state, selected via `./dev run --mode`:

| Mode | Env var set | Data source | Use case |
|---|---|---|---|
| `mock` | `OPENCLAW_MOCK=1` | `assets/test/scenario_happy.json` | Offline demo, UI iteration |
| `ws` | (unset) | Native WebSocket to `wss://ubu-3xdv.tail4fb3a4.ts.net/` via Tailscale Serve (override with `OPENCLAW_GATEWAY_URL`) | **Current real-data path.** Full push-event stream; Ed25519 device pairing handled automatically — first run files a pair-request the operator approves with `openclaw devices approve <id>`. |

`auto` (the default when `--mode` is omitted) picks `mock` if `OPENCLAW_MOCK` is already set in the environment, otherwise `ws`. The selector lives in `app::App::subscription`.

## Build for release

```bash
./dev build --release      # macOS (local target) — target/release/sassy-mc
./dev cross-arm64          # ubu-3xdv (aarch64-unknown-linux-gnu, uses cross if available)
```

The release build strips debug symbols automatically. Build numbers come from `Scripts/get-build-number.sh` (git commit count, overridable with `BUILD_NUMBER` env var) so CI and local stay aligned.

## Module layout

```
src/
├── main.rs             # iced::application entry — loads UiState, sets window
├── app.rs              # App state, Message enum, update, view, subscription
├── config.rs           # Gateway URL + token lookup (env → file → keychain → ~/.openclaw)
├── device_identity.rs  # Ed25519 keypair + signed connect challenge
├── ui_state.rs         # Persistent UI state (selected tab/agent/session, window size)
├── logs.rs             # Severity classification + log buffer for the Logs tab
├── domain/             # Agent, status, room assignment
├── scene/              # Canvas program, sprite atlas, thought bubbles
├── net/                # WS client (openclaw.rs), commands, events, rpc types, mock
└── ui/                 # Per-tab views, shared widgets (chat_bubble, chat_input,
                        # sidebar, status_bar, approvals) and OKLCH theme
```

## Configuration

**Gateway URL**: defaults to `wss://ubu-3xdv.tail4fb3a4.ts.net/` — Tailscale Serve terminates TLS and injects whois headers the gateway trusts (`allowTailscale: true`), so no client-side token is required in the default path. Override with `OPENCLAW_GATEWAY_URL` to reach a raw `ws://host:port/` endpoint (which does require a token).

**Device pairing**: Ed25519 keypair generated on first run and stored in the OS keychain (macOS Keychain / Linux Secret Service / Windows Credential Manager). The client signs a challenge during the gateway `connect` handshake; scope upgrades file a pair-request the operator approves with `openclaw devices approve <id>`.

**Token lookup** (when running against a non-Tailscale endpoint), in order:
1. `OPENCLAW_TOKEN` env var (the Doppler-injected path).
2. `$XDG_CONFIG_HOME/sassy-dog/gateway-token` (chmod 0600) — the on-disk fallback we write to.
3. OS keychain — legacy read-only path, kept so installs from older builds keep working.
4. Bootstrap from `~/.openclaw/openclaw.json` → `auth.token`, then mirror into (2).

Writes land on the plaintext file, not the keychain — on macOS dev builds the binary signature changes each `cargo run`, which triggers a login-password prompt every launch when writing to the keychain. See `src/config.rs` for the full resolution flow.

## Platform notes

- **macOS:** ships with Metal via `wgpu`. No notarization needed for personal use.
- **Linux (ubu-3xdv):** requires `libxkbcommon-dev libwayland-dev libfontconfig1-dev libssl-dev mesa-vulkan-drivers`. Prefer Wayland over X11.

## Related

- [`apps/web`](../web/) — Web Mission Control dashboard (Next.js, Vercel). Desktop does **not** reuse web code.
- [`apps/agent`](../agent/) — Python telemetry agent on ubu-3xdv. Desktop connects directly to OpenClaw, independent of the agent.
- [Plan](../../docs/plans/) — monorepo-level plan documents (if present).
