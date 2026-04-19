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
use crate::domain::AgentId;
use crate::net::WsEvent;
use crate::net::events::{
    ActivityKind, agent_stream_to_activity, cron_job_from_event,
};
use crate::net::rpc::{
    AgentEventPayload, ApprovalEventPayload, Channel, ChatEventPayload, CronEventPayload,
    CronJob, MainAgent,
};

/// Cadence of the channel-status heartbeat RPC. Channels are not yet
/// broadcast as push events on the gateway, so we refresh via the
/// already-open socket on a slow interval.
const CHANNEL_HEARTBEAT: Duration = Duration::from_secs(30);
const INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// The agent we attribute chat/agent events to. Today the roster has
/// exactly one LLM agent (`main`), so all chat text is necessarily
/// from it. When the desktop grows a multi-agent roster this should
/// route by the event's `sessionKey` instead.
const CHAT_AGENT_ID: &str = "main";

/// Heuristic: `wss://<host>.ts.net/...` is a Tailscale Serve URL, which
/// adds whois headers the gateway trusts when `allowTailscale: true`.
fn is_tailscale_serve_url(url: &str) -> bool {
    url.starts_with("wss://") && url.contains(".ts.net")
}

/// Iced subscription stream for the real gateway.
///
/// Keep as a free function (not a closure) — Iced's `Subscription::run`
/// uses the function pointer as subscription identity.
pub fn connect() -> impl Stream<Item = WsEvent> {
    stream::channel(64, async move |mut out| {
        let gateway_url = config::gateway_url();
        // Always send the token if we have one — the gateway's
        // `auth.mode: "token"` requires it even for Control-UI clients
        // bypassing device-identity via `dangerouslyDisableDeviceAuth`
        // (bypass covers pairing, not shared-secret). Tailscale Serve
        // path still carries the token cleanly; no mismatch because
        // we're sending the same value the Control UI uses (stored
        // server-side as `gateway.auth.token`, resolved from env/refs).
        let token = config::try_load_token();

        let instance_id = config::instance_id().unwrap_or_else(|e| {
            tracing::warn!(error = %e, "instance-id stash failed; using ephemeral");
            uuid::Uuid::new_v4().to_string()
        });

        let mut backoff = INITIAL_BACKOFF;

        loop {
            match session(token.as_deref(), &gateway_url, &instance_id, &mut out).await {
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
    out: &mut iced::futures::channel::mpsc::Sender<WsEvent>,
) -> Result<(), SessionError> {
    tracing::info!(
        url = gateway_url,
        auth = if token.is_some() { "token" } else { "tailscale" },
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
    let _nonce: String = loop {
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

    // Step 2: send our connect request. Nonce is received but not echoed —
    // for token-only auth (no device identity) the reference client omits
    // `device` entirely. When we have no token, we also omit `auth` —
    // that flips the gateway into Tailscale-whois auth (auth.ts:577,
    // `!hasExplicitSharedSecretAuth`).
    let mut params_obj = json!({
        "minProtocol": 3,
        "maxProtocol": 3,
        "client": {
            // OpenClaw recognizes `openclaw-tui` as an operator UI
            // (`utils/message-channel.ts:52, isOperatorUiClient`). The
            // mission-control desktop is that: a read-only visualization
            // surface for an operator. Identifying as TUI (instead of
            // generic `openclaw-probe`) lets the gateway short-circuit
            // the device-pairing requirement when
            // `controlUi.dangerouslyDisableDeviceAuth: true`, without
            // us needing to implement an Ed25519 pairing handshake yet.
            "id": "openclaw-tui",
            "displayName": "Mission Control Desktop",
            "version": env!("CARGO_PKG_VERSION"),
            "platform": std::env::consts::OS,
            "mode": "ui",
            "instanceId": instance_id,
        },
        // operator role + `operator.read` scope. When
        // `controlUi.dangerouslyDisableDeviceAuth: true`, the gateway
        // preserves self-declared scopes for Control-UI-classified
        // (TUI) clients without device identity
        // (`shouldClearUnboundScopesForMissingDeviceIdentity` returns
        // false when `allowBypass` is true). Grants access to scoped
        // RPCs (`cron.list`, `channels.status`) and scope-gated
        // events (`sessions.*`, `exec.approval.*`).
        "role": "operator",
        "scopes": ["operator.read"],
        "caps": [],
    });
    if let Some(tok) = token {
        params_obj["auth"] = json!({ "token": tok });
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

    // UUID → human-readable cron name cache, populated from the
    // snapshot and consulted when cron events arrive. Empty until
    // cron.list returns (a few ms after hello-ok), so the very first
    // event after startup may still go through as UUID.
    let mut cron_id_to_name: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

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
                        handle_frame(&txt, out, &mut cron_id_to_name).await?;
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
        }
    }
}

async fn send_rpc(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    method: &str,
) -> Result<(), SessionError> {
    let frame = json!({
        "type": "req",
        "id": id.to_string(),
        "method": method,
        "params": {}
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
) -> Result<(), SessionError> {
    let frame: Value = serde_json::from_str(txt)?;

    let frame_type = frame.get("type").and_then(Value::as_str);

    match frame_type {
        Some("res") => {
            let ok = frame.get("ok") == Some(&Value::Bool(true));
            if !ok {
                // Scoped RPCs return INVALID_REQUEST when scope is
                // missing; we surface the hello-ok scope log instead.
                tracing::trace!(raw = %txt, "gateway res frame not ok");
                return Ok(());
            }
            let Some(payload) = frame.get("payload") else {
                return Ok(());
            };

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
            handle_event(event, payload, out, cron_id_to_name).await?;
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
) -> Result<(), SessionError> {
    match event {
        // Scope-free: push delta for a single cron job.
        "cron" => {
            let Some(payload) = payload else { return Ok(()) };
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
            if evt.job_name.is_none() {
                if let Some(name) = cron_id_to_name.get(&evt.job_id) {
                    evt.job_name = Some(name.clone());
                }
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

        // Scope-free: chat output from the main agent. We only surface
        // `final` assistant text as a thought bubble — deltas are noisy
        // and the app doesn't render per-token.
        "chat" => {
            let Some(payload) = payload else { return Ok(()) };
            let evt: ChatEventPayload = match serde_json::from_value(payload.clone()) {
                Ok(e) => e,
                Err(e) => {
                    tracing::trace!(error = %e, "chat event parse failed");
                    return Ok(());
                }
            };
            if evt.state != "final" {
                return Ok(());
            }
            let Some(msg) = evt.message.as_ref() else {
                return Ok(());
            };
            let text = msg.plain_text();
            if text.trim().is_empty() {
                return Ok(());
            }
            tracing::debug!(len = text.len(), "chat final");
            let _ = out
                .send(WsEvent::AgentMessage {
                    agent_id: AgentId::new(CHAT_AGENT_ID),
                    text,
                })
                .await;
        }

        // Scope-free: agent run stream (tool-call phases, lifecycle).
        // Surfaces as activity nudges, not text.
        "agent" => {
            let Some(payload) = payload else { return Ok(()) };
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
            let Some(payload) = payload else { return Ok(()) };
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
            send_activity(out, ActivityKind::ToolCalling).await;
        }

        // Scoped (APPROVALS_SCOPE): exec approvals for the operator.
        "exec.approval.requested" => {
            let Some(payload) = payload else { return Ok(()) };
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
            chunk.get("text").and_then(Value::as_str).map(str::to_string)
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
    let candidate = v.get("channels").or(Some(v))?;
    let list: Vec<Channel> = serde_json::from_value(candidate.clone()).ok()?;
    if list.iter().any(|c| c.name.is_empty()) {
        return None;
    }
    Some(list)
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
