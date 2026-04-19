//! Typed shapes for OpenClaw gateway RPC responses.
//!
//! Shapes intentionally stay loose — we pull only the fields we need
//! and ignore the rest. OpenClaw ships frequent gateway updates and
//! we'd rather tolerate unknown fields than hand-chase schema drift.

use serde::Deserialize;

/// One entry in `cron.list` responses. `id` is the stable UUID used by
/// `cron` broadcast events; `name` is the human-readable identity the
/// desktop roster keys by.
#[derive(Debug, Clone, Deserialize)]
pub struct CronJob {
    pub name: String,
    #[serde(default)]
    pub id: Option<String>,
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

/// Response of the `agent.identity.get` RPC. Carries the persona
/// metadata (display name + emoji) that makes the office feel less
/// anonymous — `main` is really "Sebastian 🦀" to the operator.
///
/// The WS `agents.list` RPC returns only basic config (id, workspace,
/// model) — no identity fields — so we pull from `agent.identity.get`
/// instead, which is the authoritative source backed by
/// `resolveAssistantIdentity` on the server.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentInfo {
    #[serde(rename = "agentId")]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
}

/// Gateway broadcast `cron` event. Shape from
/// `openclaw/src/cron/service/state.ts` (`CronEvent`).
#[derive(Debug, Clone, Deserialize)]
pub struct CronEventPayload {
    /// Stable cron identifier (UUID); matches the `id` field in
    /// `~/.openclaw/cron/jobs.json`. Agent roster keys by `name`, not
    /// id, so the caller needs to map via `jobs.json` or snapshot.
    #[serde(rename = "jobId")]
    pub job_id: String,
    /// Human-readable cron name (`"zpool-health-check"`). Not always
    /// present in events — jobs created via the old CLI may only carry
    /// `jobId`. When absent, caller falls back to `job_id` for roster
    /// matching, which means newly-added unnamed jobs render by UUID
    /// until the next RPC snapshot fills the mapping.
    #[serde(default, rename = "jobName")]
    pub job_name: Option<String>,
    /// `"added" | "updated" | "removed" | "started" | "finished"`.
    pub action: String,
    #[serde(default, rename = "runAtMs")]
    pub run_at_ms: Option<i64>,
    #[serde(default, rename = "durationMs")]
    pub duration_ms: Option<i64>,
    /// Present on `action == "finished"`: `"ok"`, `"error"`, `"failed"`,
    /// `"timeout"`, etc.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default, rename = "nextRunAtMs")]
    pub next_run_at_ms: Option<i64>,
}

/// One entry in the `sessions.list` response (and embedded in the
/// `sessionSnapshot` spread on `session.message` payloads). We only
/// pull the fields the status bar reads.
#[derive(Debug, Clone, Deserialize)]
pub struct SessionInfo {
    pub key: String,
    #[serde(default, rename = "totalTokens")]
    pub total_tokens: Option<i64>,
    #[serde(default, rename = "contextTokens")]
    pub context_tokens: Option<i64>,
}

/// Response of the `logs.tail` RPC
/// (`openclaw/src/logging/log-tail.ts:13`). Returns a slice of the
/// rolling log file starting at `cursor`; the caller stores the new
/// cursor and passes it back on the next poll to get only new lines.
///
/// `reset: true` means the log file rolled over and our previous
/// cursor is no longer valid — clear the local buffer and start over.
#[derive(Debug, Clone, Deserialize)]
pub struct LogTailPayload {
    #[serde(default)]
    pub cursor: i64,
    #[serde(default)]
    pub lines: Vec<String>,
    #[serde(default)]
    pub reset: bool,
}

/// Gateway broadcast `agent` event. Shape from
/// `openclaw/src/gateway/server-chat.ts` agent-run stream payloads.
///
/// We only need `stream` to classify activity (thinking / tool-calling
/// / errored); finer-grained per-tool rendering would pull in `data`.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentEventPayload {
    /// `"assistant" | "tool" | "item" | "error" | "lifecycle" | ...`.
    pub stream: String,
}

/// Gateway broadcast `exec.approval.requested` / `.resolved` payload.
/// Loose — OpenClaw's approval model is detailed and we only surface
/// "there's a pending thing" in the UI for now.
#[derive(Debug, Clone, Deserialize)]
pub struct ApprovalEventPayload {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "sessionKey")]
    pub session_key: Option<String>,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
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

    #[test]
    fn cron_event_parse() {
        // Shape from openclaw/src/cron/service/state.ts CronEvent.
        let json = r#"
        {
            "jobId": "zpool-health-check",
            "action": "finished",
            "runAtMs": 1776440000000,
            "durationMs": 2415,
            "status": "ok",
            "nextRunAtMs": 1776443600000
        }
        "#;
        let evt: CronEventPayload = serde_json::from_str(json).unwrap();
        assert_eq!(evt.action, "finished");
        assert_eq!(evt.status.as_deref(), Some("ok"));
        assert_eq!(evt.duration_ms, Some(2415));
    }

    #[test]
    fn agent_event_parses_stream_ignoring_extras() {
        // Extra fields (runId, ts, sessionKey, data) must parse cleanly.
        let json = r#"
        {
            "runId": "r1",
            "stream": "tool",
            "ts": 1776440000000,
            "sessionKey": "s1",
            "data": {"phase": "start", "name": "bash"}
        }
        "#;
        let evt: AgentEventPayload = serde_json::from_str(json).unwrap();
        assert_eq!(evt.stream, "tool");
    }
}
