# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## What this is

PeriClaw is a native Rust desktop app — a pixel-art "Agent Office" visualization of an OpenClaw AI agent farm. Lobster sprites in themed rooms, thought bubbles on state transitions, real-time WebSocket connection to the OpenClaw gateway. Built on **Iced 0.14** (Elm-style declarative UI, `wgpu`-rendered, `tokio` async).

## Common commands

Always go through `./dev` rather than calling `cargo` directly — it routes to the right script in `Scripts/` and sets up env vars / bundling correctly.

```bash
./dev run                        # auto mode (demo if PERICLAW_DEMO set, else ws)
./dev run --mode demo            # scripted fixture (offline)
./dev run --mode ws              # real WebSocket to OpenClaw gateway
./dev run --log both             # tee logs to console + Logs/desktop-<ts>.log
./dev run --log-level trace      # bump verbosity
./dev build --release            # stripped binary at target/release/periclaw
./dev test                       # cargo test (pass-through args, e.g. `./dev test pattern`)
./dev lint                       # cargo clippy --all-targets -- -D warnings
./dev fmt                        # cargo fmt (--check for dry-run)
./dev ci                         # full CI pipeline locally (fmt-check + clippy + test + release build)
./dev cross-arm64                # cross-compile aarch64-unknown-linux-gnu
```

`./dev run` on macOS invokes `cargo bundle --format osx` and launches the resulting `.app` so dock/Cmd-Tab/notifications attribute correctly. **`cargo build` alone does not refresh the bundle** — use `./dev run` to pick up changes when launching the bundled app.

CI requires `./dev ci` to pass on macos-14. The required check name is `fix(scene): … fmt + clippy + test`. The follow-up `build-release (macos-14)` job runs after CI and must also pass for branch-protection-style merges.

## Architecture

The whole app is a single Iced `App` (`src/app.rs`). Three things drive it:

1. **`Subscription`** (`app::App::subscription` ~line 2113) — picks the data-source stream:
   - `demo::connect` in Demo mode — replays `assets/test/scenario_happy.json`.
   - `openclaw::connect` keyed on `(gateway_url, token, save_nonce)` so URL/token changes auto-restart it.
   - Plus a tiered tick (33ms / 50ms / 500ms based on whether anything is animating) and a global event listener for window/keyboard.
2. **`Message`** enum (`app.rs:41`) — every UI event, tick, and `Message::Ws(WsEvent)` flows through `update`.
3. **`update`** (`app.rs:751`) — pattern-matches on `Message`, mutates `App` state, returns `Task` for async commands.

### Net layer (`src/net/`)

- **`openclaw.rs`** — the live WS client. Single `handle_frame` dispatcher: top-level `match frame_type` on `"event"` / `"res"` / `"ping"`; `handle_event` then matches on `"cron"` / `"agent"` / `"session.message"` / `"session.tool"` / `"sessions.changed"` / `"chat"` / `"exec.approval.*"` / `"update.available"` etc. Each arm parses its payload and emits a typed `WsEvent` on the mpsc channel. **Trace-level logs go to nowhere by default** — bump `--log-level` or temporarily promote to `info` when diagnosing missed events.
- **`events.rs`** — `WsEvent` enum (the domain-flavored events `app.rs` consumes) and `ActivityKind` (Thinking / ToolCalling / Errored / Idle). `agent_stream_to_activity` maps `tool`/`item`/`assistant`/`error` strings; `lifecycle` is handled inline in `openclaw.rs` because it needs `data.phase` to distinguish start/end/error.
- **`rpc.rs`** — `serde::Deserialize` types for OpenClaw payloads (CronJob, AgentInfo, AgentEventPayload, SessionInfo, etc.). `#[serde(default)]` and renaming are the norm — older gateway builds omit fields.
- **`commands.rs`** — outbound RPCs (SendChat, RunCron, etc.) sent via a process-wide channel.
- **`demo.rs`** — fixture-replay subscription used by `--mode demo`. Each loop covers background job work, a visible tool/message turn, a silent run, and an error recovery path so scene/UI tweaks can be verified offline.

### Domain (`src/domain/`)

- **`Agent`** — sprite-bearing roster entry (id, display, room).
- **`Job`** — the *work* model (cron, channel, etc.) — split from Agent in commit `6f3a0a4`. Don't conflate.
- **`AgentStatus`** — Unknown / Disabled / Ok / Running / Error. Status drives sprite animation state and the power-up sparkle.
- **`Room`** — Command Deck / Galley / Engine Room layout assignment for sprites.

### Scene (`src/scene/`)

- **`office.rs`** — the canvas program. `running_rooms` (a HashSet keyed off either a Running job or an agent in `AgentStatus::Running`) is the base signal for the **power-up sparkle** above each sprite, but any visible bubble for that same agent suppresses the sparkle so the bubble becomes the foreground "working" signal. *If the indicator isn't showing when you expect, first ask whether the run ever flipped to Running, then whether a bubble is intentionally hiding it.*
- **`sprite.rs`** — pixel-art atlas (lobster IDLE / WALK frames, POWER_UP overlay).
- **`thought_bubble.rs`** — short-lived bubbles with alpha-fade. `transition_text(prev, next)` picks the message ("...working...", "eureka!", "anomaly!", etc.).

