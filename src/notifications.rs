//! Native OS notifications — approval requests, cron failures,
//! gateway update availability.
//!
//! Every trigger routes through a `Notifier` that tracks what's
//! already been surfaced so the 30-second `channels.status` /
//! `cron.list` heartbeats don't re-fire notifications for the same
//! underlying event every cycle. The actual OS call is offloaded
//! to a spawned thread — `notify-rust::Notification::show()` can
//! block briefly on macOS while the notification center accepts
//! the request, and the UI thread must never stall.
//!
//! A missing notifier (e.g. no dbus daemon on a headless Linux box
//! in CI) is logged at `debug` and then skipped; the app still
//! functions, just without the OS-level prompts.

use std::collections::{HashMap, HashSet};

use crate::domain::AgentId;
use crate::net::events::GatewayUpdate;
use crate::net::rpc::{ApprovalEventPayload, CronState};

const APP_NAME: &str = "Periclaw";

/// Per-app dedup state. Lives in `App` so it survives across
/// `WsEvent` arrivals but is cleared on disconnect (the heartbeats
/// bootstrap fresh on reconnect, so re-firing for unresolved state
/// after a blip is desired — the operator may have dismissed the
/// previous OS notification).
#[derive(Debug, Default)]
pub struct Notifier {
    /// Approval ids we've already surfaced. `ApprovalEventPayload.id`
    /// is optional; when absent we fall back to `tool:summary` to
    /// keep dedup working for gateways that don't assign ids.
    approvals_seen: HashSet<String>,
    /// Last error string we notified about per cron, keyed by
    /// `AgentId`. Re-fires when the error text changes but not
    /// when the same error keeps getting re-reported.
    cron_errors_notified: HashMap<AgentId, String>,
    /// "<current>→<latest>" string we've already surfaced. Changes
    /// when the gateway publishes a new latest, so a second
    /// upgrade-available notification lands if the user ignores the
    /// first one long enough for another release to ship.
    update_notified: Option<String>,
}

impl Notifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop dedup state on disconnect so the heartbeats on the
    /// next reconnect can re-surface anything still unresolved.
    pub fn reset_on_disconnect(&mut self) {
        self.approvals_seen.clear();
        self.cron_errors_notified.clear();
        // `update_notified` is intentionally retained — an update
        // being available is a server-facts thing, not a
        // connection-state thing. The gateway will re-broadcast
        // `update.available` on reconnect, and we'll dedupe against
        // the retained string.
    }

    /// Handle a new exec-approval request. Fires a notification on
    /// first sight of a given id (or synthetic id, when the server
    /// doesn't assign one).
    pub fn approval_requested(&mut self, payload: &ApprovalEventPayload) {
        let key = approval_dedup_key(payload);
        if !self.approvals_seen.insert(key) {
            return;
        }
        let tool = payload.tool.as_deref().unwrap_or("tool");
        let summary = payload
            .summary
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(shorten)
            .unwrap_or_else(|| "awaiting operator decision".to_string());
        fire("Approval requested", &format!("{tool}: {summary}"));
    }

    /// Handle a newly-resolved approval: clear it from dedup so a
    /// later request with the same id (shouldn't happen in practice,
    /// but gateways have been known to recycle) re-fires cleanly.
    pub fn approval_resolved(&mut self, id: Option<&str>) {
        if let Some(id) = id {
            self.approvals_seen.remove(id);
        } else {
            self.approvals_seen.clear();
        }
    }

    /// Check whether a cron just transitioned into a failed state
    /// and fire the notification if so. Takes the **new** state
    /// and compares its `last_error` against our dedup map.
    pub fn cron_state_changed(&mut self, agent_id: &AgentId, state: &CronState) {
        let Some(error) = state.last_error.as_deref() else {
            // No error this cycle — clear any dedup we held so a
            // future error re-fires.
            self.cron_errors_notified.remove(agent_id);
            return;
        };
        let already = self.cron_errors_notified.get(agent_id).map(String::as_str) == Some(error);
        if already {
            return;
        }
        self.cron_errors_notified
            .insert(agent_id.clone(), error.to_string());
        fire(
            &format!("Cron failed: {}", agent_id.as_str()),
            &shorten(error),
        );
    }

    /// Gateway announced a newer release.
    pub fn update_available(&mut self, update: &GatewayUpdate) {
        let key = format!("{}->{}", update.current, update.latest);
        if self.update_notified.as_deref() == Some(key.as_str()) {
            return;
        }
        self.update_notified = Some(key);
        fire(
            "OpenClaw update available",
            &format!("{} → {}", update.current, update.latest),
        );
    }
}

fn approval_dedup_key(payload: &ApprovalEventPayload) -> String {
    if let Some(id) = payload.id.as_deref() {
        return id.to_string();
    }
    // Fall back to tool+summary so a same-shape request doesn't
    // re-notify. Matches the same fallback used in
    // `App::apply_ws`'s ApprovalRequested arm.
    format!(
        "{}:{}",
        payload.tool.as_deref().unwrap_or("?"),
        payload.summary.as_deref().unwrap_or(""),
    )
}

