//! Events pushed from the WS stream (real or mock) into the Iced app.
//!
//! These are the domain-flavored events the UI actually cares about —
//! richer than raw RPC responses, simpler than the whole OpenClaw API.

use crate::domain::{AgentId, AgentStatus};
use crate::net::rpc::{
    AgentInfo, ApprovalEventPayload, Channel, CronEventPayload, CronJob, CronState, LogTailPayload,
    MainAgent, SessionInfo,
};

#[derive(Debug, Clone)]
pub enum WsEvent {
    /// Initial or periodic snapshot of all cron jobs.
    CronSnapshot(Vec<CronJob>),
    /// Push-driven delta for a single cron job (from the `cron` broadcast).
    CronDelta(CronJob),
    /// Initial or periodic snapshot of all channel providers.
    ChannelSnapshot(Vec<Channel>),
    /// Main agent status update.
    MainAgent(MainAgent),
    /// `agents.list` snapshot — discovery of every chat-capable agent
    /// plus the server-side default. Drives roster-display overrides
    /// (Sebastian 🦀, etc.), picker rows in the Chat tab, and the
    /// initial `selected_chat_agent` on first connect.
    AgentsList {
        default_id: String,
        agents: Vec<AgentInfo>,
    },
    /// Rich persona fill-in from `agent.identity.get` — merged on top
    /// of the less-complete identity that `agents.list` carries. Name
    /// falls back to the agent id if unset; emoji is optional.
    AgentIdentity {
        agent_id: AgentId,
        name: Option<String>,
        emoji: Option<String>,
    },
    /// Real agent chat text — feed directly into a thought bubble.
    AgentMessage { agent_id: AgentId, text: String },
    /// Agent chose not to reply this turn (OpenClaw `NO_REPLY`
    /// sentinel). Nothing to render, but we use it to clear the
    /// chat-activity indicator right away instead of waiting for the
    /// idle timeout.
    AgentSilentTurn { agent_id: AgentId },
    /// Bootstrap chat history for one agent's main session, delivered
    /// per-agent as the operator switches into each one for the first
    /// time per connection. Replaces any existing in-memory history
    /// for that agent with the server's canonical transcript.
    ChatHistory {
        agent_id: AgentId,
        messages: Vec<crate::ui::chat_view::ChatMessage>,
    },
    /// Tool-invocation text (e.g. `⚙ exec`) — spawns a distinctly
    /// styled bubble so the operator can tell tool calls apart from
    /// conversational messages.
    AgentToolInvoked { agent_id: AgentId, text: String },
    /// Agent activity signal (tool call started, errored, etc.) without
    /// new text. Used to nudge the sprite's animation state.
    AgentActivity {
        agent_id: AgentId,
        kind: ActivityKind,
    },
    /// A session summary changed (scope-gated; only arrives with READ scope).
    SessionsChanged,
    /// Snapshot of a single session's metadata (token counts, model,
    /// last activity, etc.). Drives both the status bar's `ctx:`
    /// indicator and the full Sessions nav tab.
    SessionUsage(SessionInfo),
    /// Gateway rejected connect with `NOT_PAIRED / scope-upgrade` —
    /// the paired device record doesn't cover the scopes we asked
    /// for. Carries the pair-request id the operator approves via
    /// `openclaw devices approve <id>`. When `None`, the upgrade
    /// has been resolved (or wasn't required) and any pending
    /// indicator should be cleared.
    ScopeUpgradePending(Option<String>),
    /// Pending exec approval requires operator attention.
    ApprovalRequested(ApprovalEventPayload),
    /// Previously-pending approval has resolved (granted/denied).
    ApprovalResolved { id: Option<String> },
    /// Gateway-side update notification (or `None` to clear an
    /// earlier notification). Surfaces `current → latest` in the
    /// status bar so the operator sees they can upgrade.
    UpdateAvailable(Option<GatewayUpdate>),
    /// Incremental batch of new log lines since the last cursor.
    /// Feeds the Logs nav tab's ring buffer.
    LogTail(LogTailPayload),
    /// Connection to the gateway is healthy.
    Connected,
    /// Connection dropped (reason for display).
    Disconnected(String),
}

/// Gateway-side update notification, emitted via `update.available`.
#[derive(Debug, Clone)]
pub struct GatewayUpdate {
    pub current: String,
    pub latest: String,
    pub channel: String,
}

/// Coarse-grained agent activity signals derived from the `agent` event
/// stream (`"tool"`, `"item"`, `"error"`, `"lifecycle"`, ...). Intentionally
/// simple — the UI doesn't need full tool-call reconstruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    Thinking,
    ToolCalling,
    Errored,
}

/// Map a cron job's reported state to our domain status enum.
pub fn cron_status(cron: &CronJob) -> AgentStatus {
    if cron.state.running {
        return AgentStatus::Running;
    }
    match cron.state.last_status.as_deref() {
        Some("ok") => AgentStatus::Ok,
        Some("error") | Some("failed") | Some("timeout") => AgentStatus::Error,
        Some(_) => AgentStatus::Unknown,
        None => AgentStatus::Unknown,
    }
}

/// Map a channel's state to our domain status enum.
pub fn channel_status(ch: &Channel) -> AgentStatus {
    if !ch.enabled {
        return AgentStatus::Disabled;
    }
    if ch.last_error.is_some() {
        return AgentStatus::Error;
    }
    if ch.connected {
        AgentStatus::Ok
    } else {
        AgentStatus::Unknown
    }
}