### UI (`src/ui/`)

One file per nav tab (`agents_view`, `chat_view`, `sessions_view`, `logs_view`, `settings_view`, `approvals`) plus shared widgets (`chat_bubble`, `chat_input`, `sidebar`, `status_bar`, `sparkline`). Theming is OKLCH-source-of-truth in `palette.rs` → sRGB for Iced via the `palette` crate.

## Status / activity flow (high-level)

This is the bit that took multiple iterations to get right and is easy to break:

```
Operator clicks SendChat in PeriClaw
  → app.rs:840  apply_status_update(target, Running)        # immediate sparkle
  → command goes out
  → gateway emits `agent` events (stream=item|tool, etc.)
  → openclaw.rs `agent` handler → WsEvent::AgentActivity{Thinking|ToolCalling}
  → app.rs `WsEvent::AgentActivity` → apply_status_update_silent(.., Running)
  → eventually `session.message` (role=assistant) arrives
  → app.rs `WsEvent::AgentMessage` → bubble + apply_status_update_silent(.., Ok)

External Slack/Telegram/WhatsApp DM → agent
  → gateway emits `agent` events with stream=lifecycle (data.phase=start),
    then a stream of stream=assistant chunks, then lifecycle (data.phase=end)
  → `session.message` does NOT broadcast for external-channel runs
  → `lifecycle:start` → ActivityKind::Thinking → Running (sparkle on unless a bubble is already visible)
  → `assistant` chunks keep it warm
  → `lifecycle:end`  → ActivityKind::Idle → Ok (sparkle off, chat_activities cleared)
```

Two helpers govern transition-bubble noise:

- `apply_status_update` (`app.rs:1867`) — pushes a `transition_text` bubble.
- `apply_status_update_silent` — same minus the bubble. **Use this for chat-driven transitions** (AgentActivity, AgentMessage, AgentInboundUserMessage) — otherwise per-chunk "...working..."/"eureka!" stack on top of the message bubble. Errors stay loud (use the non-silent variant) because `anomaly!` is a useful surprise.

Scene priority rule: if a run is visible via a bubble (`Message`, `Tool`, `Work`, or status/error stub), the bubble wins and the sparkle stays hidden. Sparkle is reserved for silent running.

## Configuration & secrets

Resolution lives in `src/config.rs` and `src/secret_store.rs`. Read these before guessing.

- **Gateway URL**: `OPENCLAW_GATEWAY_URL` env > Settings-tab persisted value. No default.
- **Token** lookup order: `OPENCLAW_TOKEN` env → `$XDG_CONFIG_HOME/periclaw/gateway-token` (chmod 0600) → OS keychain (legacy read-only) → bootstrap from `~/.openclaw/openclaw.json` `auth.token`. Writes go to the plaintext file, not the keychain — macOS dev builds re-sign each `cargo run` and the keychain prompts for a password every launch otherwise.
- **Device identity**: Ed25519 keypair in `$XDG_CONFIG_HOME/periclaw/device-key` AND OS keychain. Auto-generated on first run; the gateway pairs via signed challenge.

The WS handshake sends an `Origin` header derived from the gateway URL — the gateway's `controlUi.allowedOrigins` must include it or the upgrade is rejected with no useful client-side error.

## Naming convention

- **`PeriClaw`** in user-visible strings (mirrors `OpenClaw`).
- **`periclaw`** lowercase for internal identifiers (crate name, binary, config dir).

## Working with the OpenClaw gateway

OpenClaw is the upstream agent runtime — sibling repo at `~/Repos/openclaw`. When investigating gateway behavior (event payload shapes, broadcast scopes, what `session.message` carries vs. `agent` events vs. `chat` events), `git pull` it first and grep the source rather than guessing. Key files:

- `src/gateway/server-chat.ts` — the `agent`/`chat`/`session.tool` broadcast emission
- `src/gateway/server-session-events.ts` — `session.message` transcript broadcasts
- `src/gateway/server-broadcast.ts` — registry of all event names + scope requirements
- `src/agents/pi-embedded-subscribe.handlers.lifecycle.ts` — `lifecycle` event shape (`data.phase` = `start | end | error`)

## Diagnosing UI-not-updating bugs

The pattern that tends to recur: the gateway *is* sending events, the WS *is* receiving frames, but some mapping silently returns `None` or filters out a role/phase, so `update` never sees the signal.

1. Run with `./dev run --log both --log-level info` (or temporarily promote a `tracing::trace!` / `tracing::debug!` to `info!` at the dispatch site).
2. Inspect `Logs/desktop-<ts>.log` for the event flow. Specifically look for `DIAG`-style or `agent message → bubble` lines.
3. If raw frames arrive but no app-side state change: the bug is in `handle_event` parsing or the `WsEvent` → `update` arm. Add a one-line `tracing::info!` with the parsed values to pinpoint.
4. If no frames arrive at all: the WS connection is silent (stale conn, wrong scope, gateway not broadcasting). Check `agents.list count=…` showed up at bootstrap and `WS connected` is logged.
