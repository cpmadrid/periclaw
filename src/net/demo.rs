//! Scripted fixture stream used by Demo mode.
//!
//! Reads `assets/test/scenario_happy.json` at startup, then emits
//! snapshots on a timer matching the `at_ms` marks in the file.
//! Loops forever using `loop_seconds`.
//!
//! This is what lets us iterate on the scene without a live OpenClaw
//! gateway. Each loop intentionally covers:
//!
//! - background job work bubbling on and off
//! - a visible tool + assistant reply turn
//! - a silent lifecycle run
//! - an error + recovery path

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iced::futures::channel::mpsc::Sender;
use iced::futures::{SinkExt, Stream};
use iced::stream;
use serde::Deserialize;

use crate::domain::AgentId;
use crate::net::WsEvent;
use crate::net::commands::GatewayCommand;
use crate::net::events::ActivityKind;
use crate::net::rpc::{
    AgentIdentity, AgentInfo, AgentModelRef, Channel, CronJob, LogTailPayload, MainAgent,
    SessionInfo, SessionUsagePoint,
};
use crate::ui::chat_view::ChatMessage;

const FIXTURE: &str = include_str!("../../assets/test/scenario_happy.json");
const MAIN_AGENT_ID: &str = "main";
const MAIN_SESSION_KEY: &str = "agent:main:main";
const DIGEST_SESSION_KEY: &str = "agent:main:weekly-digest";

#[derive(Debug, Deserialize)]
struct Scenario {
    loop_seconds: u64,
    ticks: Vec<Tick>,
}

#[derive(Debug, Deserialize)]
struct Tick {
    at_ms: u64,
    crons: Vec<CronJob>,
    channels: Vec<Channel>,
    main: MainAgent,
}

#[derive(Debug, Clone)]
struct TimedDemoEvent {
    at_ms: u64,
    event: WsEvent,
}

/// Iced Subscription stream that emits scripted events.
///
/// The function identity is used by Iced to dedupe — keep this as a
/// free function, not a closure.
pub fn connect() -> impl Stream<Item = WsEvent> {
    stream::channel(64, async move |mut out| {
        let scenario: Scenario = match serde_json::from_str(FIXTURE) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(?e, "failed to parse demo scenario fixture");
                let _ = out
                    .send(WsEvent::Disconnected(format!("bad fixture: {e}")))
                    .await;
                return;
            }
        };

        tracing::info!(
            loop_seconds = scenario.loop_seconds,
            ticks = scenario.ticks.len(),
            "demo stream starting"
        );

        let _ = out.send(WsEvent::Connected).await;
        let mut cmd_rx = crate::net::commands::take_rx();
        emit_bootstrap(&mut out).await;

        loop {
            let loop_start = tokio::time::Instant::now();
            let events = loop_events();
            let mut tick_idx = 0;
            let mut event_idx = 0;

            while tick_idx < scenario.ticks.len() || event_idx < events.len() {
                let next_tick_ms = scenario.ticks.get(tick_idx).map(|t| t.at_ms);
                let next_event_ms = events.get(event_idx).map(|e| e.at_ms);
                let next_ms = match (next_tick_ms, next_event_ms) {
                    (Some(t), Some(e)) => t.min(e),
                    (Some(t), None) => t,
                    (None, Some(e)) => e,
                    (None, None) => break,
                };
                let target = loop_start + Duration::from_millis(next_ms);
                wait_until_handling_commands(target, &mut out, &mut cmd_rx).await;

                while let Some(tick) = scenario.ticks.get(tick_idx) {
                    if tick.at_ms != next_ms {
                        break;
                    }
                    emit_tick(&mut out, tick).await;
                    tick_idx += 1;
                }

                while let Some(scripted) = events.get(event_idx) {
                    if scripted.at_ms != next_ms {
                        break;
                    }
                    let _ = out.send(scripted.event.clone()).await;
                    event_idx += 1;
                }
            }
            // Hold until the loop_seconds mark before restarting.
            let loop_end = loop_start + Duration::from_secs(scenario.loop_seconds);
            if tokio::time::Instant::now() < loop_end {
                wait_until_handling_commands(loop_end, &mut out, &mut cmd_rx).await;
            }
        }
    })
}

