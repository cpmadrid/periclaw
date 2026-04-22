//! Room model — the named bounded areas of the Office scene sprites
//! inhabit. Stored as data (id + label) rather than a closed enum so
//! operators can add, rename, and reorder them via persisted state.

use serde::{Deserialize, Serialize};

/// A room in the office. `id` is the stable key used for persistence
/// and home-room assignments; `label` is the text drawn at the top-left
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

/// The default room set seeded into `UiState` on first launch.
/// Three rooms: Sebastian sits in the Command Deck; the other two are
/// thematic placeholders the operator can rename or repurpose.
pub fn default_rooms() -> Vec<Room> {
    vec![
        Room::new("command-deck", "Command Deck"),
        Room::new("galley", "Galley"),
        Room::new("engine-room", "Engine Room"),
    ]
}

/// Fallback home room used when no agent override is configured. Agents
/// without an entry in `UiState.agent_rooms` land here.
pub const MAIN_ROOM: &str = "command-deck";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rooms_have_stable_ids() {
        let rooms = default_rooms();
        let ids: Vec<&str> = rooms.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["command-deck", "galley", "engine-room"]);
    }

    #[test]
    fn main_room_is_in_defaults() {
        let rooms = default_rooms();
        assert!(rooms.iter().any(|r| r.id == MAIN_ROOM));
    }
}
