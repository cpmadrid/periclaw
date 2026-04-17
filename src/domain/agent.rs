//! The static roster of agents depicted in the Agent Office.
//!
//! Names come directly from the live OpenClaw setup on ubu-3xdv:
//! - Cron jobs from `~/.openclaw/cron/jobs.json`
//! - The single `main` agent from `agents.list[]` in openclaw.json
//! - Channels from `channels.{slack,telegram,whatsapp}` in openclaw.json

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
    /// Short label displayed under the sprite (e.g. "zpool", "velovate-sync").
    pub display: &'static str,
    pub kind: AgentKind,
}

impl Agent {
    pub fn color(&self) -> Color {
        // Sprite colors per the Pac-Man homage — each cron gets its own
        // ghost color, main is the signature green, channels are dimmer.
        match self.id.as_str() {
            // Inky-blue observatory dweller
            "zpool-health-check" => *theme::STATUS_UP,
            // Pinky, a little pink/magenta
            "openclaw-auto-update" => *theme::STATUS_DEGRADED,
            // Winky (our version) — the sync worker
            "teamapp-velovate-sync-hourly" => *theme::TERMINAL_GREEN,
            // Clyde the digester
            "velovate-weekly-digest" => *theme::STATUS_UNKNOWN,
            // Main is the signature green, brightest
            "main" => *theme::TERMINAL_GREEN,
            // Channels dim by default
            _ => *theme::MUTED,
        }
    }
}

/// The canonical roster — the list of sprites that will be rendered.
///
/// Keeping this static (not derived from a gateway call) for v1 because
/// the set of agents is small and slow-changing. If you add a cron on
/// ubu-3xdv, update this list and recompile.
pub fn roster() -> Vec<Agent> {
    vec![
        // Cron jobs
        Agent {
            id: AgentId::new("zpool-health-check"),
            display: "zpool",
            kind: AgentKind::Cron,
        },
        Agent {
            id: AgentId::new("openclaw-auto-update"),
            display: "auto-update",
            kind: AgentKind::Cron,
        },
        Agent {
            id: AgentId::new("teamapp-velovate-sync-hourly"),
            display: "velovate-sync",
            kind: AgentKind::Cron,
        },
        Agent {
            id: AgentId::new("velovate-weekly-digest"),
            display: "weekly-digest",
            kind: AgentKind::Cron,
        },
        // Main agent
        Agent {
            id: AgentId::new("main"),
            display: "main",
            kind: AgentKind::Main,
        },
        // Channels
        Agent {
            id: AgentId::new("slack"),
            display: "slack",
            kind: AgentKind::Channel,
        },
        Agent {
            id: AgentId::new("telegram"),
            display: "telegram",
            kind: AgentKind::Channel,
        },
        Agent {
            id: AgentId::new("whatsapp"),
            display: "whatsapp",
            kind: AgentKind::Channel,
        },
    ]
}
