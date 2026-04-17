//! Typed shapes for OpenClaw gateway RPC responses.
//!
//! Shapes intentionally stay loose — we pull only the fields we need
//! and ignore the rest. OpenClaw ships frequent gateway updates and
//! we'd rather tolerate unknown fields than hand-chase schema drift.

use serde::Deserialize;

/// One entry in `cron.status` / `cron.list` responses.
#[derive(Debug, Clone, Deserialize)]
pub struct CronJob {
    pub name: String,
    #[serde(default)]
    pub state: CronState,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CronState {
    #[serde(rename = "nextRunAtMs")]
    pub next_run_at_ms: Option<i64>,
    #[serde(rename = "lastRunAtMs")]
    pub last_run_at_ms: Option<i64>,
    #[serde(rename = "lastStatus")]
    pub last_status: Option<String>,
    #[serde(rename = "lastDurationMs")]
    pub last_duration_ms: Option<i64>,
    #[serde(rename = "lastError")]
    pub last_error: Option<String>,
    /// Set when the cron is currently executing.
    #[serde(default)]
    pub running: bool,
}

/// One entry in `channels.status`.
#[derive(Debug, Clone, Deserialize)]
pub struct Channel {
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub connected: bool,
    #[serde(rename = "lastError")]
    pub last_error: Option<String>,
}

/// Main agent reported by `status` or `models.authStatus`.
#[derive(Debug, Clone, Deserialize)]
pub struct MainAgent {
    pub id: String,
    pub model: Option<String>,
    /// `"idle"`, `"running"`, `"error"` — keep raw, map in domain layer.
    pub state: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cron_status_fixture() {
        let json = r#"
        {
            "name": "teamapp-velovate-sync-hourly",
            "state": {
                "nextRunAtMs": 1776441517517,
                "lastRunAtMs": 1776427117517,
                "lastStatus": "ok",
                "lastDurationMs": 160203,
                "lastError": null,
                "running": false
            }
        }
        "#;
        let cron: CronJob = serde_json::from_str(json).unwrap();
        assert_eq!(cron.name, "teamapp-velovate-sync-hourly");
        assert_eq!(cron.state.last_status.as_deref(), Some("ok"));
        assert!(!cron.state.running);
    }

    #[test]
    fn ignores_unknown_fields() {
        let json = r#"
        {
            "name": "openclaw-auto-update",
            "future_field_we_havent_seen": 42,
            "state": {
                "lastStatus": "error",
                "lastError": "network timeout",
                "mystery": true
            }
        }
        "#;
        let cron: CronJob = serde_json::from_str(json).unwrap();
        assert_eq!(cron.name, "openclaw-auto-update");
        assert_eq!(cron.state.last_status.as_deref(), Some("error"));
    }

    #[test]
    fn channel_parse() {
        let json = r#"{ "name": "slack", "enabled": true, "connected": true }"#;
        let ch: Channel = serde_json::from_str(json).unwrap();
        assert!(ch.enabled && ch.connected);
    }
}
