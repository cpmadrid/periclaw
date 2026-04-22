//! Real WebSocket client for the OpenClaw gateway.
//!
//! Drives the UI from **push events**, not polling. The gateway already
//! broadcasts a rich set of scope-free events (`cron`, `chat`, `agent`,
//! `tick`, `health`) that the client consumes directly; scoped events
//! (`sessions.changed`, `session.message`, `session.tool`,
//! `exec.approval.*`) also arrive when `operator.read` scope is granted
//! (see below).
//!
//! ## Bootstrap vs. live state
//!
//! - Push events populate state **incrementally** as things happen.
//! - Immediately after `hello-ok` we fire `cron.status` +
//!   `channels.status` once to get an initial snapshot. These are
//!   scope-gated and will fail with `missing scope: operator.read` if
//!   the gateway hasn't granted read scope to this connection.
//! - `channels.status` is re-fired every 30s as a heartbeat since
//!   OpenClaw does not yet broadcast channel state changes.
//!
//! ## Scopes
//!
//! Bearer-token-only auth (no device pairing) is denied `operator.read`
//! by default. To unlock scoped events and RPCs in dev, set
//! `gateway.controlUi.dangerouslyDisableDeviceAuth: true` in the
//! OpenClaw config and restart the gateway. `hello-ok` payload includes
//! the granted scopes — we log them at connect time so it's obvious
//! which mode we're in.
//!
//! ## Protocol
//!
//! Verified against openclaw source
//! (`openclaw/src/gateway/server/ws-connection.ts`,
//! `openclaw/src/gateway/client.ts`):
//!
//! 1. WS upgrade to `ws://host:port/` — **root path**. NOT
//!    `/__openclaw__/ws` (that's the canvas-host server which
//!    intercepts WS upgrades first and runs a different protocol).
//!    `Authorization: Bearer <token>` header is required.
//! 2. Server sends `{type:"event", event:"connect.challenge",
//!    payload:{nonce}}` before accepting any other frames.
//! 3. Client sends `{type:"req", id, method:"connect", params:{...}}`.
//! 4. Server replies `{type:"res", id, ok:true,
//!    payload:{type:"hello-ok", server, features, snapshot, policy,
//!    auth}}`.
//! 5. After hello-ok, RPC envelopes and event frames flow in both
//!    directions indefinitely.
//!
//! On any failure the session errors, emits `WsEvent::Disconnected`,
//! sleeps with exponential backoff capped at 30s, and retries.

use std::pin::Pin;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iced::futures::Stream;
use iced::stream;
use serde_json::{Value, json};
use tokio::time::{Instant, sleep_until, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

use crate::config;
use crate::device_identity::{DeviceIdentity, SignConnectParams};
use crate::domain::AgentId;
use crate::net::WsEvent;
use crate::net::commands::{self, GatewayCommand};
use crate::net::events::{
    ActivityKind, GatewayUpdate, agent_stream_to_activity, cron_job_from_event,
};
use crate::net::rpc::{
    AgentEventPayload, AgentInfo, ApprovalEventPayload, Channel, CronEventPayload, CronJob,
    LogTailPayload, MainAgent, SessionInfo,
};

/// Cadence of the channel-status heartbeat RPC. Channels are not yet
/// broadcast as push events on the gateway, so we refresh via the
/// already-open socket on a slow interval.
const CHANNEL_HEARTBEAT: Duration = Duration::from_secs(30);
/// Cadence of the `logs.tail` poll that drives the Logs nav tab.
/// Fast enough to feel live (new lines land in ≤3s), slow enough to
/// keep the bandwidth trivial (each call returns only new lines
/// since the stored cursor).
const LOG_TAIL_INTERVAL: Duration = Duration::from_secs(3);
const INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Wait while the operator approves a pending scope-upgrade
/// pair-request. The gateway mints a fresh requestId on every
/// reconnect attempt — if we retry on a short cycle the id shown
/// in the UI goes stale before the operator can copy and run the
/// approve command. 5 minutes is comfortably longer than the
/// "copy, ssh in, paste" round-trip; the operator can also just
/// relaunch the desktop to force an immediate retry once approved.
const SCOPE_UPGRADE_BACKOFF: Duration = Duration::from_secs(300);

/// Milliseconds since the UNIX epoch — the format `signedAtMs` in
/// the device-auth payload expects. Falls back to zero if the system
/// clock is somehow behind the epoch (it isn't, but the cast is safer).
fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The agent we attribute chat/agent events to. Today the roster has
/// exactly one LLM agent (`main`), so all chat text is necessarily
/// from it. When the desktop grows a multi-agent roster this should
/// route by the event's `sessionKey` instead.
const CHAT_AGENT_ID: &str = "main";

/// Stable, hashable bundle that [`connect`] takes as its subscription
/// input. Its `Hash` impl contributes to the subscription identity,
/// so changing any field tears down the current WS session and
/// starts a fresh one. `save_nonce` is bumped on every Settings Save
/// so re-saving unchanged values still restarts the subscription —
/// otherwise the operator gets no feedback when they click Save on
/// values that were already correct.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ConnectParams {
    pub gateway_url: String,
    pub token: Option<String>,
    pub save_nonce: u64,
}

/// Iced subscription stream for the real gateway.
///
/// Called via `Subscription::run_with(params, openclaw::connect)` so
/// the `params` argument participates in subscription identity. The
/// return type is a boxed `Pin<Box<dyn Stream + Send>>` (not
/// `impl Stream`) because `Subscription::run_with` takes a fn pointer
/// and fn-pointer coercion requires a nameable return type.
pub fn connect(params: &ConnectParams) -> Pin<Box<dyn Stream<Item = WsEvent> + Send>> {
    let gateway_url = params.gateway_url.clone();
    let token = params.token.clone();
    Box::pin(stream::channel(64, async move |mut out| {
        let instance_id = config::instance_id().unwrap_or_else(|e| {
            tracing::warn!(error = %e, "instance-id stash failed; using ephemeral");
            uuid::Uuid::new_v4().to_string()
        });

        // Load (or mint on first run) the Ed25519 device identity
        // used to sign the connect challenge. Pairing is driven via
        // Control UI — this crate just presents the public key and
        // signs the nonce.
        let device = match DeviceIdentity::load_or_create() {
            Ok(d) => Some(d),
            Err(e) => {
                tracing::warn!(error = %e, "device identity unavailable; connect will rely on bypass flag");
                None
            }
        };

        // Claim the UI→WS command receiver. Single owner — subsequent
        // reconnects reuse the same receiver across session attempts.
        // If someone already took it (shouldn't happen on a fresh
        // process) we substitute a dangling receiver so the session
        // loop's select arm has something to await without panicking.
        let mut cmd_rx = commands::take_rx().unwrap_or_else(|| {
            tracing::warn!("command receiver already claimed; UI → WS commands will no-op");
            let (_dangling_tx, rx) = tokio::sync::mpsc::unbounded_channel();
            rx
        });

        let mut backoff = INITIAL_BACKOFF;

        loop {
            match session(
                token.as_deref(),
                &gateway_url,
                &instance_id,
                device.as_ref(),
                &mut cmd_rx,
                &mut out,
            )
            .await
            {
                Ok(()) => {
                    tracing::warn!("session ended cleanly (unexpected); reconnecting");
                    backoff = INITIAL_BACKOFF;
                }
                Err(SessionError::ScopeUpgradePending { request_id }) => {
                    tracing::warn!(
                        %request_id,
                        "scope-upgrade pairing required — waiting for operator approval",
                    );
                    let _ = out
                        .send(WsEvent::ScopeUpgradePending(Some(request_id.clone())))
                        .await;
                    let _ = out
                        .send(WsEvent::Disconnected(format!(
                            "awaiting scope-upgrade approval ({request_id})"
                        )))
                        .await;
                    // Hot-reconnecting re-acknowledges the same
                    // requestId without progress and spams logs. Wait
                    // a long human-paced interval — the operator has
                    // to go run `openclaw devices approve <id>`. Any
                    // inbound command (practically: the "Retry now"
                    // button sending `Reconnect`) short-circuits the
                    // wait so approvals take effect immediately.
                    wait_or_command(SCOPE_UPGRADE_BACKOFF, &mut cmd_rx).await;
                    continue;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "session errored; reconnecting");
                    let _ = out.send(WsEvent::Disconnected(e.to_string())).await;
                }
            }

            wait_or_command(backoff, &mut cmd_rx).await;
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }))
}

