//! Real WebSocket client for the OpenClaw gateway on ubu-3xdv.
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

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iced::futures::Stream;
use iced::stream;
use serde_json::{Value, json};
use tokio::time::{Instant, sleep, sleep_until};
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
    MainAgent, SessionInfo,
};

/// Cadence of the channel-status heartbeat RPC. Channels are not yet
/// broadcast as push events on the gateway, so we refresh via the
/// already-open socket on a slow interval.
const CHANNEL_HEARTBEAT: Duration = Duration::from_secs(30);
const INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

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

/// Iced subscription stream for the real gateway.
///
/// Keep as a free function (not a closure) — Iced's `Subscription::run`
/// uses the function pointer as subscription identity.
pub fn connect() -> impl Stream<Item = WsEvent> {
    stream::channel(64, async move |mut out| {
        let gateway_url = config::gateway_url();
        let token = config::try_load_token();

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
                }
                Err(e) => {
                    tracing::warn!(error = %e, "session errored; reconnecting");
                    let _ = out.send(WsEvent::Disconnected(e.to_string())).await;
                }
            }

            sleep(backoff).await;
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    })
}

#[derive(Debug, thiserror::Error)]
enum SessionError {
    #[error("connect {0}")]
    Connect(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("handshake rejected: {0}")]
    HandshakeRejected(String),
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
    // Gateway enforces `controlUi.allowedOrigins` on WS upgrade. Use
    // one of the configured origins — the Control UI's own dashboard
    // origin satisfies this for either endpoint.
    req.headers_mut().insert(
        "Origin",
        HeaderValue::from_static("https://ubu-3xdv.tail4fb3a4.ts.net"),
    );
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
    // `operator.approvals` is required for `exec.approval.resolve` to
    // land (server-methods/method-scopes.ts:43). Without it the button
    // clicks look like they worked (the panel collapses optimistically)
    // but the gateway silently ignores the RPC and re-broadcasts the
    // same pending approval on the next reconnect.
    let scopes: &[&str] = &["operator.read", "operator.approvals"];
    let mut params_obj = json!({
        "minProtocol": 3,
        "maxProtocol": 3,
        "client": {
            "id": client_id,
            "displayName": "Mission Control Desktop",
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
                                let _ = out.send(WsEvent::Connected).await;
                                break;
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
    // Pull `agent.identity.get` so the roster can swap the placeholder
    // "main" display for the operator-chosen persona (name + emoji).
    // The WS `agents.list` response omits identity — this one is
    // backed by `resolveAssistantIdentity` on the server.
    rpc_id += 1;
    send_rpc_params(
        &mut socket,
        rpc_id,
        "agent.identity.get",
        json!({ "agentId": "main" }),
    )
    .await?;

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

    // Channel state isn't broadcast as a push event, so we refresh it
    // on a low-cadence heartbeat over the same socket. No cron poll —
    // the gateway's `cron` broadcast covers that live.
    let mut next_channel_heartbeat = Instant::now() + CHANNEL_HEARTBEAT;

    loop {
        tokio::select! {
            _ = sleep_until(next_channel_heartbeat) => {
                rpc_id += 1;
                send_rpc(&mut socket, rpc_id, "channels.status").await?;
                next_channel_heartbeat = Instant::now() + CHANNEL_HEARTBEAT;
            }
            msg = socket.next() => {
                match msg {
                    Some(Ok(WsMsg::Text(txt))) => {
                        handle_frame(&txt, out, &mut cron_id_to_name, &mut seen_message_ids).await?;
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
                rpc_id += 1;
                send_command_rpc(&mut socket, rpc_id, &cmd).await?;
            }
        }
    }
}

/// Pure helper — build the RPC frame JSON for a UI command. Split
/// from the socket-sending path so the envelope can be unit-tested
/// against gateway schemas without a live WS.
fn build_command_frame(id: u64, cmd: &GatewayCommand) -> (&'static str, Value) {
    let (method, params) = match cmd {
        GatewayCommand::ResolveApproval {
            id: approval_id,
            decision,
        } => (
            "exec.approval.resolve",
            json!({ "id": approval_id, "decision": decision }),
        ),
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

async fn handle_frame(
    txt: &str,
    out: &mut iced::futures::channel::mpsc::Sender<WsEvent>,
    cron_id_to_name: &mut std::collections::HashMap<String, String>,
    seen_message_ids: &mut std::collections::VecDeque<String>,
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
            tracing::trace!(id = %res_id, "gateway res ok");

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
            if let Some(agents) = try_agents_list(payload) {
                tracing::info!(count = agents.len(), "agents list");
                let _ = out.send(WsEvent::AgentsIdentity(agents)).await;
                return Ok(());
            }
            if let Some(sessions) = try_session_list(payload) {
                tracing::debug!(count = sessions.len(), "session list");
                for s in sessions {
                    if let (Some(total), Some(ctx)) = (s.total_tokens, s.context_tokens) {
                        let _ = out
                            .send(WsEvent::SessionUsage {
                                session_key: s.key,
                                total_tokens: total,
                                context_tokens: ctx,
                            })
                            .await;
                    }
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
                send_activity(out, kind).await;
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
            // Pick off the session-usage snapshot that the gateway
            // spreads into this payload — lets the status bar track
            // context growth without a separate poll.
            if let Some(session_key) = payload.get("sessionKey").and_then(Value::as_str) {
                let total = payload.get("totalTokens").and_then(Value::as_i64);
                let ctx = payload.get("contextTokens").and_then(Value::as_i64);
                if let (Some(total), Some(ctx)) = (total, ctx) {
                    let _ = out
                        .send(WsEvent::SessionUsage {
                            session_key: session_key.to_string(),
                            total_tokens: total,
                            context_tokens: ctx,
                        })
                        .await;
                }
            }
            let text = extract_message_text(payload);
            if text.trim().is_empty() {
                return Ok(());
            }
            tracing::debug!(len = text.len(), "session.message assistant");
            let _ = out
                .send(WsEvent::AgentMessage {
                    agent_id: AgentId::new(CHAT_AGENT_ID),
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
            tracing::debug!(phase, tool = tool_name, "session.tool");
            if phase == "start" {
                let _ = out
                    .send(WsEvent::AgentToolInvoked {
                        agent_id: AgentId::new(CHAT_AGENT_ID),
                        text: format!("⚙ {tool_name}"),
                    })
                    .await;
            }
            send_activity(out, ActivityKind::ToolCalling).await;
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

async fn send_activity(
    out: &mut iced::futures::channel::mpsc::Sender<WsEvent>,
    kind: ActivityKind,
) {
    let _ = out
        .send(WsEvent::AgentActivity {
            agent_id: AgentId::new(CHAT_AGENT_ID),
            kind,
        })
        .await;
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

fn try_agents_list(v: &Value) -> Option<Vec<AgentInfo>> {
    // `agent.identity.get` returns a single object:
    // `{agentId, name, avatar, emoji?}`. The presence of `agentId`
    // (as opposed to plain `id`) is our signal — other RPC responses
    // don't use that key.
    if v.get("agentId").is_some() {
        let info: AgentInfo = serde_json::from_value(v.clone()).ok()?;
        return Some(vec![info]);
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
