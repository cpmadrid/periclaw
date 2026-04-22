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
//! ## Subscription restarts
//!
//! Iced may tear down and rebuild the subscription whenever the
//! `ConnectParams` identity changes (URL update, token update,
//! `save_nonce` bump). Each new subscription instance calls
//! [`take_rx`], expecting a live receiver it can `await` on. If we
//! handed out the receiver once and returned `None` forever after,
//! the second subscription would run with a dead receiver — and
//! `tokio::sync::mpsc::UnboundedReceiver::recv()` on a channel with
//! all senders dropped returns `None` **immediately**. That breaks
//! every `wait_or_command` in the session loop, collapsing backoffs
//! to effectively zero and producing a reconnect-spam storm.
//!
//! The fix: [`take_rx`] always hands back a live receiver. On first
//! call it returns the original; on subsequent calls it **rebuilds**
//! the channel (new `tx` + `rx`) and atomically swaps the static
//! sender so future [`sender`] calls write into the new channel.
//! Any tx clones that the UI held from before the swap will fail
//! sends silently — callers already handle `Err` from `send`, and
//! the UI re-clones on every button dispatch anyway, so stale tx
//! clones are short-lived.

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
    /// Live sender. Swapped atomically (via Mutex) each time
    /// [`take_rx`] rebuilds the channel so new `sender()` clones
    /// always target the receiver the current session is awaiting.
    tx: Mutex<UnboundedSender<GatewayCommand>>,
    /// Live receiver, parked here until the session grabs it via
    /// [`take_rx`]. `Some` before the first take, `None` between
    /// takes (the next take rebuilds both sides of the channel).
    rx: Mutex<Option<UnboundedReceiver<GatewayCommand>>>,
}

fn channel() -> &'static Channel {
    static CHAN: OnceLock<Channel> = OnceLock::new();
    CHAN.get_or_init(|| {
        let (tx, rx) = unbounded_channel();
        Channel {
            tx: Mutex::new(tx),
            rx: Mutex::new(Some(rx)),
        }
    })
}

/// Get a cloneable sender for dispatching commands from the UI. The
/// UI re-fetches this per action rather than caching it, so after a
/// subscription restart the new sender reaches the current session
/// with no explicit refresh step.
pub fn sender() -> UnboundedSender<GatewayCommand> {
    channel()
        .tx
        .lock()
        .expect("commands tx mutex poisoned")
        .clone()
}

/// Claim a live receiver for the WS session. On first call returns
/// the original. On subsequent calls (subscription restart) rebuilds
/// the channel and returns the fresh receiver, swapping the static
/// sender so the UI's next `sender()` clone writes into the new
/// channel.
///
/// Never returns `None` — a dead receiver in the session loop
/// collapses every `wait_or_command` to zero sleep and triggers
/// reconnect-spam storms, so we always hand back something live.
pub fn take_rx() -> UnboundedReceiver<GatewayCommand> {
    let ch = channel();
    if let Some(rx) = ch.rx.lock().ok().and_then(|mut g| g.take()) {
        return rx;
    }
    // Subsequent subscription instance. Rebuild.
    let (new_tx, new_rx) = unbounded_channel();
    if let Ok(mut tx_guard) = ch.tx.lock() {
        *tx_guard = new_tx;
    }
    new_rx
}