/// Clip to a sane length for an OS notification body — macOS
/// truncates around 200 chars in the notification center banner,
/// Linux's zbus body wraps but looks awful long.
fn shorten(text: &str) -> String {
    const MAX: usize = 140;
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(MAX - 1).collect();
    out.push('…');
    out
}

fn fire(title: &str, body: &str) {
    let title = title.to_string();
    let body = body.to_string();
    // Offload the OS call — notify-rust can block briefly on
    // macOS while UserNotifications accepts the request, and we
    // must not stall the Iced event loop.
    std::thread::spawn(move || {
        if let Err(e) = notify_rust::Notification::new()
            .appname(APP_NAME)
            .summary(&title)
            .body(&body)
            .show()
        {
            // No available notifier (headless Linux in CI, Windows
            // with toasts disabled, etc.) — debug-log and move on.
            tracing::debug!(error = %e, title, "notification not delivered");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_approval(id: Option<&str>, tool: &str, summary: &str) -> ApprovalEventPayload {
        ApprovalEventPayload {
            id: id.map(str::to_string),
            session_key: None,
            tool: Some(tool.to_string()),
            summary: Some(summary.to_string()),
        }
    }

    #[test]
    fn approval_dedup_by_id() {
        let mut n = Notifier::new();
        let p = make_approval(Some("abc"), "bash", "run ls");
        // First call should be surfaced (can't assert the fire
        // directly — it's a side effect — but we can assert the
        // dedup set grew).
        n.approval_requested(&p);
        assert!(n.approvals_seen.contains("abc"));
        n.approval_requested(&p);
        // Set size still one; no extra bookkeeping.
        assert_eq!(n.approvals_seen.len(), 1);
    }

    #[test]
    fn approval_dedup_falls_back_when_id_missing() {
        let mut n = Notifier::new();
        let p = make_approval(None, "bash", "run ls");
        n.approval_requested(&p);
        n.approval_requested(&p);
        assert_eq!(n.approvals_seen.len(), 1);
        assert!(n.approvals_seen.contains("bash:run ls"));
    }

    #[test]
    fn approval_resolved_clears_dedup() {
        let mut n = Notifier::new();
        n.approval_requested(&make_approval(Some("abc"), "bash", "run"));
        n.approval_resolved(Some("abc"));
        assert!(!n.approvals_seen.contains("abc"));
    }

    #[test]
    fn cron_error_refires_only_when_text_changes() {
        let mut n = Notifier::new();
        let id = AgentId::new("sync");
        let mut state = CronState {
            last_error: Some("timeout".to_string()),
            ..Default::default()
        };
        n.cron_state_changed(&id, &state);
        assert_eq!(
            n.cron_errors_notified.get(&id).map(String::as_str),
            Some("timeout")
        );
        // Same error next tick — dedup'd.
        n.cron_state_changed(&id, &state);
        assert_eq!(n.cron_errors_notified.len(), 1);
        // Different error — should refire (we can't observe the
        // fire() call but the dedup map should update).
        state.last_error = Some("connection refused".to_string());
        n.cron_state_changed(&id, &state);
        assert_eq!(
            n.cron_errors_notified.get(&id).map(String::as_str),
            Some("connection refused"),
        );
    }

    #[test]
    fn cron_error_cleared_when_last_error_goes_none() {
        let mut n = Notifier::new();
        let id = AgentId::new("sync");
        let state = CronState {
            last_error: Some("timeout".to_string()),
            ..Default::default()
        };
        n.cron_state_changed(&id, &state);
        let healthy = CronState::default();
        n.cron_state_changed(&id, &healthy);
        // Dedup cleared so a future regression would refire.
        assert!(!n.cron_errors_notified.contains_key(&id));
    }

    #[test]
    fn update_dedup_on_version_pair() {
        let mut n = Notifier::new();
        let u1 = GatewayUpdate {
            current: "1.0.0".into(),
            latest: "1.1.0".into(),
            channel: "stable".into(),
        };
        n.update_available(&u1);
        n.update_available(&u1);
        assert_eq!(n.update_notified.as_deref(), Some("1.0.0->1.1.0"));
        let u2 = GatewayUpdate {
            current: "1.0.0".into(),
            latest: "1.2.0".into(),
            channel: "stable".into(),
        };
        n.update_available(&u2);
        assert_eq!(n.update_notified.as_deref(), Some("1.0.0->1.2.0"));
    }

    #[test]
    fn shorten_trims_long_bodies() {
        let long = "x".repeat(200);
        let out = shorten(&long);
        assert_eq!(out.chars().count(), 140);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn shorten_leaves_short_bodies_alone() {
        assert_eq!(shorten("  hello  "), "hello");
    }

    #[test]
    fn reset_on_disconnect_keeps_update_state() {
        let mut n = Notifier::new();
        n.approvals_seen.insert("abc".into());
        n.cron_errors_notified
            .insert(AgentId::new("sync"), "err".into());
        n.update_notified = Some("1.0->1.1".into());
        n.reset_on_disconnect();
        assert!(n.approvals_seen.is_empty());
        assert!(n.cron_errors_notified.is_empty());
        assert_eq!(n.update_notified.as_deref(), Some("1.0->1.1"));
    }
}
