//! The roster of agents depicted in the Agent Office.
//!
//! Only the `main` agent is seeded at startup; cron and channel entries
//! are populated live from the gateway's `cron.list` / `channels.status`
//! snapshots so renames or additions on ubu-3xdv surface immediately.

use iced::Color;

use crate::ui::theme;

/// Stable identifier used to key state across updates.
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

/// What kind of entity this agent is — drives the sprite color / home room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    /// A named OpenClaw cron job.
    Cron,
    /// The `main` LLM agent / foreman.
    Main,
    /// A channel provider (Slack, Telegram, WhatsApp).
    Channel,
}

/// One sprite in the office.
#[derive(Debug, Clone)]
pub struct Agent {
    pub id: AgentId,
    /// Short label displayed under the sprite. Derived from the gateway's
    /// name for dynamic entries; hand-set for the `main` seed.
    pub display: String,
    pub kind: AgentKind,
}

impl Agent {
    pub fn cron(id: impl Into<String>) -> Self {
        let id: String = id.into();
        let display = shorten_for_display(&id);
        Self {
            id: AgentId::new(id),
            display,
            kind: AgentKind::Cron,
        }
    }

    pub fn channel(id: impl Into<String>) -> Self {
        let id: String = id.into();
        Self {
            id: AgentId::new(id.clone()),
            display: id,
            kind: AgentKind::Channel,
        }
    }

    /// Constructor for a non-default Main agent discovered via
    /// `agents.list`. The seed roster only contains `"main"`; any
    /// additional chat-capable agent comes in this way so it gets a
    /// sprite on the Overview scene and a row in the Chat picker.
    pub fn main_with_display(id: impl Into<String>, display: impl Into<String>) -> Self {
        Self {
            id: AgentId::new(id.into()),
            display: display.into(),
            kind: AgentKind::Main,
        }
    }

    pub fn color(&self) -> Color {
        // Main is the signature bright green; everything else gets a
        // deterministic ghost color derived from the id so renames stay
        // visually stable and new crons/channels don't all share one hue.
        if self.id.as_str() == "main" {
            return *theme::TERMINAL_GREEN;
        }
        match self.kind {
            AgentKind::Main => *theme::TERMINAL_GREEN,
            AgentKind::Cron => GHOST_PALETTE[stable_index(self.id.as_str(), GHOST_PALETTE.len())],
            AgentKind::Channel => *theme::MUTED,
        }
    }
}

/// Seed roster — just `main`. Cron and channel sprites are added
/// dynamically as their first snapshot event arrives.
pub fn seed_roster() -> Vec<Agent> {
    vec![Agent {
        id: AgentId::new("main"),
        display: "main".to_string(),
        kind: AgentKind::Main,
    }]
}

/// Collapse boilerplate prefixes/suffixes off long cron names so the
/// display label stays readable under the sprite. Examples:
///   `teamapp-velovate-sync-hourly` → `velovate-sync-hourly`
///   `openclaw-auto-update`         → `auto-update`
///   `zpool-health-check`           → `zpool-health-check`
fn shorten_for_display(id: &str) -> String {
    const PREFIXES: &[&str] = &["teamapp-", "openclaw-"];
    let mut s = id;
    for p in PREFIXES {
        if let Some(rest) = s.strip_prefix(p) {
            s = rest;
            break;
        }
    }
    s.to_string()
}

/// Pac-Man ghost palette — four classic ghost hues drawn from the
/// theme. `stable_index` hashes the id into this table so the same
/// cron always gets the same ghost.
fn ghost_palette() -> [Color; 4] {
    [
        *theme::STATUS_UP,
        *theme::STATUS_DEGRADED,
        *theme::TERMINAL_GREEN,
        *theme::STATUS_UNKNOWN,
    ]
}

// Lazy: recompute on every color() call. Cheap (4 Color clones, no lock).
#[allow(non_upper_case_globals)]
static GHOST_PALETTE: std::sync::LazyLock<[Color; 4]> = std::sync::LazyLock::new(ghost_palette);

fn stable_index(id: &str, modulus: usize) -> usize {
    // djb2 — tiny hash, no dep, deterministic across runs.
    let mut h: u64 = 5381;
    for b in id.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    (h as usize) % modulus.max(1)
}