/// Map a `ws://` / `wss://` gateway URL onto the matching HTTP origin
/// (`http://` / `https://`, scheme + authority). Returns `None` for
/// URLs that aren't WebSocket — the caller falls back to letting the
/// gateway reject the handshake rather than sending a garbage Origin.
fn derive_origin(ws_url: &str) -> Option<String> {
    let (scheme, rest) = if let Some(r) = ws_url.strip_prefix("wss://") {
        ("https", r)
    } else if let Some(r) = ws_url.strip_prefix("ws://") {
        ("http", r)
    } else {
        return None;
    };
    let authority = rest.split_once('/').map(|(a, _)| a).unwrap_or(rest);
    if authority.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{authority}"))
}

#[derive(Debug, thiserror::Error)]
enum SessionError {
    #[error("connect {0}")]
    Connect(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("handshake rejected: {0}")]
    HandshakeRejected(String),
    /// Gateway filed a pair-request to expand scopes; operator must
    /// approve via `openclaw devices approve <request_id>` before
    /// this connection can succeed. Carried separately so the
    /// reconnect loop can back off far longer than a transient
    /// network failure.
    #[error("scope-upgrade pairing required (request {request_id})")]
    ScopeUpgradePending { request_id: String },
    #[error("handshake timeout")]
    HandshakeTimeout,
    #[error("socket closed")]
    SocketClosed,
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("send: {0}")]
    Send(String),
}

