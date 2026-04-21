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

/// One row in the `agents.list` RPC response. Carries everything the
/// desktop needs to render an agent in the Chat picker + Overview:
/// stable id, operator-facing name/emoji from the nested `identity`
/// object, and the model string if configured.
///
/// Populated server-side by `listAgentsForGateway`
/// (`openclaw/src/gateway/session-utils.ts:652`). Identity is nested
/// one level (`identity: {name, emoji, avatar, ...}`); we flatten on
/// display through the `display_*` helpers.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub identity: Option<AgentIdentity>,
    /// Model ref — the server returns `{primary, fallbacks}` (see
    /// `resolveGatewayAgentModel` at `session-utils.ts:635`), not a
    /// plain string. We surface only `primary` via
    /// [`AgentInfo::primary_model`]; the fallbacks aren't interesting
    /// to the desktop yet.
    #[serde(default)]
    pub model: Option<AgentModelRef>,
    #[serde(default)]
    pub workspace: Option<String>,
}

impl AgentInfo {
    /// Display-name picked from the richest source available:
    /// `identity.name` (operator-chosen persona) → `name` (config
    /// label) → id (fallback).
    pub fn display_name(&self) -> &str {
        self.identity
            .as_ref()
            .and_then(|i| i.name.as_deref())
            .or(self.name.as_deref())
            .unwrap_or(self.id.as_str())
    }

    /// Full display string including the persona emoji when present —
    /// e.g. `"Sebastian 🦀"`. Used in the Chat picker and as the
    /// sprite label on the Overview canvas.
    pub fn display_with_emoji(&self) -> String {
        let name = self.display_name();
        match self.identity.as_ref().and_then(|i| i.emoji.as_deref()) {
            Some(e) if !e.is_empty() => format!("{name} {e}"),
            _ => name.to_string(),
        }
    }

    /// Primary model ref (e.g. `"anthropic/claude-opus-4-7"`) or
    /// `None` if none configured. The picker subtitle shows this
    /// alongside the workspace basename.
    pub fn primary_model(&self) -> Option<&str> {
        self.model.as_ref().and_then(|m| m.primary.as_deref())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentModelRef {
    #[serde(default)]
    pub primary: Option<String>,
    #[serde(default)]
    pub fallbacks: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentIdentity {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
}

/// Full `agents.list` response envelope. `default_id` is the agent
/// the desktop should select first in the Chat picker.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentsListResponse {
    #[serde(rename = "defaultId")]
    pub default_id: String,
    #[serde(default)]
    pub agents: Vec<AgentInfo>,
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
/// `sessionSnapshot` spread on `session.message` payloads). The
/// status bar reads just the totals; the Sessions view renders the
/// fuller set.
#[derive(Debug, Clone, Deserialize)]
pub struct SessionInfo {
    // `sessions.list` names this `key`; `session.message` events
    // call the same field `sessionKey`. Accept both so a single
    // struct handles both payload shapes.
    #[serde(alias = "sessionKey")]
    pub key: String,
    #[serde(default, rename = "totalTokens")]
    pub total_tokens: Option<i64>,
    #[serde(default, rename = "contextTokens")]
    pub context_tokens: Option<i64>,
    #[serde(default, rename = "inputTokens")]
    pub input_tokens: Option<i64>,
    #[serde(default, rename = "outputTokens")]
    pub output_tokens: Option<i64>,
    #[serde(default, rename = "updatedAt")]
    pub updated_at_ms: Option<i64>,
    #[serde(default, rename = "ageMs")]
    pub age_ms: Option<i64>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default, rename = "thinkingLevel")]
    pub thinking_level: Option<String>,
    #[serde(default, rename = "agentId")]
    pub agent_id: Option<String>,
}

/// A single point in a session's token-usage time series. Shape from
/// `openclaw/src/shared/session-usage-timeseries-types.ts`. We only
/// deserialize the fields the sparkline actually consumes —
/// `cumulative_tokens` (the primary y-axis value) and `timestamp`
/// (x-axis). Input / output / cache splits are preserved so a future
/// stacked view can grow into them without another RPC round-trip.
#[derive(Debug, Clone, Deserialize)]
pub struct SessionUsagePoint {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub input: i64,
    #[serde(default)]
    pub output: i64,
    #[serde(default, rename = "cacheRead")]
    pub cache_read: i64,
    #[serde(default, rename = "cacheWrite")]
    pub cache_write: i64,
    #[serde(default, rename = "totalTokens")]
    pub total_tokens: i64,
    #[serde(default, rename = "cumulativeTokens")]
    pub cumulative_tokens: i64,
    #[serde(default)]
    pub cost: f64,
    #[serde(default, rename = "cumulativeCost")]
    pub cumulative_cost: f64,
}

/// Response payload for `sessions.usage.timeseries`. Already
/// downsampled on the gateway to ≤200 points; no client-side
/// bucketing required. We only care about the `points` vec — the
/// gateway also echoes `sessionId`, but the client correlates by
/// request id, so echoing it back would just duplicate state.
#[derive(Debug, Clone, Deserialize)]
pub struct SessionUsageTimeseries {
    #[serde(default)]
    pub points: Vec<SessionUsagePoint>,
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
/// We use `stream` to classify activity (thinking / tool-calling /
/// errored) and `sessionKey` to route the activity to the right
/// agent in the UI (so "Sebastian is thinking…" doesn't appear while
/// a different agent is the one working).
#[derive(Debug, Clone, Deserialize)]
pub struct AgentEventPayload {
    /// `"assistant" | "tool" | "item" | "error" | "lifecycle" | ...`.
    pub stream: String,
    /// `"agent:<id>:<sid>"`. Optional because some agent events (e.g.
    /// generic error/lifecycle with no session context) omit it.
    #[serde(default, rename = "sessionKey")]
    pub session_key: Option<String>,
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
            "name": "example-data-sync-hourly",
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
        assert_eq!(cron.name, "example-data-sync-hourly");
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
    fn agents_list_parses_sebastian_shape() {
        // Mirrors the real response from `listAgentsForGateway` —
        // `model` is a nested `{primary, fallbacks}` object, not a
        // plain string. An earlier desktop revision declared
        // `model: Option<String>` and silently dropped every agent
        // because that deserialize failed; this test pins the shape.
        let json = r#"
        {
            "defaultId": "main",
            "mainKey": "main",
            "scope": "per-sender",
            "agents": [
                {
                    "id": "main",
                    "name": "Sebastian",
                    "identity": { "name": "Sebastian", "emoji": "🦀" },
                    "workspace": "/Users/chris/.openclaw",
                    "model": {
                        "primary": "anthropic/claude-opus-4-7",
                        "fallbacks": ["anthropic/claude-sonnet-4-6"]
                    }
                }
            ]
        }
        "#;
        let resp: AgentsListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.default_id, "main");
        assert_eq!(resp.agents.len(), 1);
        let a = &resp.agents[0];
        assert_eq!(a.id, "main");
        assert_eq!(a.display_name(), "Sebastian");
        assert_eq!(a.display_with_emoji(), "Sebastian 🦀");
        assert_eq!(a.primary_model(), Some("anthropic/claude-opus-4-7"));
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