async fn wait_until_handling_commands(
    target: tokio::time::Instant,
    out: &mut Sender<WsEvent>,
    cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<GatewayCommand>,
) {
    loop {
        if tokio::time::Instant::now() >= target {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep_until(target) => return,
            Some(cmd) = cmd_rx.recv() => handle_demo_command(out, cmd).await,
            else => return,
        }
    }
}

async fn handle_demo_command(out: &mut Sender<WsEvent>, cmd: GatewayCommand) {
    match cmd {
        GatewayCommand::SendChat {
            agent_id, message, ..
        } => {
            let agent_id = AgentId::new(agent_id);
            let _ = out
                .send(WsEvent::AgentActivity {
                    agent_id: agent_id.clone(),
                    kind: ActivityKind::Thinking,
                })
                .await;
            let _ = out
                .send(WsEvent::AgentToolInvoked {
                    agent_id: agent_id.clone(),
                    text: "⚙ demo.reply".to_string(),
                })
                .await;
            let _ = out
                .send(WsEvent::AgentMessage {
                    agent_id: agent_id.clone(),
                    text: format!(
                        "Demo reply: I received \"{}\" and kept the exchange offline.",
                        compact_prompt(&message, 72)
                    ),
                })
                .await;
            let _ = out
                .send(WsEvent::AgentActivity {
                    agent_id,
                    kind: ActivityKind::Idle,
                })
                .await;
        }
        GatewayCommand::FetchChatHistory { agent_id } => {
            let _ = out
                .send(WsEvent::ChatHistory {
                    agent_id: AgentId::new(agent_id),
                    messages: demo_chat_history(),
                })
                .await;
        }
        GatewayCommand::FetchAgentIdentity { agent_id } => {
            let _ = out
                .send(WsEvent::AgentIdentity {
                    agent_id: AgentId::new(agent_id),
                    name: Some("Sebastian".to_string()),
                    emoji: Some("🦀".to_string()),
                })
                .await;
        }
        GatewayCommand::FetchSessionHistory { session_key } => {
            let _ = out
                .send(WsEvent::SessionHistory {
                    session_key: session_key.clone(),
                    messages: demo_session_history(&session_key),
                })
                .await;
        }
        GatewayCommand::FetchSessionUsage { session_key } => {
            let _ = out
                .send(WsEvent::SessionUsageTimeseries {
                    session_key,
                    points: usage_points(now_ms() - 90_000, &[8_000, 19_500, 28_000, 42_000]),
                })
                .await;
        }
        GatewayCommand::ResetSession { session_key } => {
            let agent_id = agent_id_from_session_key(&session_key);
            let _ = out
                .send(WsEvent::ChatHistory {
                    agent_id,
                    messages: Vec::new(),
                })
                .await;
        }
        GatewayCommand::Reconnect => {
            let _ = out.send(WsEvent::Connected).await;
        }
        GatewayCommand::ResolveApproval { .. } | GatewayCommand::RunCron { .. } => {}
    }
}

async fn emit_tick(out: &mut Sender<WsEvent>, tick: &Tick) {
    let _ = out.send(WsEvent::CronSnapshot(tick.crons.clone())).await;
    let _ = out
        .send(WsEvent::ChannelSnapshot(tick.channels.clone()))
        .await;
    let _ = out.send(WsEvent::MainAgent(tick.main.clone())).await;
}

async fn emit_bootstrap(out: &mut Sender<WsEvent>) {
    for event in bootstrap_events(now_ms()) {
        let _ = out.send(event).await;
    }
}

