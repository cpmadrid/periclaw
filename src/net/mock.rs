//! Scripted fixture stream used when `OPENCLAW_MOCK=1`.
//!
//! Reads `assets/test/scenario_happy.json` at startup, then emits
//! snapshots on a timer matching the `at_ms` marks in the file.
//! Loops forever using `loop_seconds`.
//!
//! This is what lets us iterate on the scene without a live OpenClaw
//! gateway. Real WS client lands in M3.

use std::time::Duration;

use iced::futures::{SinkExt, Stream};
use iced::stream;
use serde::Deserialize;

use crate::net::WsEvent;
use crate::net::rpc::{Channel, CronJob, MainAgent};

const FIXTURE: &str = include_str!("../../assets/test/scenario_happy.json");

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

/// Iced Subscription stream that emits scripted events.
///
/// The function identity is used by Iced to dedupe — keep this as a
/// free function, not a closure.
pub fn connect() -> impl Stream<Item = WsEvent> {
    stream::channel(64, async move |mut out| {
        let scenario: Scenario = match serde_json::from_str(FIXTURE) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(?e, "failed to parse mock scenario fixture");
                let _ = out
                    .send(WsEvent::Disconnected(format!("bad fixture: {e}")))
                    .await;
                return;
            }
        };

        tracing::info!(
            loop_seconds = scenario.loop_seconds,
            ticks = scenario.ticks.len(),
            "mock WS stream starting"
        );

        let _ = out.send(WsEvent::Connected).await;

        loop {
            let loop_start = tokio::time::Instant::now();
            for tick in &scenario.ticks {
                let target = loop_start + Duration::from_millis(tick.at_ms);
                tokio::time::sleep_until(target).await;

                let _ = out.send(WsEvent::CronSnapshot(tick.crons.clone())).await;
                let _ = out
                    .send(WsEvent::ChannelSnapshot(tick.channels.clone()))
                    .await;
                let _ = out.send(WsEvent::MainAgent(tick.main.clone())).await;
            }
            // Hold until the loop_seconds mark before restarting.
            let loop_end = loop_start + Duration::from_secs(scenario.loop_seconds);
            if tokio::time::Instant::now() < loop_end {
                tokio::time::sleep_until(loop_end).await;
            }
        }
    })
}

/// True when OPENCLAW_MOCK env var is set to any truthy value.
pub fn enabled() -> bool {
    matches!(
        std::env::var("OPENCLAW_MOCK").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}
