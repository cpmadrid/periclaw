//! Events pushed from the WS stream (real or mock) into the Iced app.
//!
//! These are the domain-flavored events the UI actually cares about —
//! richer than raw RPC responses, simpler than the whole OpenClaw API.

use crate::domain::{AgentId, AgentStatus};
use crate::net::rpc::{Channel, CronJob, MainAgent};

#[derive(Debug, Clone)]
pub enum WsEvent {
    /// Initial or periodic snapshot of all cron jobs.
    CronSnapshot(Vec<CronJob>),
    /// Initial or periodic snapshot of all channel providers.
    ChannelSnapshot(Vec<Channel>),
    /// Main agent status update.
    MainAgent(MainAgent),
    /// Connection to the gateway is healthy.
    Connected,
    /// Connection dropped (reason for display).
    Disconnected(String),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::rpc::CronState;

    fn cron_with(running: bool, last: Option<&str>) -> CronJob {
        CronJob {
            name: "x".into(),
            state: CronState {
                running,
                last_status: last.map(|s| s.to_string()),
                ..Default::default()
            },
        }
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