fn bootstrap_events(now_ms: i64) -> Vec<WsEvent> {
    vec![
        WsEvent::AgentsList {
            default_id: MAIN_AGENT_ID.to_string(),
            agents: vec![demo_agent()],
        },
        WsEvent::ChatHistory {
            agent_id: AgentId::new(MAIN_AGENT_ID),
            messages: demo_chat_history(),
        },
        WsEvent::SessionUsage(demo_session(
            MAIN_SESSION_KEY,
            "main",
            39_200,
            131_072,
            now_ms - 15_000,
            "live",
        )),
        WsEvent::SessionUsage(demo_session(
            DIGEST_SESSION_KEY,
            "main",
            81_500,
            131_072,
            now_ms - 86_000,
            "archived",
        )),
        WsEvent::SessionHistory {
            session_key: DIGEST_SESSION_KEY.to_string(),
            messages: demo_session_history(DIGEST_SESSION_KEY),
        },
        WsEvent::SessionUsageTimeseries {
            session_key: MAIN_SESSION_KEY.to_string(),
            points: usage_points(now_ms - 90_000, &[12_000, 24_500, 31_800, 39_200]),
        },
        WsEvent::SessionUsageTimeseries {
            session_key: DIGEST_SESSION_KEY.to_string(),
            points: usage_points(now_ms - 260_000, &[18_000, 37_000, 64_000, 81_500]),
        },
        WsEvent::LogTail(LogTailPayload {
            cursor: 3,
            lines: vec![
                "INFO demo stream connected".to_string(),
                "INFO demo agents.list count=1 default=main".to_string(),
                "WARN demo whatsapp disabled by fixture".to_string(),
            ],
            reset: true,
        }),
    ]
}

fn demo_chat_history() -> Vec<ChatMessage> {
    vec![
        ChatMessage::user("Can you check the office systems?"),
        ChatMessage::assistant(
            "Demo mode is online. I will cycle through chat, tools, cron jobs, and sessions so the desktop can be tested offline.",
        ),
    ]
}

fn demo_session_history(session_key: &str) -> Vec<ChatMessage> {
    if session_key == DIGEST_SESSION_KEY {
        return vec![
            ChatMessage::user("Summarize the last weekly digest run."),
            ChatMessage::assistant(
                "Demo digest: cron completed, two channels were healthy, and WhatsApp stayed disabled by configuration.",
            ),
        ];
    }
    demo_chat_history()
}

fn agent_id_from_session_key(session_key: &str) -> AgentId {
    let agent = session_key
        .strip_prefix("agent:")
        .and_then(|rest| rest.split_once(':').map(|(agent, _)| agent))
        .filter(|agent| !agent.is_empty())
        .unwrap_or(MAIN_AGENT_ID);
    AgentId::new(agent)
}

