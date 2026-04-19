//! **Deprecated: debug fallback only.** The WebSocket client
//! (`net::openclaw`) is now the primary data path — it receives cron
//! updates as push events from the gateway instead of polling. This
//! SSH-based reader is retained for diagnosing WS issues and is only
//! selected when both `OPENCLAW_FORCE_SSH=1` and `OPENCLAW_SSH_HOST`
//! are set. Do not extend.
//!
//! ---
//!
//! Pragmatic real-data path: cat OpenClaw state files over SSH.
//!
//! The gateway's WebSocket handshake proved finicky to replicate from
//! Rust (see `net::openclaw` for the scaffolded attempt). The CLI
//! `openclaw cron list --json` takes 7+ seconds per call (new Node
//! process, WS handshake, shutdown) so that path is too slow.
//!
//! Instead we read the gateway's own state files directly:
//!
//! - `~/.openclaw/cron/jobs.json` — full cron job state (live data,
//!   updated by the gateway in-place after each run)
//! - `~/.openclaw/openclaw.json` — static channel enable/disable config
//!
//! Each `ssh <host> cat <path>` takes ~300ms over Tailscale. We poll
//! crons every 7s and channels every 30s.
//!
//! Activated by `OPENCLAW_SSH_HOST=<ssh-target>`. The target must have
//! passwordless SSH from this machine.

use std::time::Duration;

use iced::futures::{SinkExt, Stream};
use iced::stream;
use serde::Deserialize;

use crate::net::WsEvent;
use crate::net::rpc::{Channel, CronJob, MainAgent};

const CRON_PATH: &str = ".openclaw/cron/jobs.json";
const CONFIG_PATH: &str = ".openclaw/openclaw.json";

const CRON_POLL_EVERY: Duration = Duration::from_secs(7);
const CONFIG_POLL_EVERY: Duration = Duration::from_secs(30);
const SSH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Deserialize)]
struct JobsFile {
    #[serde(default)]
    jobs: Vec<CronJob>,
}

#[derive(Debug, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    channels: Option<ChannelsBlock>,
    #[serde(default)]
    agents: Option<AgentsBlock>,
}

#[derive(Debug, Deserialize)]
struct ChannelsBlock {
    #[serde(flatten)]
    providers: std::collections::HashMap<String, ChannelEntry>,
}