/// Map main-agent state string to domain status.
pub fn main_agent_status(agent: &MainAgent) -> AgentStatus {
    match agent.state.as_deref() {
        Some("running") => AgentStatus::Running,
        Some("idle") | Some("ok") => AgentStatus::Ok,
        Some("error") => AgentStatus::Error,
        _ => AgentStatus::Unknown,
    }
}

/// Convenience: derive the AgentId for a cron's name.
pub fn cron_agent_id(cron: &CronJob) -> AgentId {
    AgentId::new(&cron.name)
}

/// Convenience: derive the AgentId for a channel's name.
pub fn channel_agent_id(ch: &Channel) -> AgentId {
    AgentId::new(&ch.name)
}

/// Reconstruct a synthetic `CronJob` from a push `cron` event so the
/// existing `apply_status_update` path can treat it like a snapshot
/// entry. Returns `None` for actions that don't imply a live status
/// change (`added`/`updated`/`removed`) — callers should issue an RPC
/// refresh or ignore.
pub fn cron_job_from_event(evt: &CronEventPayload) -> Option<CronJob> {
    // Prefer the human-readable name (filled in by the openclaw.rs
    // UUID→name cache) so the roster match succeeds. Falls back to
    // the UUID so the log/UI still shows *something* before the
    // initial snapshot lands.
    let name = evt.job_name.clone().unwrap_or_else(|| evt.job_id.clone());
    match evt.action.as_str() {
        "started" => Some(CronJob {
            name,
            id: Some(evt.job_id.clone()),
            state: CronState {
                running: true,
                ..Default::default()
            },
        }),
        "finished" => Some(CronJob {
            name,
            id: Some(evt.job_id.clone()),
            state: CronState {
                running: false,
                last_status: evt.status.clone(),
                last_run_at_ms: evt.run_at_ms,
                last_duration_ms: evt.duration_ms,
                last_error: evt.error.clone(),
                next_run_at_ms: evt.next_run_at_ms,
            },
        }),
        _ => None,
    }
}

/// Interpret an `agent` event `stream` string as coarse activity kind.
/// `lifecycle` / `assistant` aren't mapped here — those surface as
/// `AgentMessage` via the `chat` channel instead.
pub fn agent_stream_to_activity(stream: &str) -> Option<ActivityKind> {
    match stream {
        "tool" => Some(ActivityKind::ToolCalling),
        "item" => Some(ActivityKind::Thinking),
        "error" => Some(ActivityKind::Errored),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::rpc::{CronEventPayload, CronState};

    fn cron_with(running: bool, last: Option<&str>) -> CronJob {
        CronJob {
            name: "x".into(),
            id: None,
            state: CronState {
                running,
                last_status: last.map(|s| s.to_string()),
                ..Default::default()
            },
        }
    }

    fn evt(action: &str, status: Option<&str>) -> CronEventPayload {
        CronEventPayload {
            job_id: "zpool-health-check".into(),
            job_name: None,
            action: action.into(),
            run_at_ms: None,
            duration_ms: None,
            status: status.map(str::to_string),
            error: None,
            next_run_at_ms: None,
        }
    }

    #[test]
    fn started_event_yields_running_job() {
        let job = cron_job_from_event(&evt("started", None)).expect("started produces job");
        assert!(job.state.running);
        assert_eq!(job.name, "zpool-health-check");
        assert_eq!(cron_status(&job), AgentStatus::Running);
    }

    #[test]
    fn finished_ok_event_yields_ok_job() {
        let job = cron_job_from_event(&evt("finished", Some("ok"))).expect("finished produces job");
        assert!(!job.state.running);
        assert_eq!(job.state.last_status.as_deref(), Some("ok"));
        assert_eq!(cron_status(&job), AgentStatus::Ok);
    }

    #[test]
    fn finished_error_event_yields_error_job() {
        let job = cron_job_from_event(&evt("finished", Some("error"))).unwrap();
        assert_eq!(cron_status(&job), AgentStatus::Error);
    }

    #[test]
    fn non_run_actions_ignored() {
        for action in ["added", "updated", "removed"] {
            assert!(
                cron_job_from_event(&evt(action, None)).is_none(),
                "action {action} should not produce a CronJob — use RPC refresh"
            );
        }
    }

    #[test]
    fn agent_stream_mapping() {
        assert_eq!(
            agent_stream_to_activity("tool"),
            Some(ActivityKind::ToolCalling)
        );
        assert_eq!(
            agent_stream_to_activity("item"),
            Some(ActivityKind::Thinking)
        );
        assert_eq!(
            agent_stream_to_activity("error"),
            Some(ActivityKind::Errored)
        );
        assert_eq!(agent_stream_to_activity("assistant"), None);
        assert_eq!(agent_stream_to_activity("lifecycle"), None);
    }

    #[test]
    fn running_takes_precedence_over_last_status() {
        assert_eq!(
            cron_status(&cron_with(true, Some("error"))),
            AgentStatus::Running
        );
    }

    #[test]
    fn error_variants_mapped() {
        for s in ["error", "failed", "timeout"] {
            assert_eq!(cron_status(&cron_with(false, Some(s))), AgentStatus::Error);
        }
    }

    #[test]
    fn unknown_last_status_is_unknown() {
        assert_eq!(
            cron_status(&cron_with(false, Some("weird-future-status"))),
            AgentStatus::Unknown
        );
    }

    #[test]
    fn channel_disabled_short_circuits() {
        let ch = Channel {
            name: "whatsapp".into(),
            enabled: false,
            connected: false,
            last_error: None,
        };
        assert_eq!(channel_status(&ch), AgentStatus::Disabled);
    }
}
