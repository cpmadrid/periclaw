//! Pure mapping from (agent, status) → which room the sprite occupies.
//!
//! Each agent has a "home room" where it lives when idle, and a
//! "work room" it moves to while running. Error states get their
//! own room (Security) so they're visually salient.

use super::{AgentId, AgentKind, AgentStatus};

/// Six rooms arranged in a 3×2 grid (top row first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoomId {
    Observatory, // top-left
    CommandHq,   // top-center
    Security,    // top-right
    ResearchLab, // bottom-left
    MemoryVault, // bottom-center
    Studio,      // bottom-right
}

impl RoomId {
    pub fn label(self) -> &'static str {
        match self {
            RoomId::Observatory => "Observatory",
            RoomId::CommandHq => "Command HQ",
            RoomId::Security => "Security",
            RoomId::ResearchLab => "Research Lab",
            RoomId::MemoryVault => "Memory Vault",
            RoomId::Studio => "Studio",
        }
    }

    /// Grid column (0..=2), from the top-left.
    pub fn col(self) -> u8 {
        match self {
            RoomId::Observatory | RoomId::ResearchLab => 0,
            RoomId::CommandHq | RoomId::MemoryVault => 1,
            RoomId::Security | RoomId::Studio => 2,
        }
    }

    /// Grid row (0..=1).
    pub fn row(self) -> u8 {
        match self {
            RoomId::Observatory | RoomId::CommandHq | RoomId::Security => 0,
            RoomId::ResearchLab | RoomId::MemoryVault | RoomId::Studio => 1,
        }
    }
}

/// Compute the room assignment for an agent in a given status.
///
/// Rules:
/// - Any agent in `Error` heads to Security (red-alert room).
/// - Running cron jobs go to Command HQ (they're actively orchestrating).
/// - Idle crons return to their themed home room.
/// - Main agent lives in Command HQ always; when running, stays in HQ.
/// - Channels live in Studio when OK; go to Memory Vault if disabled.
pub fn room_for(id: &AgentId, kind: AgentKind, status: AgentStatus) -> RoomId {
    if status == AgentStatus::Error {
        return RoomId::Security;
    }

    match kind {
        AgentKind::Main => RoomId::CommandHq,
        AgentKind::Cron => match (id.as_str(), status) {
            (_, AgentStatus::Running) => RoomId::CommandHq,
            // Each cron's idle "home" — thematic association
            ("zpool-health-check", _) => RoomId::Observatory,
            ("openclaw-auto-update", _) => RoomId::ResearchLab,
            ("example-data-sync-hourly", _) => RoomId::MemoryVault,
            ("example-weekly-digest", _) => RoomId::Studio,
            // Unknown cron: fall back to Research Lab
            _ => RoomId::ResearchLab,
        },
        AgentKind::Channel => match status {
            AgentStatus::Disabled => RoomId::MemoryVault,
            _ => RoomId::Studio,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> AgentId {
        AgentId::new(s)
    }

    #[test]
    fn error_always_routes_to_security() {
        for kind in [AgentKind::Cron, AgentKind::Main, AgentKind::Channel] {
            assert_eq!(
                room_for(&id("anything"), kind, AgentStatus::Error),
                RoomId::Security
            );
        }
    }

    #[test]
    fn main_lives_in_hq() {
        for status in [
            AgentStatus::Ok,
            AgentStatus::Running,
            AgentStatus::Unknown,
            AgentStatus::Disabled,
        ] {
            assert_eq!(
                room_for(&id("main"), AgentKind::Main, status),
                RoomId::CommandHq
            );
        }
    }

    #[test]
    fn running_crons_visit_command_hq() {
        let crons = [
            "zpool-health-check",
            "openclaw-auto-update",
            "example-data-sync-hourly",
            "example-weekly-digest",
        ];
        for name in crons {
            assert_eq!(
                room_for(&id(name), AgentKind::Cron, AgentStatus::Running),
                RoomId::CommandHq,
            );
        }
    }

    #[test]
    fn idle_crons_go_home() {
        let cases = [
            ("zpool-health-check", RoomId::Observatory),
            ("openclaw-auto-update", RoomId::ResearchLab),
            ("example-data-sync-hourly", RoomId::MemoryVault),
            ("example-weekly-digest", RoomId::Studio),
        ];
        for (name, expected) in cases {
            assert_eq!(
                room_for(&id(name), AgentKind::Cron, AgentStatus::Ok),
                expected,
            );
        }
    }

    #[test]
    fn channels_live_in_studio_when_ok() {
        for name in ["slack", "telegram", "whatsapp"] {
            assert_eq!(
                room_for(&id(name), AgentKind::Channel, AgentStatus::Ok),
                RoomId::Studio,
            );
        }
    }

    #[test]
    fn disabled_channels_go_to_memory_vault() {
        assert_eq!(
            room_for(&id("whatsapp"), AgentKind::Channel, AgentStatus::Disabled),
            RoomId::MemoryVault,
        );
    }
}
