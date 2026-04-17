//! Real WebSocket client for the OpenClaw gateway on ubu-3xdv.
//!
//! **Status (M3): scaffolded, handshake not yet landing.**
//! All the plumbing is in place — auth header, Authorization token,
//! frame format, connect.challenge listener, hello-ok matcher, poll
//! loop, exponential-backoff reconnect — but the session currently
//! times out waiting for the server's `connect.challenge` event, for
//! reasons not yet isolated. Running without `OPENCLAW_MOCK=1` leaves
//! the sprites unknown-status in the Agent Office; the app stays up
//! and keeps retrying. Fully wiring this is a follow-up spike.
//!
//! Protocol (from openclaw/docs/gateway/protocol.md and the local
//! npm bundle's client reference implementation):
//!
//! 1. WS upgrade to `ws://100.87.202.125:18789/__openclaw__/ws` with
//!    `Authorization: Bearer <token>` header.
//! 2. Wait for server-initiated
//!    `{type:"event", event:"connect.challenge", payload:{nonce}}`.
//!    The server sends this once the connection is accepted; clients
//!    must NOT send `connect` until they've seen it (verified in
//!    `dist/client-*.js` — `sendConnect` gates on `this.connectNonce`).
//! 3. Client sends
//!    `{type:"req", id, method:"connect", params:{minProtocol:3,
//!    maxProtocol:3, client:{id, version, platform, mode}, role,
//!    scopes, caps, auth:{token}}}`. `device` is omitted for
//!    token-only auth.
//! 4. Server replies
//!    `{type:"res", id, ok:true, payload:{type:"hello-ok", server,
//!    features, snapshot, policy, auth}}` — note `hello-ok` is the
//!    payload type nested under the standard `res` envelope.
//! 5. After hello-ok, poll `cron.status` + `channels.status` with the
//!    same envelope.
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

use crate::config::{self, GATEWAY_URL};
use crate::net::WsEvent;
use crate::net::rpc::{Channel, CronJob, MainAgent};

const POLL_EVERY: Duration = Duration::from_secs(7);
const INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Iced subscription stream for the real gateway.
///
/// Keep as a free function (not a closure) — Iced's `Subscription::run`
/// uses the function pointer as subscription identity.
pub fn connect() -> impl Stream<Item = WsEvent> {
    stream::channel(64, async move |mut out| {
        let token = match config::load_token() {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = %e, "no gateway token available");
                let _ = out
                    .send(WsEvent::Disconnected(format!("no token: {e}")))
                    .await;
                return;
            }
        };

        let instance_id = config::instance_id().unwrap_or_else(|e| {
            tracing::warn!(error = %e, "instance-id stash failed; using ephemeral");
            uuid::Uuid::new_v4().to_string()
        });

        let mut backoff = INITIAL_BACKOFF;

        loop {
            match session(&token, &instance_id, &mut out).await {
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
    token: &str,
    instance_id: &str,
    out: &mut iced::futures::channel::mpsc::Sender<WsEvent>,
) -> Result<(), SessionError> {
    tracing::info!(url = GATEWAY_URL, "connecting to gateway");
    let mut req = GATEWAY_URL
        .into_client_request()
        .map_err(SessionError::Connect)?;
    let auth_header = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|e| SessionError::Send(e.to_string()))?;
    req.headers_mut().insert("Authorization", auth_header);
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
    // `device` entirely; the server validated our wait-for-challenge behavior.
    let connect_frame = json!({
        "type": "req",
        "id": "connect-1",
        "method": "connect",
        "params": {
            "minProtocol": 3,
            "maxProtocol": 3,
            "client": {
                "id": "openclaw-probe",
                "displayName": "Mission Control Desktop",
                "version": env!("CARGO_PKG_VERSION"),
                "platform": std::env::consts::OS,
                "mode": "probe",
                "instanceId": instance_id,
            },
            "role": "operator",
            "scopes": [],
            "caps": [],
            "auth": { "token": token }
        }
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
                                let conn_id = v.get("payload")
                                    .and_then(|p| p.get("server"))
                                    .and_then(|s| s.get("connId"))
                                    .and_then(Value::as_str)
                                    .unwrap_or("?");
                                tracing::info!(conn_id, "gateway connect handshake accepted");
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

    // Main poll loop: request cron.status + channels.status every POLL_EVERY.
    let mut next_poll = Instant::now();
    let mut rpc_id: u64 = 100;

    loop {
        tokio::select! {
            _ = sleep_until(next_poll) => {
                rpc_id += 1;
                send_rpc(&mut socket, rpc_id, "cron.status").await?;
                rpc_id += 1;
                send_rpc(&mut socket, rpc_id, "channels.status").await?;
                next_poll = Instant::now() + POLL_EVERY;
            }
            msg = socket.next() => {
                match msg {
                    Some(Ok(WsMsg::Text(txt))) => {
                        handle_frame(&txt, out).await?;
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
) -> Result<(), SessionError> {
    let frame: Value = serde_json::from_str(txt)?;

    let frame_type = frame.get("type").and_then(Value::as_str);

    match frame_type {
        Some("res") => {
            let ok = frame.get("ok") == Some(&Value::Bool(true));
            if !ok {
                tracing::warn!(raw = %txt, "gateway res frame not ok");
                return Ok(());
            }
            let Some(payload) = frame.get("payload") else {
                return Ok(());
            };

            if let Some(crons) = try_cron_list(payload) {
                tracing::debug!(count = crons.len(), "cron snapshot");
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
            tracing::trace!(event, raw = %txt, "gateway event");
        }
        other => {
            tracing::trace!(?other, raw = %txt, "unknown frame type");
        }
    }

    Ok(())
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