async fn session(
    token: Option<&str>,
    gateway_url: &str,
    instance_id: &str,
    device: Option<&DeviceIdentity>,
    cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<GatewayCommand>,
    out: &mut iced::futures::channel::mpsc::Sender<WsEvent>,
) -> Result<(), SessionError> {
    tracing::info!(
        url = gateway_url,
        auth = if token.is_some() {
            "token"
        } else {
            "tailscale"
        },
        device_id = device.map(|d| d.device_id.as_str()).unwrap_or("none"),
        "connecting to gateway",
    );
    let mut req = gateway_url
        .into_client_request()
        .map_err(SessionError::Connect)?;
    if let Some(tok) = token {
        let auth_header = HeaderValue::from_str(&format!("Bearer {tok}"))
            .map_err(|e| SessionError::Send(e.to_string()))?;
        req.headers_mut().insert("Authorization", auth_header);
    }
    // Gateway enforces `controlUi.allowedOrigins` on WS upgrade.
    // Derive the Origin from the gateway URL itself so the user's
    // allowedOrigins only needs to list the gateway's own hostname.
    if let Some(origin) = derive_origin(gateway_url) {
        let origin_header = HeaderValue::from_str(&origin)
            .map_err(|e| SessionError::Send(format!("invalid origin: {e}")))?;
        req.headers_mut().insert("Origin", origin_header);
    }
    let (mut socket, _resp) = connect_async(req).await?;

    // Step 1: wait for the server-initiated `connect.challenge` event before
    // sending our connect request. The gateway refuses to process `connect`
    // until the client has acknowledged the challenge (verified against the
    // reference client in openclaw/dist/client-*.js).
    let handshake_deadline = Instant::now() + Duration::from_secs(10);
    let nonce: String = loop {
        tokio::select! {
            _ = sleep_until(handshake_deadline) => {
                return Err(SessionError::HandshakeTimeout);
            }
            msg = socket.next() => {
                match msg {
                    Some(Ok(WsMsg::Text(txt))) => {
                        tracing::debug!(raw = %txt, "pre-connect frame");
                        let v: Value = serde_json::from_str(&txt)?;
                        let is_event = v.get("type") == Some(&Value::String("event".into()));
                        let is_challenge = v.get("event")
                            == Some(&Value::String("connect.challenge".into()));
                        if is_event && is_challenge {
                            let nonce = v.get("payload")
                                .and_then(|p| p.get("nonce"))
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            if nonce.is_empty() {
                                return Err(SessionError::HandshakeRejected(
                                    "challenge missing nonce".into(),
                                ));
                            }
                            tracing::debug!("received connect.challenge");
                            break nonce;
                        }
                    }
                    Some(Ok(WsMsg::Close(frame))) => {
                        return Err(SessionError::HandshakeRejected(
                            frame.map(|f| f.reason.to_string())
                                .unwrap_or_else(|| "closed before challenge".into()),
                        ));
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(SessionError::Connect(e)),
                    None => return Err(SessionError::SocketClosed),
                }
            }
        }
    };

    // Step 2: send our connect request with a signed device identity.
    // `openclaw-tui` + `operator` + `operator.read` is the same shape
    // the Control UI uses; including a paired device identity means
    // the `dangerouslyDisableDeviceAuth` bypass flag is no longer
    // required — proper Ed25519 challenge-response covers the gate.
    let client_id = "openclaw-tui";
    let client_mode = "ui";
    let role = "operator";
    // read (events + snapshots), approvals (Allow/Deny resolves
    // actually land), admin (firing `cron.run` from the UI). On a
    // device not yet paired to the richer scope set the gateway
    // returns `NOT_PAIRED` with `reason: "scope-upgrade"` — handled
    // below with a long backoff + visible notice in the UI.
    let scopes: &[&str] = &["operator.read", "operator.approvals", "operator.admin"];
    let mut params_obj = json!({
        "minProtocol": 3,
        "maxProtocol": 3,
        "client": {
            "id": client_id,
            "displayName": "Periclaw",
            "version": env!("CARGO_PKG_VERSION"),
            "platform": std::env::consts::OS,
            "mode": client_mode,
            "instanceId": instance_id,
        },
        "role": role,
        "scopes": scopes,
        "caps": [],
    });
    if let Some(tok) = token {
        params_obj["auth"] = json!({ "token": tok });
    }
    if let Some(dev) = device {
        // Sign the gateway's challenge nonce so the server can verify
        // this connection against the paired device record. Matches
        // `openclaw/ui/src/ui/gateway.ts:buildGatewayConnectDevice`.
        let signed = dev.sign_connect(SignConnectParams {
            client_id,
            client_mode,
            role,
            scopes,
            token,
            nonce: &nonce,
            signed_at_ms: chrono_now_ms(),
        });
        // Schema: `frames.ts:43` — `{id, publicKey, signature, signedAt, nonce}`
        // with `additionalProperties: false`. `signedAt` is the same ms value
        // that went into the v2 payload; the gateway echoes the nonce into
        // its verification payload, so we must pass it here too.
        params_obj["device"] = json!({
            "id": dev.device_id,
            "publicKey": dev.public_key_base64url(),
            "signature": signed.signature_base64url,
            "signedAt": signed.signed_at_ms,
            "nonce": nonce,
        });
    }
    let connect_frame = json!({
        "type": "req",
        "id": "connect-1",
        "method": "connect",
        "params": params_obj,
    });

    socket
        .send(WsMsg::Text(connect_frame.to_string().into()))
        .await
        .map_err(|e| SessionError::Send(e.to_string()))?;

    // Step 3: await the hello-ok response. Envelope is
    // `{type:"res", id:"connect-1", ok:true, payload:{type:"hello-ok", ...}}`.
    let hello_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        tokio::select! {
            _ = sleep_until(hello_deadline) => {
                return Err(SessionError::HandshakeTimeout);
            }
            msg = socket.next() => {
                match msg {
                    Some(Ok(WsMsg::Text(txt))) => {
                        tracing::debug!(raw = %txt, "handshake frame");
                        let v: Value = serde_json::from_str(&txt)?;
                        let is_res = v.get("type") == Some(&Value::String("res".into()));
                        let id_match = matches!(
                            v.get("id"),
                            Some(Value::String(s)) if s == "connect-1"
                        );
                        if is_res && id_match {
                            let ok = v.get("ok") == Some(&Value::Bool(true));
                            let payload_type = v.get("payload")
                                .and_then(|p| p.get("type"))
                                .and_then(Value::as_str);
                            if ok && payload_type == Some("hello-ok") {
                                let payload = v.get("payload");
                                let conn_id = payload
                                    .and_then(|p| p.get("server"))
                                    .and_then(|s| s.get("connId"))
                                    .and_then(Value::as_str)
                                    .unwrap_or("?");
                                // `auth.scopes` reveals what the gateway
                                // granted us. If `operator.read` is
                                // missing, scoped RPCs and `session.*`
                                // events won't arrive — flag loudly.
                                let granted = payload
                                    .and_then(|p| p.get("auth"))
                                    .and_then(|a| a.get("scopes"))
                                    .and_then(Value::as_array)
                                    .map(|a| a.iter()
                                        .filter_map(Value::as_str)
                                        .map(str::to_string)
                                        .collect::<Vec<_>>())
                                    .unwrap_or_default();
                                tracing::info!(
                                    conn_id,
                                    scopes = ?granted,
                                    "gateway connect accepted",
                                );
                                // Hand a successful connect to the UI
                                // and clear any lingering scope-upgrade
                                // notice.
                                let _ = out.send(WsEvent::ScopeUpgradePending(None)).await;
                                let _ = out.send(WsEvent::Connected).await;
                                break;
                            }
                            // Scope-upgrade pair requests are distinct
                            // from generic handshake failures: the
                            // gateway will keep rejecting us until the
                            // operator approves the pair-request, so
                            // fast reconnection is pointless and would
                            // flood logs. Tag the error variant so the
                            // outer loop backs off long and the UI can
                            // show the requestId.
                            let details = v
                                .get("error")
                                .and_then(|e| e.get("details"));
                            let err_code = details
                                .and_then(|d| d.get("code"))
                                .and_then(Value::as_str);
                            let reason = details
                                .and_then(|d| d.get("reason"))
                                .and_then(Value::as_str);
                            let request_id = details
                                .and_then(|d| d.get("requestId"))
                                .and_then(Value::as_str);
                            if err_code == Some("PAIRING_REQUIRED")
                                && reason == Some("scope-upgrade")
                                && let Some(rid) = request_id
                            {
                                return Err(SessionError::ScopeUpgradePending {
                                    request_id: rid.to_string(),
                                });
                            }
                            return Err(SessionError::HandshakeRejected(txt.to_string()));
                        }
                    }
                    Some(Ok(WsMsg::Close(frame))) => {
                        return Err(SessionError::HandshakeRejected(
                            frame.map(|f| f.reason.to_string())
                                .unwrap_or_else(|| "closed during handshake".into()),
                        ));
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(SessionError::Connect(e)),
                    None => return Err(SessionError::SocketClosed),
                }
            }
        }
    }

    // Bootstrap: one-shot cron + channels snapshot so the UI has
    // initial state without waiting for an event to fire. `cron.list`
    // (not `cron.status`) gives us the full `{id, name, state}` rows;
    // we need `id` to translate incoming `cron` events (UUID-keyed)
    // back to roster names.
    let mut rpc_id: u64 = 100;
    rpc_id += 1;
    send_rpc(&mut socket, rpc_id, "cron.list").await?;
    rpc_id += 1;
    send_rpc(&mut socket, rpc_id, "channels.status").await?;
    // Subscribe to session events so `session.message` (text from
    // agent turns routed through external channels like Slack) and
    // `sessions.changed` are delivered to this connection. Without
    // subscribing, OpenClaw suppresses chat broadcasts for
    // non-internal channels (server-chat.ts:590 `isControlUiVisible`
    // gate), so Slack/Telegram conversations wouldn't reach the UI.
    rpc_id += 1;
    send_rpc(&mut socket, rpc_id, "sessions.subscribe").await?;
    // Initial session-list fetch so the status bar has token-usage
    // numbers immediately; subsequent session.message events keep it
    // in sync as the main session grows.
    rpc_id += 1;
    send_rpc(&mut socket, rpc_id, "sessions.list").await?;
    // Discover every chat-capable agent in one call — id, identity
    // (name/emoji), model, workspace. Replaces the older
    // `agent.identity.get {agentId: "main"}` bootstrap: that one only
    // fetched the single default agent, which made a multi-agent
    // picker impossible. `agents.list` (see
    // `openclaw/src/gateway/server-methods/agents.ts:427`) returns the
    // full set plus a `defaultId` pointing at the agent the desktop
    // should select first.
    rpc_id += 1;
    send_rpc(&mut socket, rpc_id, "agents.list").await?;
    // Note: `chat.history` is NOT fired at bootstrap anymore — the
    // app selects the default agent from the agents.list response and
    // then dispatches `GatewayCommand::FetchChatHistory` for it, which
    // routes through the same command channel as SendChat. That keeps
    // bootstrap cheap when an install has many agents and avoids an
    // unnecessary round-trip for agents the operator never opens.

    // UUID → human-readable cron name cache, populated from the
    // snapshot and consulted when cron events arrive. Empty until
    // cron.list returns (a few ms after hello-ok), so the very first
    // event after startup may still go through as UUID.
    let mut cron_id_to_name: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    // Recently-rendered session.message ids. OpenClaw emits two
    // updates per assistant turn (initial insert + metadata attach),
    // both carrying the same messageId — dedup client-side so the
    // bubble only spawns once. Bounded so it can't grow unbounded
    // on a long session.
    let mut seen_message_ids: std::collections::VecDeque<String> =
        std::collections::VecDeque::with_capacity(32);

    // In-flight `chat.history` RPC ids → agent ids. chat.history's
    // response payload doesn't echo the sessionKey, so we remember
    // the agent we're asking about at send time and tag the
    // corresponding `ChatHistory` event when the res comes back.
    let mut pending_history: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    // Parallel map for arbitrary session-keyed history fetches (the
    // Sessions tab drill-in). Kept separate from `pending_history`
    // because the routed events are different — the app stores
    // agent-main history in `chat_logs` and session-specific history
    // in `session_transcripts`.
    let mut pending_session_history: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    // Sessions drill-in token-usage sparkline. `sessions.usage.
    // timeseries` responses carry `sessionId` but not the full
    // session key we asked for, so we remember the key at send time
    // to route the event back to the right UI row.
    let mut pending_session_usage: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    // Channel state isn't broadcast as a push event, so we refresh it
    // on a low-cadence heartbeat over the same socket. No cron poll —
    // the gateway's `cron` broadcast covers that live.
    let mut next_channel_heartbeat = Instant::now() + CHANNEL_HEARTBEAT;
    // Kick off log tailing one interval after connect so the Logs tab
    // lands pre-populated by the time the operator clicks over. The
    // cursor starts unset — the first call returns the recent slice
    // from EOF and every subsequent call sends back only new bytes.
    let mut next_log_tail = Instant::now() + LOG_TAIL_INTERVAL;
    let mut log_cursor: Option<i64> = None;

    loop {
        tokio::select! {
            _ = sleep_until(next_channel_heartbeat) => {
                rpc_id += 1;
                send_rpc(&mut socket, rpc_id, "channels.status").await?;
                next_channel_heartbeat = Instant::now() + CHANNEL_HEARTBEAT;
            }
            _ = sleep_until(next_log_tail) => {
                rpc_id += 1;
                let params = match log_cursor {
                    Some(c) => json!({ "cursor": c }),
                    None => json!({}),
                };
                send_rpc_params(&mut socket, rpc_id, "logs.tail", params).await?;
                next_log_tail = Instant::now() + LOG_TAIL_INTERVAL;
            }
            msg = socket.next() => {
                match msg {
                    Some(Ok(WsMsg::Text(txt))) => {
                        handle_frame(
                            &txt,
                            out,
                            &mut cron_id_to_name,
                            &mut seen_message_ids,
                            &mut log_cursor,
                            &mut pending_history,
                            &mut pending_session_history,
                            &mut pending_session_usage,
                        ).await?;
                    }
                    Some(Ok(WsMsg::Ping(p))) => {
                        let _ = socket.send(WsMsg::Pong(p)).await;
                    }
                    Some(Ok(WsMsg::Close(_))) | None => {
                        return Err(SessionError::SocketClosed);
                    }
                    Some(Err(e)) => return Err(SessionError::Connect(e)),
                    Some(Ok(_)) => {}
                }
            }
            Some(cmd) = cmd_rx.recv() => {
                match cmd {
                    // No-op when the session is already live. The
                    // button that sends this only renders while
                    // disconnected, so a live-session Reconnect is
                    // an edge case (e.g. racing with a just-accepted
                    // handshake) — drop it.
                    GatewayCommand::Reconnect => {
                        tracing::debug!("Reconnect received on live session; ignoring");
                    }
                    GatewayCommand::FetchChatHistory { ref agent_id } => {
                        // Remember which agent this id is for so the
                        // `res` handler can tag the emitted event.
                        rpc_id += 1;
                        pending_history.insert(rpc_id.to_string(), agent_id.clone());
                        send_command_rpc(&mut socket, rpc_id, &cmd).await?;
                    }
                    GatewayCommand::FetchSessionHistory { ref session_key } => {
                        // Parallel path to FetchChatHistory but
                        // routed to the session-specific transcript
                        // store — the app matches on the emitted
                        // event type, not the request id.
                        rpc_id += 1;
                        pending_session_history
                            .insert(rpc_id.to_string(), session_key.clone());
                        send_command_rpc(&mut socket, rpc_id, &cmd).await?;
                    }
                    GatewayCommand::FetchSessionUsage { ref session_key } => {
                        rpc_id += 1;
                        pending_session_usage
                            .insert(rpc_id.to_string(), session_key.clone());
                        send_command_rpc(&mut socket, rpc_id, &cmd).await?;
                    }
                    _ => {
                        rpc_id += 1;
                        send_command_rpc(&mut socket, rpc_id, &cmd).await?;
                    }
                }
            }
        }
    }
}

/// Wait `duration`, returning early if a command arrives. The
/// specific command doesn't matter — any signal from the UI wakes
/// the reconnect loop. In practice the UI only sends `Reconnect`
/// while disconnected (cron/approval buttons aren't rendered then).
async fn wait_or_command(
    duration: Duration,
    cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<GatewayCommand>,
) {
    match timeout(duration, cmd_rx.recv()).await {
        Ok(Some(cmd)) => {
            tracing::info!(?cmd, "reconnect wait interrupted by UI command");
        }
        Ok(None) => {
            // Sender dropped — channel closed. Nothing to do; the
            // outer loop will keep retrying anyway.
        }
        Err(_) => {
            // Timeout — normal scheduled wake-up.
        }
    }
}

/// Pure helper — build the RPC frame JSON for a UI command. Split
/// from the socket-sending path so the envelope can be unit-tested
/// against gateway schemas without a live WS. Panics on `Reconnect`,
/// which is a control-plane signal handled by the outer loop and
/// should never reach here.
fn build_command_frame(id: u64, cmd: &GatewayCommand) -> (&'static str, Value) {
    let (method, params) = match cmd {
        GatewayCommand::ResolveApproval {
            id: approval_id,
            decision,
        } => (
            "exec.approval.resolve",
            json!({ "id": approval_id, "decision": decision }),
        ),
        GatewayCommand::RunCron { job_id } => (
            "cron.run",
            // `mode: "force"` matches the CLI default — fire the job
            // now regardless of schedule.
            json!({ "id": job_id, "mode": "force" }),
        ),
        GatewayCommand::SendChat {
            agent_id,
            message,
            idempotency_key,
        } => (
            "chat.send",
            // Schema: `protocol/schema/logs-chat.ts:35` — sessionKey +
            // message + idempotencyKey are the required fields. We
            // use the fully-qualified `agent:<id>:main` form for every
            // agent (including the default) so routing is uniform —
            // the bare `"main"` shorthand only works for the default.
            json!({
                "sessionKey": agent_main_session_key(agent_id),
                "message": message,
                "idempotencyKey": idempotency_key,
            }),
        ),
        GatewayCommand::FetchChatHistory { agent_id } => (
            "chat.history",
            json!({
                "sessionKey": agent_main_session_key(agent_id),
                "limit": 200,
            }),
        ),
        GatewayCommand::FetchAgentIdentity { agent_id } => {
            ("agent.identity.get", json!({ "agentId": agent_id }))
        }
        GatewayCommand::ResetSession { session_key } => (
            // Schema: `server-methods/sessions.ts:1332` —
            // `{ key: sessionKey, reason: "reset" | "new" }`. We
            // use "reset" so the session entry survives with a
            // fresh transcript; "new" rotates the identifier.
            "sessions.reset",
            json!({ "key": session_key, "reason": "reset" }),
        ),
        GatewayCommand::FetchSessionHistory { session_key } => (
            // Same RPC as the Chat-tab bootstrap path, just with an
            // arbitrary session key — the gateway happily resolves
            // any `agent:<id>:<sessionId>` form, not just `:main`.
            "chat.history",
            json!({ "sessionKey": session_key, "limit": 200 }),
        ),
        GatewayCommand::FetchSessionUsage { session_key } => (
            // Schema: `openclaw/src/gateway/server-methods/usage.ts:829`
            // — requires `key` (session key, not an agent id). The
            // gateway downsamples to ≤200 points and returns
            // `{ sessionId, points: [...] }`.
            "sessions.usage.timeseries",
            json!({ "key": session_key }),
        ),
        GatewayCommand::Reconnect => {
            unreachable!("Reconnect is handled in the session select arm, never sent as RPC")
        }
    };
    let frame = json!({
        "type": "req",
        "id": id.to_string(),
        "method": method,
        "params": params,
    });
    (method, frame)
}

async fn send_command_rpc(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    cmd: &GatewayCommand,
) -> Result<(), SessionError> {
    let (method, frame) = build_command_frame(id, cmd);
    tracing::info!(method, id, "sending UI command");
    socket
        .send(WsMsg::Text(frame.to_string().into()))
        .await
        .map_err(|e| SessionError::Send(e.to_string()))
}

async fn send_rpc(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    method: &str,
) -> Result<(), SessionError> {
    send_rpc_params(socket, id, method, json!({})).await
}

async fn send_rpc_params(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    method: &str,
    params: Value,
) -> Result<(), SessionError> {
    let frame = json!({
        "type": "req",
        "id": id.to_string(),
        "method": method,
        "params": params,
    });
    socket
        .send(WsMsg::Text(frame.to_string().into()))
        .await
        .map_err(|e| SessionError::Send(e.to_string()))
}

/// Build the gateway sessionKey for an agent's main chat session.
/// Mirrors `buildAgentMainSessionKey` at
/// `openclaw/src/routing/session-key.ts:120`. The bare `"main"`
/// shorthand only addresses the default agent; the full form works
/// for every agent including the default, so we use it uniformly.
fn agent_main_session_key(agent_id: &str) -> String {
    format!("agent:{agent_id}:main")
}

/// Is this assistant text one of OpenClaw's silent-reply sentinels
/// (`NO_REPLY` / `HEARTBEAT_OK`)? Mirrors `isSilentReplyText` +
/// `isSilentReplyEnvelopeText` in `openclaw/src/auto-reply/tokens.ts`
/// — the server suppresses these before delivering to external
/// channels, but `session.message` broadcasts carry the raw text, so
/// we filter client-side. Case-insensitive, tolerant of surrounding
/// whitespace and the `{"action":"NO_REPLY"}` envelope form.
pub(crate) fn is_silent_reply(text: &str) -> bool {
    const SENTINELS: &[&str] = &["NO_REPLY", "HEARTBEAT_OK"];
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    for token in SENTINELS {
        if trimmed.eq_ignore_ascii_case(token) {
            return true;
        }
        // `{"action":"NO_REPLY"}` envelope form. We don't parse JSON
        // for this — a lightweight string check is enough since the
        // surrounding code only uses the result as a yes/no gate.
        if trimmed.starts_with('{')
            && trimmed.ends_with('}')
            && trimmed.to_ascii_uppercase().contains(token)
            && trimmed.contains("\"action\"")
        {
            return true;
        }
    }
    false
}

/// Extract the `agent_id` portion from a `session.message` sessionKey
/// of the form `"agent:<id>:<sessionId>"`. Returns `None` for keys
/// that don't match the canonical shape, so older / legacy keys fall
/// through to the caller's default.
pub(crate) fn agent_id_from_session_key(key: &str) -> Option<&str> {
    let rest = key.strip_prefix("agent:")?;
    // `rest` = `"<agentId>:<sessionId>"`. Split at the first colon.
    let end = rest.find(':')?;
    let id = &rest[..end];
    (!id.is_empty()).then_some(id)
}

// Session-loop frame dispatcher. The argument list grew organically
// as new RPC flows (chat.history drill-in, usage timeseries) needed
// their own correlation state; grouping into a struct would add a
// layer between the session loop and its scratch state for little
// real benefit.
#[allow(clippy::too_many_arguments)]
async fn handle_frame(
    txt: &str,
    out: &mut iced::futures::channel::mpsc::Sender<WsEvent>,
    cron_id_to_name: &mut std::collections::HashMap<String, String>,
    seen_message_ids: &mut std::collections::VecDeque<String>,
    log_cursor: &mut Option<i64>,
    pending_history: &mut std::collections::HashMap<String, String>,
    pending_session_history: &mut std::collections::HashMap<String, String>,
    pending_session_usage: &mut std::collections::HashMap<String, String>,
) -> Result<(), SessionError> {
    let frame: Value = serde_json::from_str(txt)?;

    let frame_type = frame.get("type").and_then(Value::as_str);

    match frame_type {
        Some("res") => {
            let ok = frame.get("ok") == Some(&Value::Bool(true));
            let res_id = frame.get("id").and_then(Value::as_str).unwrap_or("?");
            if !ok {
                let err_msg = frame
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("(no message)");
                tracing::warn!(id = %res_id, error = %err_msg, "gateway RPC rejected");
                return Ok(());
            }
            let Some(payload) = frame.get("payload") else {
                return Ok(());
            };
            // Log any res that carries a `status` at info — most
            // stateful RPC acks (chat.send → "started", cron.run →
            // "queued", etc.) use this shape, and surfacing them
            // at the default log level turns silent send-but-nothing
            // symptoms into a visible "gateway said X" line.
            if let Some(status) = payload.get("status").and_then(Value::as_str) {
                tracing::info!(id = %res_id, status, "gateway res ok");
            } else {
                tracing::trace!(id = %res_id, "gateway res ok");
            }

            if let Some(crons) = try_cron_list(payload) {
                tracing::debug!(count = crons.len(), "cron snapshot");
                // Learn id → name mappings so subsequent UUID-keyed
                // events resolve to roster-matching names.
                for cron in &crons {
                    if let Some(id) = cron.id.as_deref() {
                        cron_id_to_name.insert(id.to_string(), cron.name.clone());
                    }
                }
                let _ = out.send(WsEvent::CronSnapshot(crons)).await;
                return Ok(());
            }
            if let Some(tail) = try_log_tail(payload) {
                tracing::trace!(lines = tail.lines.len(), cursor = tail.cursor, "log tail");
                *log_cursor = Some(tail.cursor);
                let _ = out.send(WsEvent::LogTail(tail)).await;
                return Ok(());
            }
            if let Some(identity) = try_agent_identity(payload) {
                let (agent_id, name, emoji) = identity;
                tracing::info!(
                    agent = %agent_id,
                    name = ?name,
                    emoji = ?emoji,
                    "agent identity",
                );
                let _ = out
                    .send(WsEvent::AgentIdentity {
                        agent_id: crate::domain::AgentId::new(agent_id),
                        name,
                        emoji,
                    })
                    .await;
                return Ok(());
            }
            if let Some((default_id, agents)) = try_agents_list(payload) {
                tracing::info!(
                    count = agents.len(),
                    default = %default_id,
                    "agents list",
                );
                let _ = out.send(WsEvent::AgentsList { default_id, agents }).await;
                return Ok(());
            }
            if let Some(history) = try_chat_history(payload) {
                // chat.history responses don't echo the sessionKey,
                // so we recover what the call was for from one of
                // the two pending maps we stamped at send time.
                // Session-drill-in requests have priority: they're
                // explicit operator actions with a specific key,
                // whereas the agent-main path has a sane fallback
                // ("main") for stale ids.
                if let Some(session_key) = pending_session_history.remove(res_id) {
                    tracing::info!(
                        count = history.len(),
                        session = %session_key,
                        "session history drill-in",
                    );
                    let _ = out
                        .send(WsEvent::SessionHistory {
                            session_key,
                            messages: history,
                        })
                        .await;
                    return Ok(());
                }
                let agent_id = pending_history
                    .remove(res_id)
                    .unwrap_or_else(|| "main".to_string());
                tracing::info!(
                    count = history.len(),
                    agent = %agent_id,
                    "chat history bootstrap",
                );
                let _ = out
                    .send(WsEvent::ChatHistory {
                        agent_id: crate::domain::AgentId::new(agent_id),
                        messages: history,
                    })
                    .await;
                return Ok(());
            }
            // `sessions.usage.timeseries` response — matched by
            // request-id before falling through to shape-based
            // detectors, because the payload's only distinctive
            // field (`points`) could collide with any future array-
            // shaped RPC.
            if let Some(session_key) = pending_session_usage.remove(res_id)
                && let Some(timeseries) = try_session_usage_timeseries(payload)
            {
                tracing::info!(
                    count = timeseries.points.len(),
                    session = %session_key,
                    "session usage timeseries",
                );
                let _ = out
                    .send(WsEvent::SessionUsageTimeseries {
                        session_key,
                        points: timeseries.points,
                    })
                    .await;
                return Ok(());
            }
            if let Some(sessions) = try_session_list(payload) {
                tracing::debug!(count = sessions.len(), "session list");
                for s in sessions {
                    let _ = out.send(WsEvent::SessionUsage(s)).await;
                }
                return Ok(());
            }
            if let Some(channels) = try_channel_list(payload) {
                tracing::debug!(count = channels.len(), "channel snapshot");
                let _ = out.send(WsEvent::ChannelSnapshot(channels)).await;
                return Ok(());
            }
            if let Some(main) = try_main_agent(payload) {
                let _ = out.send(WsEvent::MainAgent(main)).await;
                return Ok(());
            }
            tracing::trace!(raw = %txt, "unrecognized res payload");
        }
        Some("event") => {
            let event = frame.get("event").and_then(Value::as_str).unwrap_or("?");
            let payload = frame.get("payload");
            handle_event(event, payload, out, cron_id_to_name, seen_message_ids).await?;
        }
        other => {
            tracing::trace!(?other, raw = %txt, "unknown frame type");
        }
    }

    Ok(())
}

async fn handle_event(
    event: &str,
    payload: Option<&Value>,
    out: &mut iced::futures::channel::mpsc::Sender<WsEvent>,
    cron_id_to_name: &std::collections::HashMap<String, String>,
    seen_message_ids: &mut std::collections::VecDeque<String>,
) -> Result<(), SessionError> {
    match event {
        // Scope-free: push delta for a single cron job.
        "cron" => {
            let Some(payload) = payload else {
                return Ok(());
            };
            let mut evt: CronEventPayload = match serde_json::from_value(payload.clone()) {
                Ok(e) => e,
                Err(e) => {
                    tracing::debug!(error = %e, raw = %payload, "cron event parse failed");
                    return Ok(());
                }
            };
            // Translate the UUID jobId to the roster's name so
            // apply_status_update lands on the right agent. Falls
            // back to the raw UUID only if we haven't seen the
            // snapshot yet (first few events after connect).
            if evt.job_name.is_none()
                && let Some(name) = cron_id_to_name.get(&evt.job_id)
            {
                evt.job_name = Some(name.clone());
            }
            tracing::debug!(
                job_id = %evt.job_id,
                job_name = ?evt.job_name,
                action = %evt.action,
                status = ?evt.status,
                "cron event",
            );
            if let Some(job) = cron_job_from_event(&evt) {
                let _ = out.send(WsEvent::CronDelta(job)).await;
            }
            // `added`/`updated`/`removed` don't imply live status change;
            // a future improvement can re-fire `cron.status` here if the
            // job set drifts from the static roster.
        }

        // Scope-free `chat` event. We *could* render bubbles from this
        // stream too, but `session.message` (scoped, below) covers both
        // internal and external channels — handling both causes
        // duplicate bubbles on the internal-channel path. Keep this as
        // a trace-level observation of agent delta streaming.
        "chat" => {
            tracing::trace!(
                ?payload,
                "chat event (not rendered — using session.message)"
            );
        }

        // Scope-free: agent run stream (tool-call phases, lifecycle).
        // Surfaces as activity nudges, not text.
        "agent" => {
            let Some(payload) = payload else {
                return Ok(());
            };
            let evt: AgentEventPayload = match serde_json::from_value(payload.clone()) {
                Ok(e) => e,
                Err(e) => {
                    tracing::trace!(error = %e, "agent event parse failed");
                    return Ok(());
                }
            };
            if let Some(kind) = agent_stream_to_activity(&evt.stream) {
                let agent_id = evt
                    .session_key
                    .as_deref()
                    .and_then(agent_id_from_session_key)
                    .unwrap_or(CHAT_AGENT_ID)
                    .to_string();
                let _ = out
                    .send(WsEvent::AgentActivity {
                        agent_id: AgentId::new(agent_id),
                        kind,
                    })
                    .await;
            }
        }

        // Scoped (requires operator.read): session set changed.
        "sessions.changed" => {
            let _ = out.send(WsEvent::SessionsChanged).await;
        }

        // Scoped: per-message updates inside a session. Unlike the
        // scope-free `chat` event (which is suppressed for non-internal
        // channels — Slack/Telegram/WhatsApp), `session.message` fires
        // for every assistant turn regardless of originating channel.
        // This is the path that surfaces Slack-routed conversations
        // onto the office scene.
        "session.message" => {
            let Some(payload) = payload else {
                return Ok(());
            };
            let role = payload
                .get("message")
                .and_then(|m| m.get("role"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if role != "assistant" {
                // Skip user / system / tool messages — we're surfacing
                // agent output, not operator input.
                return Ok(());
            }
            // OpenClaw fires `session.message` twice per assistant turn
            // (initial insert + metadata finalize); both carry the
            // same messageId. Skip duplicates so the bubble renders
            // once. When the id is missing we fall through — rare,
            // and at worst one extra bubble for that turn.
            if let Some(mid) = payload
                .get("messageId")
                .and_then(Value::as_str)
                .or_else(|| {
                    payload
                        .get("message")
                        .and_then(|m| m.get("id"))
                        .and_then(Value::as_str)
                })
            {
                if seen_message_ids.iter().any(|s| s == mid) {
                    return Ok(());
                }
                seen_message_ids.push_back(mid.to_string());
                if seen_message_ids.len() > 32 {
                    seen_message_ids.pop_front();
                }
            }
            // The gateway spreads the same session-snapshot fields
            // that `sessions.list` returns into this payload, so we
            // can update the per-session view without a separate
            // poll. Reuse `SessionInfo`'s deserializer — extra
            // fields on `session.message` are ignored by serde.
            if let Ok(info) =
                serde_json::from_value::<crate::net::rpc::SessionInfo>(payload.clone())
                && !info.key.is_empty()
            {
                let _ = out.send(WsEvent::SessionUsage(info)).await;
            }
            let text = extract_message_text(payload);
            if text.trim().is_empty() {
                return Ok(());
            }
            // Route by sessionKey so a reply from a non-default agent
            // lands in the right chat log / over the right sprite.
            // Fallback to `main` if the key is missing or malformed —
            // older gateway builds and some internal tests don't set
            // it on every broadcast.
            let session_key = payload
                .get("sessionKey")
                .and_then(Value::as_str)
                .unwrap_or("");
            let agent_id_str = agent_id_from_session_key(session_key)
                .unwrap_or(CHAT_AGENT_ID)
                .to_string();
            // Silent-reply sentinel handling: the server itself
            // suppresses `NO_REPLY` when delivering to external
            // channels, but `session.message` broadcasts carry the
            // raw assistant content. Strip / detect it client-side
            // so it doesn't render as a bubble or log entry.
            if is_silent_reply(&text) {
                tracing::debug!(
                    agent = %agent_id_str,
                    "session.message silent-reply sentinel — skipping render",
                );
                let _ = out
                    .send(WsEvent::AgentSilentTurn {
                        agent_id: AgentId::new(agent_id_str),
                    })
                    .await;
                return Ok(());
            }
            tracing::debug!(
                len = text.len(),
                agent = %agent_id_str,
                session_key,
                "session.message assistant",
            );
            let _ = out
                .send(WsEvent::AgentMessage {
                    agent_id: AgentId::new(agent_id_str),
                    text,
                })
                .await;
        }
        "session.tool" => {
            // Extract the tool name and phase so the bubble text is
            // useful ("⚙ bash", "⚙ read_file", etc.) instead of a
            // generic "tool-calling" indicator. Multiple tool events
            // fire per invocation (start/output/done); we render a
            // bubble only on `phase == "start"` to avoid flashing.
            let Some(payload) = payload else {
                return Ok(());
            };
            let data = payload.get("data");
            let phase = data
                .and_then(|d| d.get("phase"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let tool_name = data
                .and_then(|d| d.get("name").or_else(|| d.get("tool")))
                .and_then(Value::as_str)
                .unwrap_or("tool");
            let session_key = payload
                .get("sessionKey")
                .and_then(Value::as_str)
                .unwrap_or("");
            let agent_id = agent_id_from_session_key(session_key)
                .unwrap_or(CHAT_AGENT_ID)
                .to_string();
            tracing::debug!(phase, tool = tool_name, agent = %agent_id, "session.tool");
            if phase == "start" {
                let _ = out
                    .send(WsEvent::AgentToolInvoked {
                        agent_id: AgentId::new(agent_id.clone()),
                        text: format!("⚙ {tool_name}"),
                    })
                    .await;
            }
            let _ = out
                .send(WsEvent::AgentActivity {
                    agent_id: AgentId::new(agent_id),
                    kind: ActivityKind::ToolCalling,
                })
                .await;
        }

        // Scoped (APPROVALS_SCOPE): exec approvals for the operator.
        "exec.approval.requested" => {
            let Some(payload) = payload else {
                return Ok(());
            };
            match serde_json::from_value::<ApprovalEventPayload>(payload.clone()) {
                Ok(p) => {
                    let _ = out.send(WsEvent::ApprovalRequested(p)).await;
                }
                Err(e) => tracing::debug!(error = %e, "approval.requested parse failed"),
            }
        }
        "exec.approval.resolved" => {
            let id = payload
                .and_then(|p| p.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let _ = out.send(WsEvent::ApprovalResolved { id }).await;
        }

        // Gateway-side upgrade notification. Payload shape (from
        // `openclaw/src/gateway/events.ts:5`):
        // `{ updateAvailable: { currentVersion, latestVersion, channel } | null }`.
        "update.available" => {
            let Some(payload) = payload else {
                return Ok(());
            };
            let update = payload.get("updateAvailable");
            let parsed = update.and_then(|u| {
                // null clears; an object populates.
                if u.is_null() {
                    return None;
                }
                let current = u.get("currentVersion").and_then(Value::as_str)?;
                let latest = u.get("latestVersion").and_then(Value::as_str)?;
                let channel = u.get("channel").and_then(Value::as_str).unwrap_or("stable");
                Some(GatewayUpdate {
                    current: current.to_string(),
                    latest: latest.to_string(),
                    channel: channel.to_string(),
                })
            });
            if let Some(ref upd) = parsed {
                tracing::info!(
                    current = %upd.current,
                    latest = %upd.latest,
                    channel = %upd.channel,
                    "gateway update available",
                );
            }
            let _ = out.send(WsEvent::UpdateAvailable(parsed)).await;
        }

        "tick" | "health" | "heartbeat" | "connect.challenge" => {
            // Heartbeats already handled by WS ping/pong; challenge is
            // the handshake frame and can't arrive on a live session.
        }

        _ => {
            tracing::trace!(event, "unhandled gateway event");
        }
    }

    Ok(())
}

/// Pull human-readable text out of a `session.message` payload's
/// `message.content[]`. OpenClaw messages carry content as an array of
/// typed chunks (`{type:"text",text:"..."}`, tool results, media, etc.)
/// — we concatenate the text chunks and drop the rest.
fn extract_message_text(payload: &Value) -> String {
    let Some(content) = payload
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    else {
        return String::new();
    };
    content
        .iter()
        .filter_map(|chunk| {
            let kind = chunk.get("type").and_then(Value::as_str)?;
            if kind != "text" {
                return None;
            }
            chunk
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect::<Vec<_>>()
        .join("")
}

fn try_cron_list(v: &Value) -> Option<Vec<CronJob>> {
    // Gateway shapes observed: {"jobs":[...]} or a bare array
    let candidate = v.get("jobs").or(Some(v))?;
    let list: Vec<CronJob> = serde_json::from_value(candidate.clone()).ok()?;
    // Reject empty-ish matches that could just be some other shape.
    if list.iter().any(|c| c.name.is_empty()) {
        return None;
    }
    Some(list)
}

fn try_channel_list(v: &Value) -> Option<Vec<Channel>> {
    // Only accept payloads that explicitly carry a `channels` key — a
    // looser match (e.g. treating the whole payload as a map) picks up
    // unrelated RPC responses like `{subscribed: true}`.
    let candidate = v.get("channels")?;

    // Shape A: bare array of `{name, enabled, connected, ...}`.
    if candidate.is_array() {
        let list: Vec<Channel> = serde_json::from_value(candidate.clone()).ok()?;
        if list.iter().any(|c| c.name.is_empty()) {
            return None;
        }
        return Some(list);
    }

    // Shape B: map keyed by provider name, as emitted by `channels.status`
    // (`{ "slack": {configured, running, lastError, ...}, ... }`). Fields
    // don't include `name`/`enabled`/`connected` directly — synthesize
    // them from `configured` (enabled) and `running` + `lastError`
    // (connected) so the desktop's roster + status mapping keep working.
    let obj = candidate.as_object()?;
    let order = v
        .get("channelOrder")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| obj.keys().cloned().collect());

    let channels = order
        .iter()
        .filter_map(|name| {
            let entry = obj.get(name)?;
            let enabled = entry
                .get("configured")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let running = entry
                .get("running")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let last_error = entry
                .get("lastError")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(Channel {
                name: name.clone(),
                enabled,
                connected: running && last_error.is_none(),
                last_error,
            })
        })
        .collect::<Vec<_>>();

    (!channels.is_empty()).then_some(channels)
}

fn try_log_tail(v: &Value) -> Option<LogTailPayload> {
    // Distinguishing a logs.tail response: it's the only RPC
    // returning both `cursor` (number) and `lines` (array of strings)
    // at the top level.
    if !v.get("cursor").is_some_and(Value::is_number) {
        return None;
    }
    if !v.get("lines").is_some_and(Value::is_array) {
        return None;
    }
    serde_json::from_value(v.clone()).ok()
}

/// `agent.identity.get` response shape: `{agentId, name, avatar, emoji?}`.
/// The top-level `agentId` is our detection signal — neither
/// `agents.list` (uses `defaultId`+`agents`) nor any other response we
/// care about carries that exact key at the root.
fn try_agent_identity(v: &Value) -> Option<(String, Option<String>, Option<String>)> {
    let agent_id = v.get("agentId").and_then(Value::as_str)?.to_string();
    if agent_id.is_empty() {
        return None;
    }
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty());
    let emoji = v
        .get("emoji")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty());
    Some((agent_id, name, emoji))
}

fn try_agents_list(v: &Value) -> Option<(String, Vec<AgentInfo>)> {
    // `agents.list` returns `{defaultId, mainKey, scope, agents: [...]}`.
    // Presence of `defaultId` + the `agents` array is our signal —
    // keeps us out of the way of `sessions.list` (which carries
    // `sessions: [...]`) and of other `agents`-prefixed RPCs that
    // might arrive in future. `AgentsListResponse` accepts any shape
    // that has those two keys thanks to `#[serde(default)]` on
    // everything else.
    if v.get("defaultId").is_none() || !v.get("agents").is_some_and(Value::is_array) {
        return None;
    }
    let resp: crate::net::rpc::AgentsListResponse = serde_json::from_value(v.clone()).ok()?;
    Some((resp.default_id, resp.agents))
}

fn try_chat_history(v: &Value) -> Option<Vec<crate::ui::chat_view::ChatMessage>> {
    use crate::ui::chat_view::{ChatMessage, ChatRole};

    let arr = v.get("messages").and_then(Value::as_array)?;
    // `messages` is a shared key — reject when the entries don't
    // look like chat messages at all (no role field anywhere) so a
    // future RPC that happens to return `{messages: [...]}` in a
    // different shape doesn't get mis-routed here.
    if !arr
        .iter()
        .any(|m| m.get("role").and_then(Value::as_str).is_some())
    {
        return None;
    }
    let history: Vec<ChatMessage> = arr
        .iter()
        .filter_map(|m| {
            let role_str = m.get("role").and_then(Value::as_str)?;
            let role = match role_str {
                "user" => ChatRole::User,
                "assistant" => ChatRole::Assistant,
                _ => ChatRole::Other,
            };
            let text = flatten_message_content(m)?;
            if text.trim().is_empty() {
                return None;
            }
            // Drop silent-reply sentinels — they're agent "stay
            // quiet" signals, not content. Without this the Chat
            // tab bootstrap renders a wall of `NO_REPLY` rows for
            // agents that chose silence on past turns.
            if is_silent_reply(&text) {
                return None;
            }
            Some(ChatMessage {
                role,
                text,
                at: std::time::SystemTime::now(),
            })
        })
        .collect();
    Some(history)
}

/// Pull the text out of a chat-history entry. OpenClaw's
/// `chat.history` returns a union shape — either `content: "..."` or
/// `content: [{type: "text", text: "..."}, ...]` (or the legacy
/// top-level `text` field). Join text chunks and drop non-text blocks
/// so the UI renders a single clean string.
fn flatten_message_content(message: &Value) -> Option<String> {
    if let Some(s) = message.get("content").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    if let Some(arr) = message.get("content").and_then(Value::as_array) {
        let joined = arr
            .iter()
            .filter_map(|chunk| {
                let kind = chunk.get("type").and_then(Value::as_str)?;
                if kind != "text" {
                    return None;
                }
                chunk
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>()
            .join("");
        return Some(joined);
    }
    if let Some(s) = message.get("text").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    None
}

fn try_session_usage_timeseries(v: &Value) -> Option<crate::net::rpc::SessionUsageTimeseries> {
    // Payload shape: `{ sessionId, points: [...] }`. Accept either
    // the object itself or an envelope that nests it — future-proof
    // against an envelope change that'd otherwise drop the chart.
    if v.get("points").is_some() {
        return serde_json::from_value(v.clone()).ok();
    }
    None
}

fn try_session_list(v: &Value) -> Option<Vec<SessionInfo>> {
    let arr = v.get("sessions").and_then(Value::as_array)?;
    let out: Vec<SessionInfo> = arr
        .iter()
        .filter_map(|entry| serde_json::from_value(entry.clone()).ok())
        .collect();
    (!out.is_empty()).then_some(out)
}

fn try_main_agent(v: &Value) -> Option<MainAgent> {
    // Gateway may return {"agents":[{..}]} or a bare object
    if let Some(agents) = v.get("agents").and_then(|a| a.as_array()) {
        for a in agents {
            if let Ok(m) = serde_json::from_value::<MainAgent>(a.clone())
                && m.id == "main"
            {
                return Some(m);
            }
        }
    }
    serde_json::from_value::<MainAgent>(v.clone()).ok()
}

#[cfg(test)]
mod command_rpc_tests {
    use super::*;

    #[test]
    fn send_chat_frame_uses_agent_scoped_session_key() {
        let cmd = GatewayCommand::SendChat {
            agent_id: "memoria".into(),
            message: "hello memoria".into(),
            idempotency_key: "idem-42".into(),
        };
        let (method, frame) = build_command_frame(7, &cmd);
        assert_eq!(method, "chat.send");
        // Matches `buildAgentMainSessionKey`
        // (openclaw/src/routing/session-key.ts:120).
        assert_eq!(frame["params"]["sessionKey"], "agent:memoria:main");
        assert_eq!(frame["params"]["message"], "hello memoria");
        assert_eq!(frame["params"]["idempotencyKey"], "idem-42");
    }

    #[test]
    fn send_chat_frame_for_default_agent_uses_full_form() {
        // Even for the default ("main") agent we send the full
        // `agent:main:main` sessionKey instead of the legacy shorthand
        // so inbound `session.message` routing is symmetric.
        let cmd = GatewayCommand::SendChat {
            agent_id: "main".into(),
            message: "hi".into(),
            idempotency_key: "k".into(),
        };
        let (_, frame) = build_command_frame(1, &cmd);
        assert_eq!(frame["params"]["sessionKey"], "agent:main:main");
    }

    #[test]
    fn fetch_chat_history_frame_targets_agent_session() {
        let cmd = GatewayCommand::FetchChatHistory {
            agent_id: "docs".into(),
        };
        let (method, frame) = build_command_frame(9, &cmd);
        assert_eq!(method, "chat.history");
        assert_eq!(frame["params"]["sessionKey"], "agent:docs:main");
        assert_eq!(frame["params"]["limit"], 200);
    }

    #[test]
    fn is_silent_reply_detects_sentinels() {
        // Bare + whitespace-padded + case-insensitive — mirrors
        // `isSilentReplyText` in `auto-reply/tokens.ts`.
        assert!(is_silent_reply("NO_REPLY"));
        assert!(is_silent_reply("  NO_REPLY  "));
        assert!(is_silent_reply("\nno_reply\n"));
        assert!(is_silent_reply("HEARTBEAT_OK"));
        // Envelope form emitted by some agents:
        // `{"action":"NO_REPLY"}`.
        assert!(is_silent_reply("{\"action\":\"NO_REPLY\"}"));
        // Substantive replies that happen to contain the token must
        // NOT be suppressed — matches issue #19537 in OpenClaw.
        assert!(!is_silent_reply("Here's a reply.\nNO_REPLY"));
        assert!(!is_silent_reply("NO_REPLY but here is more"));
        assert!(!is_silent_reply(""));
        assert!(!is_silent_reply("Hello"));
    }

    #[test]
    fn session_key_parser_extracts_agent_id() {
        assert_eq!(agent_id_from_session_key("agent:main:main"), Some("main"));
        assert_eq!(
            agent_id_from_session_key("agent:memoria:dashboard:foo"),
            Some("memoria")
        );
        // Malformed / legacy keys return None so the caller can fall
        // back to a default agent instead of routing to a bogus id.
        assert_eq!(agent_id_from_session_key("main"), None);
        assert_eq!(agent_id_from_session_key("agent::main"), None);
        assert_eq!(agent_id_from_session_key("agent:onlyid"), None);
    }

    #[test]
    fn resolve_approval_frame_matches_gateway_schema() {
        let cmd = GatewayCommand::ResolveApproval {
            id: "abc-123".into(),
            decision: "allow-once".into(),
        };
        let (method, frame) = build_command_frame(42, &cmd);
        assert_eq!(method, "exec.approval.resolve");
        // Matches OpenClaw's `validateExecApprovalResolveParams`
        // (server-methods/exec-approval.ts:332) — `{id, decision}`.
        assert_eq!(frame["type"], "req");
        assert_eq!(frame["id"], "42");
        assert_eq!(frame["method"], "exec.approval.resolve");
        assert_eq!(frame["params"]["id"], "abc-123");
        assert_eq!(frame["params"]["decision"], "allow-once");
    }
}
