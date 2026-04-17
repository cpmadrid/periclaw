# Mission Control Desktop

Native Rust desktop companion to Mission Control — a pixel-art visualization of the OpenClaw AI agent farm running on `ubu-3xdv`. Octopus sprites in themed rooms, thought bubbles on state transitions, a real-time WS connection to the OpenClaw gateway.

> **Status:** M1 (scaffold). The window renders, the sidebar works, and the theme mirrors the web app. WS connection and sprite scene land in M2–M4. See [the plan](../../docs/plans/) for milestones.

## Stack

- **Framework:** [Iced 0.14](https://iced.rs) — Declarative/Elm, `wgpu`-rendered
- **Async runtime:** tokio (via Iced's `tokio` feature)
- **Color conversion:** `palette` (OKLCH from `apps/web/src/app/globals.css` → sRGB for Iced)
- **Logging:** `tracing` + `tracing-subscriber`

## Run

From this directory:

```bash
just dev        # RUST_LOG=sassy_mc=debug cargo run
just mock       # OPENCLAW_MOCK=1 cargo run (no WS to ubu-3xdv)
just check      # cargo check --all-targets
just fmt        # cargo fmt
just clippy     # cargo clippy --all-targets -- -D warnings
```

Or directly:

```bash
cargo run
```

## Build for release

```bash
just build-release          # macOS (local target)
just build-arm64            # ubu-3xdv (aarch64-unknown-linux-gnu, needs `cross`)
```

Release binary: `target/release/sassy-mc`.

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
