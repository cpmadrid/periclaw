//! Out-bound channel from the UI to the gateway WS session.
//!
//! The `net::openclaw` subscription is an Iced `stream::channel`
//! producer — it emits events into the app but has no parameter for
//! inbound commands. To let the UI trigger an RPC (e.g. resolving an
//! exec approval), we hand the WS session an unbounded receiver once
//! at construction and expose a global sender the UI can clone into
//! button handlers. Unbounded is fine here: commands are user-paced
//! (button clicks), not machine-paced.
//!
//! The channel is lazily created the first time either end touches
//! it, so start-up ordering doesn't matter.
//!
//! If the receiver has already been handed out (i.e. a second WS
//! reconnect after process-level restart of the stream), `take_rx`
//! returns `None` and the session runs without a command path — the
//! existing sender stays live so the UI doesn't crash on a click.

use std::sync::{Mutex, OnceLock};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[derive(Debug, Clone)]
pub enum GatewayCommand {
    /// Resolve an exec approval with a decision string accepted by
    /// OpenClaw's `exec.approval.resolve` RPC
    /// (`"allow-once" | "allow-always" | "deny"`).
    ResolveApproval { id: String, decision: String },
    /// Fire a cron job immediately via `cron.run`. `job_id` is the
    /// stable UUID (`jobs.json` → `id`), not the human-readable name
    /// — the RPC validates against the id.
    RunCron { job_id: String },
    /// Operator-requested reconnect — used to short-circuit the long
    /// scope-upgrade backoff after they approve the pair-request.
    /// Carries no payload; the session loop interprets it as "stop
    /// sleeping, try connecting now."
    Reconnect,
    /// Send a prompt to a specific agent's main chat session. The
    /// target `agent_id` maps to `sessionKey: "agent:<id>:main"` in
    /// `chat.send`. Reply streams back as `session.message` events
    /// keyed by the same sessionKey, so inbound routing can target
    /// the right log.
    ///
    /// `idempotency_key` is generated UI-side (one per send), not the
    /// `runId` — the gateway returns the runId in the ack payload but
    /// we don't need it; `session.message` is keyed by sessionKey.
    SendChat {
        agent_id: String,
        message: String,
        idempotency_key: String,
    },
    /// Fetch an agent's recent `chat.history` on first Chat-tab
    /// selection per connection. Lazy so switching into an agent we
    /// already hydrated doesn't re-fire the RPC; the response arrives
    /// as `WsEvent::ChatHistory { agent_id, .. }`.
    FetchChatHistory { agent_id: String },
    /// Fill in a specific agent's operator-chosen persona (name +
    /// emoji) via `agent.identity.get`. Called by the app once per
    /// agent after `agents.list` arrives — the list RPC only sees
    /// identity configured directly on the agent entry, whereas
    /// `agent.identity.get` also consults `ui.assistant` and the
    /// workspace identity file (where "Sebastian 🦀" typically lives).
    FetchAgentIdentity { agent_id: String },
    /// Reset an agent's main session via `sessions.reset`. Destroys
    /// in-memory chat history and starts a fresh session with the
    /// same key — the next prompt the operator sends lands in a
    /// clean context. `session_key` is the fully-qualified key
    /// (`agent:<id>:main`).
    ResetSession { session_key: String },
    /// Fetch `chat.history` for an **arbitrary** session (not the
    /// default `:main`). Used by the Sessions tab's drill-in detail
    /// pane. `session_key` is the fully-qualified
    /// `agent:<agentId>:<sessionId>` form as it appears in
    /// `SessionInfo.key`.
    FetchSessionHistory { session_key: String },
    /// Fetch `sessions.usage.timeseries` for the Sessions drill-in
    /// sparkline. Gateway downsamples to 200 points max, so the
    /// response is always bounded.
    FetchSessionUsage { session_key: String },
}

struct Channel {
    tx: UnboundedSender<GatewayCommand>,
    rx: Mutex<Option<UnboundedReceiver<GatewayCommand>>>,
}

fn channel() -> &'static Channel {
    static CHAN: OnceLock<Channel> = OnceLock::new();
    CHAN.get_or_init(|| {
        let (tx, rx) = unbounded_channel();
        Channel {
            tx,
            rx: Mutex::new(Some(rx)),
        }
    })
}

/// Get a cloneable sender for dispatching commands from the UI.
pub fn sender() -> UnboundedSender<GatewayCommand> {
    channel().tx.clone()
}

/// Claim the receiver. Called once by the WS session task. Returns
/// `None` on subsequent calls so we don't race two receivers against
/// one channel.
pub fn take_rx() -> Option<UnboundedReceiver<GatewayCommand>> {
    channel().rx.lock().ok().and_then(|mut g| g.take())
}