#[derive(Debug, Deserialize)]
struct ChannelEntry {
    #[serde(default)]
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct AgentsBlock {
    #[serde(default)]
    defaults: Option<AgentDefaults>,
}

#[derive(Debug, Deserialize)]
struct AgentDefaults {
    #[serde(default)]
    model: Option<ModelDefaults>,
}

#[derive(Debug, Deserialize)]
struct ModelDefaults {
    #[serde(default)]
    primary: Option<String>,
}

/// Check env for the SSH target host. Returns `None` if SSH mode isn't requested.
pub fn host() -> Option<String> {
    std::env::var("OPENCLAW_SSH_HOST")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Iced Subscription stream for the SSH+file path.
/// Keep as a free function — Iced dedupes by pointer identity.
pub fn connect() -> impl Stream<Item = WsEvent> {
    stream::channel(64, async move |mut out| {
        let Some(host) = host() else {
            let _ = out
                .send(WsEvent::Disconnected("OPENCLAW_SSH_HOST not set".into()))
                .await;
            return;
        };

        tracing::info!(host = %host, "starting SSH file-reader poller");
        let _ = out.send(WsEvent::Connected).await;

        let mut cron_interval = tokio::time::interval(CRON_POLL_EVERY);
        let mut config_interval = tokio::time::interval(CONFIG_POLL_EVERY);

        loop {
            tokio::select! {
                _ = cron_interval.tick() => {
                    match poll_crons(&host).await {
                        Ok(jobs) => {
                            tracing::debug!(count = jobs.len(), "cron snapshot via ssh");
                            let _ = out.send(WsEvent::CronSnapshot(jobs)).await;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "cron poll failed");
                            let _ = out
                                .send(WsEvent::Disconnected(format!("cron: {e}")))
                                .await;
                        }
                    }
                }
                _ = config_interval.tick() => {
                    match poll_config(&host).await {
                        Ok((channels, main)) => {
                            tracing::debug!(channels = channels.len(), "config snapshot via ssh");
                            let _ = out.send(WsEvent::ChannelSnapshot(channels)).await;
                            if let Some(m) = main {
                                let _ = out.send(WsEvent::MainAgent(m)).await;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "config poll failed");
                        }
                    }
                }
            }
        }
    })
}

async fn poll_crons(host: &str) -> Result<Vec<CronJob>, PollError> {
    let raw = cat_remote(host, CRON_PATH).await?;
    let parsed: JobsFile = serde_json::from_slice(&raw)?;
    Ok(parsed.jobs)
}

async fn poll_config(host: &str) -> Result<(Vec<Channel>, Option<MainAgent>), PollError> {
    let raw = cat_remote(host, CONFIG_PATH).await?;
    let parsed: ConfigFile = serde_json::from_slice(&raw)?;

    let channels = parsed
        .channels
        .map(|b| {
            b.providers
                .into_iter()
                .map(|(name, entry)| Channel {
                    name,
                    enabled: entry.enabled,
                    connected: entry.enabled, // optimistic; real state needs WS
                    last_error: None,
                })
                .collect()
        })
        .unwrap_or_default();

    // Config's agents block is `agents.defaults.model.primary` (a string).
    // We hardcode id="main" since that's the canonical agent across the setup.
    let main = parsed
        .agents
        .and_then(|a| a.defaults)
        .and_then(|d| d.model)
        .and_then(|m| m.primary)
        .map(|model| MainAgent {
            id: "main".to_string(),
            model: Some(model),
            state: Some("idle".into()),
        });

    Ok((channels, main))
}

async fn cat_remote(host: &str, path: &str) -> Result<Vec<u8>, PollError> {
    let output = tokio::time::timeout(
        SSH_TIMEOUT,
        tokio::process::Command::new("ssh")
            .arg("-o")
            .arg("ConnectTimeout=3")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg(host)
            // Remote command is a literal `cat <path>`. Host is env-controlled
            // (trusted) and path is a hard-coded constant — no injection vector.
            .arg(format!("cat {path}"))
            .output(),
    )
    .await
    .map_err(|_| PollError::Timeout)??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PollError::NonZero {
            code: output.status.code(),
            stderr: stderr.trim().to_string(),
        });
    }
    Ok(output.stdout)
}

#[derive(Debug, thiserror::Error)]
enum PollError {
    #[error("ssh timeout")]
    Timeout,
    #[error("spawn: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("command failed (code={code:?}): {stderr}")]
    NonZero { code: Option<i32>, stderr: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jobs_file() {
        let json = r#"
        {
            "version": 1,
            "jobs": [
                {
                    "name": "zpool-health-check",
                    "state": { "lastStatus": "ok", "running": false }
                },
                {
                    "name": "openclaw-auto-update",
                    "state": { "lastStatus": "error", "running": false }
                }
            ]
        }
        "#;
        let f: JobsFile = serde_json::from_str(json).unwrap();
        assert_eq!(f.jobs.len(), 2);
        assert_eq!(f.jobs[0].name, "zpool-health-check");
    }

    #[test]
    fn parses_config_channels() {
        let json = r#"
        {
            "channels": {
                "slack": { "enabled": true },
                "whatsapp": { "enabled": false }
            },
            "agents": {
                "defaults": {
                    "model": {
                        "primary": "openai-codex/gpt-5.4",
                        "fallbacks": ["vllm/qwen3-14b"]
                    }
                }
            }
        }
        "#;
        let cfg: ConfigFile = serde_json::from_str(json).unwrap();
        let providers = cfg.channels.unwrap().providers;
        assert!(providers.get("slack").unwrap().enabled);
        assert!(!providers.get("whatsapp").unwrap().enabled);
        let agents = cfg.agents.unwrap();
        let primary = agents
            .defaults
            .and_then(|d| d.model)
            .and_then(|m| m.primary);
        assert_eq!(primary.as_deref(), Some("openai-codex/gpt-5.4"));
    }
}
