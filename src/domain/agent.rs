//! Chat-capable agents — the roster of sprites that can hold a
//! conversation. Cron jobs and channel providers live in [`crate::domain::job`]
//! instead; the split keeps the roster tight and removes the
//! "what kind of thing is this?" branching that used to be everywhere.
//!
//! Only the `main` agent is seeded at startup; additional agents are
//! populated from the gateway's `agents.list` RPC on connect.

use iced::Color;

use crate::ui::theme;

/// Stable identifier used to key per-agent state across updates.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentId(pub String);

impl AgentId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One chat-capable agent.
#[derive(Debug, Clone)]
pub struct Agent {
    pub id: AgentId,
    /// Short label displayed under the sprite. Derived from the gateway's
    /// `agents.list` persona for dynamic entries; hand-set for the
    /// `main` seed.
    pub display: String,
}

impl Agent {
    /// Constructor for any agent discovered via `agents.list`. The seed
    /// roster only contains `"main"`; additional chat-capable agents
    /// come in this way so they get a sprite and a Chat picker row.
    pub fn new(id: impl Into<String>, display: impl Into<String>) -> Self {
        Self {
            id: AgentId::new(id.into()),
            display: display.into(),
        }
    }

    pub fn color(&self) -> Color {
        // Main gets the signature bright green; other chat agents take
        // a deterministic ghost color keyed by id so personas stay
        // visually stable across runs.
        if self.id.as_str() == "main" {
            return *theme::TERMINAL_GREEN;
        }
        GHOST_PALETTE[stable_index(self.id.as_str(), GHOST_PALETTE.len())]
    }
}

/// Seed roster — just `main`. Additional agents arrive via the
/// `agents.list` RPC once the WS connects.
pub fn seed_roster() -> Vec<Agent> {
    vec![Agent {
        id: AgentId::new("main"),
        display: "main".to_string(),
    }]
}

/// Pac-Man ghost palette — four classic ghost hues drawn from the
/// theme, used by non-main chat agents so siblings get distinct
/// colors without manual assignment.
fn ghost_palette() -> [Color; 4] {
    [
        *theme::STATUS_UP,
        *theme::STATUS_DEGRADED,
        *theme::TERMINAL_GREEN,
        *theme::STATUS_UNKNOWN,
    ]
}

static GHOST_PALETTE: std::sync::LazyLock<[Color; 4]> = std::sync::LazyLock::new(ghost_palette);

fn stable_index(id: &str, modulus: usize) -> usize {
    let mut h: u64 = 5381;
    for b in id.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    (h as usize) % modulus.max(1)
}
