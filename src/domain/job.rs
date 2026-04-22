//! Non-chat runtime entities surfaced in the Office scene.
//!
//! A `Job` is everything the operator watches that isn't a chat-capable
//! agent — cron schedules and channel providers. Split out from `Agent`
//! so the roster stays Main-only and consumers stop pattern-matching on
//! an `AgentKind` just to decide whether a thing can be chatted with.

use iced::Color;

use crate::domain::AgentStatus;
use crate::ui::theme;

/// Stable identifier for a job. Mirrors the shape of [`crate::domain::AgentId`]
/// — distinct type so a job id and a chat-agent id can't be silently swapped.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JobId(pub String);

impl JobId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What kind of job this is — drives sprite choice (lobster vs monitor)
/// and which detail renderer the Agents tab picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    /// Scheduled OpenClaw cron (`cron.list`, `cron.run`).
    Cron,
    /// Messaging channel provider (Slack / Telegram / WhatsApp).
    Channel,
}

/// One job in the office.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: JobId,
    pub display: String,
    pub kind: JobKind,
    pub status: AgentStatus,
}

impl Job {
    pub fn cron(id: impl Into<String>) -> Self {
        let id: String = id.into();
        let display = shorten_for_display(&id);
        Self {
            id: JobId::new(id),
            display,
            kind: JobKind::Cron,
            status: AgentStatus::Unknown,
        }
    }

    pub fn channel(id: impl Into<String>) -> Self {
        let id: String = id.into();
        Self {
            id: JobId::new(id.clone()),
            display: id,
            kind: JobKind::Channel,
            status: AgentStatus::Unknown,
        }
    }

    pub fn color(&self) -> Color {
        match self.kind {
            JobKind::Cron => GHOST_PALETTE[stable_index(self.id.as_str(), GHOST_PALETTE.len())],
            JobKind::Channel => *theme::MUTED,
        }
    }
}

/// Collapse boilerplate prefixes off long cron names so the display label
/// stays readable under the sprite. Keeps the same list as the legacy
/// `Agent::cron()` constructor for continuity.
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
