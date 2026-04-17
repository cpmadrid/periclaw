/// Operational state of a single agent (cron job, main agent, or channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentStatus {
    /// Doing work right now.
    Running,
    /// Idle, healthy, last run was OK.
    Ok,
    /// Last run ended in error; not currently running.
    Error,
    /// Not yet observed / unknown state.
    Unknown,
    /// Deliberately disabled (e.g. WhatsApp channel turned off).
    Disabled,
}
