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