fn compact_prompt(prompt: &str, max: usize) -> String {
    let compact = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max {
        return compact;
    }
    let mut out: String = compact.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn loop_events() -> Vec<TimedDemoEvent> {
    let main = AgentId::new(MAIN_AGENT_ID);
    vec![
        // Visible reply path: bubble-bearing tool + assistant turn.
        event_at(
            1_200,
            WsEvent::AgentInboundUserMessage {
                agent_id: main.clone(),
            },
        ),
        event_at(
            1_800,
            WsEvent::AgentActivity {
                agent_id: main.clone(),
                kind: ActivityKind::Thinking,
            },
        ),
        event_at(
            3_000,
            WsEvent::AgentToolInvoked {
                agent_id: main.clone(),
                text: "⚙ logs.tail".to_string(),
            },
        ),
        event_at(
            3_200,
            WsEvent::AgentActivity {
                agent_id: main.clone(),
                kind: ActivityKind::ToolCalling,
            },
        ),
        event_at(
            5_300,
            WsEvent::AgentMessage {
                agent_id: main.clone(),
                text: "Demo check complete: Slack and Telegram are connected; WhatsApp is intentionally disabled.".to_string(),
            },
        ),
        event_at(
            5_700,
            WsEvent::AgentActivity {
                agent_id: main.clone(),
                kind: ActivityKind::Idle,
            },
        ),
        // Silent run: sparkle only, no reply bubble.
        event_at(
            13_000,
            WsEvent::AgentInboundUserMessage {
                agent_id: main.clone(),
            },
        ),
        event_at(
            13_700,
            WsEvent::AgentActivity {
                agent_id: main.clone(),
                kind: ActivityKind::Thinking,
            },
        ),
        event_at(15_600, WsEvent::AgentSilentTurn { agent_id: main.clone() }),
        event_at(
            16_000,
            WsEvent::AgentActivity {
                agent_id: main.clone(),
                kind: ActivityKind::Idle,
            },
        ),
        // Error path: loud anomaly bubble, then recovery to idle.
        event_at(
            21_000,
            WsEvent::AgentInboundUserMessage {
                agent_id: main.clone(),
            },
        ),
        event_at(
            22_000,
            WsEvent::AgentActivity {
                agent_id: main.clone(),
                kind: ActivityKind::Thinking,
            },
        ),
        event_at(
            23_000,
            WsEvent::AgentActivity {
                agent_id: main.clone(),
                kind: ActivityKind::Errored,
            },
        ),
        event_at(
            25_500,
            WsEvent::AgentActivity {
                agent_id: main,
                kind: ActivityKind::Idle,
            },
        ),
    ]
}

fn event_at(at_ms: u64, event: WsEvent) -> TimedDemoEvent {
    TimedDemoEvent { at_ms, event }
}

fn demo_agent() -> AgentInfo {
    AgentInfo {
        id: MAIN_AGENT_ID.to_string(),
        name: Some("Sebastian".to_string()),
        identity: Some(AgentIdentity {
            name: Some("Sebastian".to_string()),
            emoji: Some("🦀".to_string()),
            avatar: None,
            theme: Some("terminal".to_string()),
        }),
        model: Some(AgentModelRef {
            primary: Some("demo/local-agent".to_string()),
            fallbacks: Some(vec!["demo/fallback-agent".to_string()]),
        }),
        workspace: Some("/demo/openclaw".to_string()),
    }
}

fn demo_session(
    key: &str,
    agent_id: &str,
    total_tokens: i64,
    context_tokens: i64,
    updated_at_ms: i64,
    kind: &str,
) -> SessionInfo {
    SessionInfo {
        key: key.to_string(),
        total_tokens: Some(total_tokens),
        context_tokens: Some(context_tokens),
        input_tokens: Some(total_tokens * 3 / 5),
        output_tokens: Some(total_tokens * 2 / 5),
        updated_at_ms: Some(updated_at_ms),
        age_ms: Some(now_ms().saturating_sub(updated_at_ms).max(0)),
        model: Some("demo/local-agent".to_string()),
        kind: Some(kind.to_string()),
        thinking_level: Some("medium".to_string()),
        agent_id: Some(agent_id.to_string()),
    }
}

fn usage_points(start_ms: i64, totals: &[i64]) -> Vec<SessionUsagePoint> {
    totals
        .iter()
        .enumerate()
        .map(|(idx, total)| {
            let output = total / 3;
            let input = total - output;
            SessionUsagePoint {
                timestamp: start_ms + (idx as i64 * 30_000),
                input,
                output,
                cache_read: input / 10,
                cache_write: input / 20,
                total_tokens: *total,
                cumulative_tokens: *total,
                cost: (*total as f64) * 0.0000002,
                cumulative_cost: (*total as f64) * 0.0000002,
            }
        })
        .collect()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// True when Demo mode is requested via env.
///
/// `PERICLAW_DEMO=1` is the canonical name. `OPENCLAW_MOCK=1` is kept
/// as a legacy alias so older shell aliases and scripts continue to
/// route to the offline fixture.
pub fn enabled() -> bool {
    env_truthy("PERICLAW_DEMO") || env_truthy("OPENCLAW_MOCK")
}

fn env_truthy(name: &str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_event_after(
        events: &[TimedDemoEvent],
        start: usize,
        pred: impl Fn(&WsEvent) -> bool,
    ) -> usize {
        events
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, evt)| pred(&evt.event))
            .map(|(idx, _)| idx)
            .expect("matching event not found")
    }

    #[test]
    fn loop_events_are_sorted() {
        let events = loop_events();
        assert!(
            events.windows(2).all(|w| w[0].at_ms <= w[1].at_ms),
            "Demo events must stay sorted for scheduler ordering",
        );
    }

    #[test]
    fn bootstrap_covers_chat_sessions_and_logs() {
        let events = bootstrap_events(1_800_000_000_000);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, WsEvent::AgentsList { .. }))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, WsEvent::ChatHistory { .. }))
        );
        assert!(events.iter().any(|e| matches!(e, WsEvent::SessionUsage(_))));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, WsEvent::SessionUsageTimeseries { .. }))
        );
        assert!(events.iter().any(|e| matches!(e, WsEvent::LogTail(_))));
    }

    #[test]
    fn loop_covers_activity_lifecycle() {
        let events = loop_events();
        assert!(
            events
                .iter()
                .any(|e| matches!(&e.event, WsEvent::AgentInboundUserMessage { .. }))
        );
        assert!(events.iter().any(|e| matches!(
            &e.event,
            WsEvent::AgentActivity {
                kind: ActivityKind::Thinking,
                ..
            }
        )));
        assert!(
            events
                .iter()
                .any(|e| matches!(&e.event, WsEvent::AgentToolInvoked { .. }))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(&e.event, WsEvent::AgentMessage { .. }))
        );
        assert!(events.iter().any(|e| matches!(
            &e.event,
            WsEvent::AgentActivity {
                kind: ActivityKind::Idle,
                ..
            }
        )));
    }

    #[test]
    fn loop_exercises_visible_silent_and_error_paths_in_order() {
        let events = loop_events();

        let mut cursor = 0;
        cursor = find_event_after(&events, cursor, |event| {
            matches!(event, WsEvent::AgentInboundUserMessage { .. })
        });
        cursor = find_event_after(&events, cursor + 1, |event| {
            matches!(
                event,
                WsEvent::AgentActivity {
                    kind: ActivityKind::Thinking,
                    ..
                }
            )
        });
        cursor = find_event_after(&events, cursor + 1, |event| {
            matches!(event, WsEvent::AgentToolInvoked { .. })
        });
        cursor = find_event_after(&events, cursor + 1, |event| {
            matches!(event, WsEvent::AgentMessage { .. })
        });
        cursor = find_event_after(&events, cursor + 1, |event| {
            matches!(
                event,
                WsEvent::AgentActivity {
                    kind: ActivityKind::Idle,
                    ..
                }
            )
        });

        cursor = find_event_after(&events, cursor + 1, |event| {
            matches!(event, WsEvent::AgentInboundUserMessage { .. })
        });
        cursor = find_event_after(&events, cursor + 1, |event| {
            matches!(
                event,
                WsEvent::AgentActivity {
                    kind: ActivityKind::Thinking,
                    ..
                }
            )
        });
        cursor = find_event_after(&events, cursor + 1, |event| {
            matches!(event, WsEvent::AgentSilentTurn { .. })
        });
        cursor = find_event_after(&events, cursor + 1, |event| {
            matches!(
                event,
                WsEvent::AgentActivity {
                    kind: ActivityKind::Idle,
                    ..
                }
            )
        });

        cursor = find_event_after(&events, cursor + 1, |event| {
            matches!(event, WsEvent::AgentInboundUserMessage { .. })
        });
        cursor = find_event_after(&events, cursor + 1, |event| {
            matches!(
                event,
                WsEvent::AgentActivity {
                    kind: ActivityKind::Thinking,
                    ..
                }
            )
        });
        cursor = find_event_after(&events, cursor + 1, |event| {
            matches!(
                event,
                WsEvent::AgentActivity {
                    kind: ActivityKind::Errored,
                    ..
                }
            )
        });
        let _ = find_event_after(&events, cursor + 1, |event| {
            matches!(
                event,
                WsEvent::AgentActivity {
                    kind: ActivityKind::Idle,
                    ..
                }
            )
        });
    }
}
