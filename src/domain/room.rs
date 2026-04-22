//! Room model — the named bounded areas of the Office scene sprites
//! inhabit. Stored as data (id + label) rather than a closed enum so
//! operators can add, rename, and reorder them via persisted state.

use serde::{Deserialize, Serialize};

/// A room in the office. `id` is the stable key used for persistence
/// and job↔room assignments; `label` is the text drawn at the top-left
/// of the room panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    pub id: String,
    pub label: String,
}

impl Room {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// The default room set seeded into `UiState` on first launch. Matches
/// the hand-tuned 6-room layout the app has shipped with — keeps the
/// visible scene identical until the operator customizes their rooms.
pub fn default_rooms() -> Vec<Room> {
    vec![
        Room::new("observatory", "Observatory"),
        Room::new("command-hq", "Command HQ"),
        Room::new("security", "Security"),
        Room::new("research-lab", "Research Lab"),
        Room::new("memory-vault", "Memory Vault"),
        Room::new("studio", "Studio"),
    ]
}

/// Default home-room assignment for a cron job by id. Returns the
/// legacy thematic mapping (zpool → Observatory, etc.) so operators
/// who don't configure anything keep the same scene they had before.
/// Unknown crons fall back to Research Lab.
pub fn default_cron_room(job_id: &str) -> &'static str {
    match job_id {
        "zpool-health-check" => "observatory",
        "openclaw-auto-update" => "research-lab",
        "example-data-sync-hourly" => "memory-vault",
        "example-weekly-digest" => "studio",
        _ => "research-lab",
    }
}

/// Room a job visits while running — all crons converge on Command HQ
/// so the operator can spot the "who's working right now?" band at a
/// glance.
pub const RUNNING_ROOM: &str = "command-hq";

/// Room errored entities get routed to. Red-alert visual.
pub const ERROR_ROOM: &str = "security";

/// Where Main agents live.
pub const MAIN_ROOM: &str = "command-hq";

/// Where a channel job lives when OK / Unknown.
pub const CHANNEL_OK_ROOM: &str = "studio";

/// Where a channel job lives when Disabled.
pub const CHANNEL_DISABLED_ROOM: &str = "memory-vault";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rooms_have_stable_ids() {
        let rooms = default_rooms();
        let ids: Vec<&str> = rooms.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "observatory",
                "command-hq",
                "security",
                "research-lab",
                "memory-vault",
                "studio",
            ]
        );
    }

    #[test]
    fn default_cron_rooms_cover_known_crons() {
        assert_eq!(default_cron_room("zpool-health-check"), "observatory");
        assert_eq!(default_cron_room("openclaw-auto-update"), "research-lab");
        assert_eq!(default_cron_room("some-unknown-cron"), "research-lab");
    }
}
