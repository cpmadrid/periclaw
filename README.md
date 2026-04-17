# Mission Control Desktop

Native Rust desktop companion to Mission Control — a pixel-art visualization of the OpenClaw AI agent farm running on `ubu-3xdv`. Octopus sprites in themed rooms, thought bubbles on state transitions, a real-time WS connection to the OpenClaw gateway.

> **Status:** M1 (scaffold). The window renders, the sidebar works, and the theme mirrors the web app. WS connection and sprite scene land in M2–M4. See [the plan](../../docs/plans/) for milestones.

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
./dev run --mode ssh             # real data via ssh workstation
./dev run --mode ssh --ssh-host ubu-3xdv
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

Three paths to OpenClaw state, selected via `./dev run --mode`:

| Mode | Env var set | Data source | Use case |
|---|---|---|---|
| `mock` | `OPENCLAW_MOCK=1` | `assets/test/scenario_happy.json` | Offline demo, UI iteration |
| `ssh` | `OPENCLAW_SSH_HOST=workstation` | `ssh <host> cat ~/.openclaw/cron/jobs.json` every 7s, plus `openclaw.json` every 30s | **Current real-data path.** Sub-400ms per poll over Tailscale. Channel state is optimistic (enabled == connected). |
| `ws` | (neither) | Native WebSocket handshake to `ws://host:port/` | Handshake works; scoped methods need device pairing (M3.3). Currently only `health` succeeds. |

The selector lives in `app::App::subscription`.

## Build for release

```bash
./dev build --release      # macOS (local target) — target/release/sassy-mc
./dev cross-arm64          # ubu-3xdv (aarch64-unknown-linux-gnu, uses cross if available)
```

The release build strips debug symbols automatically. Build numbers come from `Scripts/get-build-number.sh` (git commit count, overridable with `BUILD_NUMBER` env var) so CI and local stay aligned.

## Module layout

```
src/
├── main.rs             # iced::application entry
├── app.rs              # App state, Message, update, view
├── ui/
│   ├── mod.rs
│   ├── theme.rs        # OKLCH palette → iced::Color (mirror of apps/web globals.css)
│   └── sidebar.rs      # Overview / Agents / Logs / Settings nav
└── (future)
    ├── scene/          # M2: Canvas + sprites
    ├── net/            # M3: OpenClaw WS client
    └── domain/         # M2: Agent, status, room assignment
```

## Configuration

**Token storage** (from M3 onward): macOS Keychain via the `keyring` crate. First run will read `~/.openclaw/openclaw.json` → `auth.token` and migrate it. On headless Linux (ubu-3xdv wall-display mode), falls back to `$XDG_CONFIG_HOME/sassy-dog/token` (chmod 600).

**OpenClaw gateway** (from M3): `ws://100.87.202.125:18789` over Tailscale (or `ws://localhost:18789` on ubu-3xdv itself). Token auth via `Authorization: Bearer <token>` header.

## Platform notes

- **macOS:** ships with Metal via `wgpu`. No notarization needed for personal use.
- **Linux (ubu-3xdv):** requires `libxkbcommon-dev libwayland-dev libfontconfig1-dev libssl-dev mesa-vulkan-drivers`. Prefer Wayland over X11.

## Related

- [`apps/web`](../web/) — Web Mission Control dashboard (Next.js, Vercel). Desktop does **not** reuse web code.
- [`apps/agent`](../agent/) — Python telemetry agent on ubu-3xdv. Desktop connects directly to OpenClaw, independent of the agent.
- [Plan](../../docs/plans/) — monorepo-level plan documents (if present).
