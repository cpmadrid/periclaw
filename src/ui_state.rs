//! Persistent UI state — selected tab, selected chat agent, window
//! size and position.
//!
//! Loaded once at startup (before the Iced application runs, so the
//! initial window can be sized to the saved dimensions) and re-saved
//! whenever the operator changes one of the tracked values. Lives at
//! `$XDG_CONFIG_HOME/periclaw/desktop-state.json` (macOS: under
//! `~/Library/Application Support/periclaw/`) alongside the gateway
//! token file.
//!
//! Corrupt or unreadable → log at `warn` and fall back to `Default`.
//! The state file is entirely recoverable — deleting it just resets
//! the UI to first-launch state — so panicking here would cost more
//! than it protects.

use std::fs;
use std::path::PathBuf;

use directories::BaseDirs;
use serde::{Deserialize, Serialize};

use crate::app::NavItem;

const STATE_FILE: &str = "desktop-state.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiState {
    /// Last-selected nav tab. Stored as a string rather than the enum
    /// so we can add / rename tabs without invalidating persisted
    /// state — unknown values silently fall back to `Overview`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<String>,
    /// Last-selected agent in the Chat picker. Stored as the raw id
    /// string so a rename on the gateway side doesn't brick the
    /// restore — if the id no longer exists, `App::new` falls back to
    /// the seed default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_agent: Option<String>,
    /// Last-selected session in the Sessions tab drill-in. Stored as
    /// the fully-qualified `agent:<id>:<sessionId>` key so a rename
    /// (rare) simply no-op's on restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_session_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowState>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
    /// Top-left position in logical pixels. Absent when the platform
    /// didn't give us a position event (some compositors don't).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<(f32, f32)>,
}

/// Read the state file from disk. Missing / unreadable / corrupt all
/// reduce to `Default` — the caller never has to pattern-match.
pub fn load() -> UiState {
    let Some(path) = state_path() else {
        return UiState::default();
    };
    if !path.exists() {
        return UiState::default();
    }
    match fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<UiState>(&raw) {
            Ok(state) => {
                tracing::debug!(path = %path.display(), "ui state loaded");
                state
            }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "ui state file corrupt, falling back to defaults",
                );
                UiState::default()
            }
        },
        Err(e) => {
            tracing::debug!(
                path = %path.display(),
                error = %e,
                "ui state file unreadable",
            );
            UiState::default()
        }
    }
}

/// Persist the current state. Best-effort — failures are logged at
/// `debug` since the consequence (next launch starts at the seed
/// defaults) is not worth surfacing to the operator.
pub fn save(state: &UiState) {
    let Some(path) = state_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        tracing::debug!(
            error = %e,
            path = %parent.display(),
            "could not create ui state dir",
        );
        return;
    }
    let raw = match serde_json::to_string_pretty(state) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "ui state serialize failed");
            return;
        }
    };
    if let Err(e) = fs::write(&path, raw) {
        tracing::debug!(
            error = %e,
            path = %path.display(),
            "ui state write failed",
        );
    }
}

fn state_path() -> Option<PathBuf> {
    let base = BaseDirs::new()?;
    Some(base.config_dir().join("periclaw").join(STATE_FILE))
}

pub fn nav_from_str(s: &str) -> Option<NavItem> {
    match s {
        "overview" => Some(NavItem::Overview),
        "chat" => Some(NavItem::Chat),
        "agents" => Some(NavItem::Agents),
        "sessions" => Some(NavItem::Sessions),
        "logs" => Some(NavItem::Logs),
        "settings" => Some(NavItem::Settings),
        _ => None,
    }
}

pub fn nav_to_str(item: NavItem) -> &'static str {
    match item {
        NavItem::Overview => "overview",
        NavItem::Chat => "chat",
        NavItem::Agents => "agents",
        NavItem::Sessions => "sessions",
        NavItem::Logs => "logs",
        NavItem::Settings => "settings",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_round_trip_covers_all_variants() {
        for item in [
            NavItem::Overview,
            NavItem::Chat,
            NavItem::Agents,
            NavItem::Sessions,
            NavItem::Logs,
            NavItem::Settings,
        ] {
            let s = nav_to_str(item);
            assert_eq!(nav_from_str(s), Some(item), "round-trip failed for {s}");
        }
    }

    #[test]
    fn nav_from_str_rejects_unknown() {
        assert_eq!(nav_from_str(""), None);
        assert_eq!(nav_from_str("metrics"), None);
        // Case-sensitive — we write lowercase, so anything else is an
        // unknown value and must drop rather than be silently coerced.
        assert_eq!(nav_from_str("Overview"), None);
    }

    #[test]
    fn empty_state_serializes_to_empty_object() {
        let raw = serde_json::to_string(&UiState::default()).unwrap();
        assert_eq!(raw, "{}");
    }

    #[test]
    fn round_trip_preserves_fields() {
        let original = UiState {
            tab: Some("chat".to_string()),
            selected_agent: Some("sebastian".to_string()),
            active_session_key: Some("agent:sebastian:abc123".to_string()),
            window: Some(WindowState {
                width: 1440.0,
                height: 900.0,
                position: Some((120.0, 80.0)),
            }),
        };
        let raw = serde_json::to_string(&original).unwrap();
        let parsed: UiState = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.tab.as_deref(), Some("chat"));
        assert_eq!(parsed.selected_agent.as_deref(), Some("sebastian"));
        assert_eq!(
            parsed.active_session_key.as_deref(),
            Some("agent:sebastian:abc123"),
        );
        let w = parsed.window.unwrap();
        assert_eq!(w.width, 1440.0);
        assert_eq!(w.height, 900.0);
        assert_eq!(w.position, Some((120.0, 80.0)));
    }

    #[test]
    fn missing_fields_fall_back_to_default() {
        // Earlier persisted state with only the tab field — forward
        // compat should accept it and synthesize empty for the rest.
        let raw = r#"{"tab":"logs"}"#;
        let parsed: UiState = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.tab.as_deref(), Some("logs"));
        assert!(parsed.selected_agent.is_none());
        assert!(parsed.window.is_none());
    }
}
