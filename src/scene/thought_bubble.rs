//! Thought-bubble overlay for sprite state transitions.
//!
//! When an agent's status changes, we spawn a bubble above its sprite
//! with short transition-appropriate text ("eureka!", "anomaly!",
//! "zzz", "...polling..."). Bubbles live for [`TTL`] and fade out in
//! their last half-second.

use std::time::{Duration, Instant};

use crate::domain::{AgentId, AgentStatus};

pub const TTL: Duration = Duration::from_millis(2000);
pub const FADE_START: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone)]
pub struct ThoughtBubble {
    pub agent: AgentId,
    pub text: String,
    pub born: Instant,
}

impl ThoughtBubble {
    pub fn new(agent: AgentId, text: impl Into<String>) -> Self {
        Self {
            agent,
            text: text.into(),
            born: Instant::now(),
        }
    }

    /// Returns `None` if the bubble has expired; otherwise the alpha in `[0,1]`.
    pub fn alpha(&self, now: Instant) -> Option<f32> {
        let age = now.saturating_duration_since(self.born);
        if age >= TTL {
            return None;
        }
        if age < FADE_START {
            return Some(1.0);
        }
        let fade_span = TTL - FADE_START;
        let into_fade = age - FADE_START;
        Some(1.0 - (into_fade.as_secs_f32() / fade_span.as_secs_f32()))
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.alpha(now).is_none()
    }
}

/// Pick a short transition message based on where we came from and where
/// we're going.
pub fn transition_text(prev: Option<AgentStatus>, next: AgentStatus) -> Option<&'static str> {
    let prev = prev.unwrap_or(AgentStatus::Unknown);
    if prev == next {
        return None;
    }
    Some(match (prev, next) {
        (_, AgentStatus::Error) => "anomaly!",
        (AgentStatus::Error, AgentStatus::Ok) => "recovered",
        (_, AgentStatus::Running) => "...working...",
        (AgentStatus::Running, AgentStatus::Ok) => "eureka!",
        (AgentStatus::Unknown, AgentStatus::Ok) => "online",
        (_, AgentStatus::Disabled) => "zzz",
        (AgentStatus::Disabled, _) => "awake",
        (_, AgentStatus::Unknown) => "?",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_error_always_anomaly() {
        assert_eq!(
            transition_text(Some(AgentStatus::Ok), AgentStatus::Error),
            Some("anomaly!")
        );
        assert_eq!(
            transition_text(Some(AgentStatus::Running), AgentStatus::Error),
            Some("anomaly!")
        );
    }

    #[test]
    fn same_status_produces_nothing() {
        assert_eq!(
            transition_text(Some(AgentStatus::Ok), AgentStatus::Ok),
            None
        );
    }

    #[test]
    fn running_to_ok_is_eureka() {
        assert_eq!(
            transition_text(Some(AgentStatus::Running), AgentStatus::Ok),
            Some("eureka!")
        );
    }

    #[test]
    fn alpha_full_then_fade_then_gone() {
        let bubble = ThoughtBubble::new(AgentId::new("x"), "hi");
        let born = bubble.born;
        assert_eq!(bubble.alpha(born), Some(1.0));
        assert_eq!(bubble.alpha(born + Duration::from_millis(1499)), Some(1.0));
        let mid = bubble.alpha(born + Duration::from_millis(1750)).unwrap();
        assert!(mid > 0.0 && mid < 1.0, "mid-fade alpha was {mid}");
        assert_eq!(bubble.alpha(born + TTL), None);
    }
}
