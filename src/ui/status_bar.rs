//! Bottom status bar showing connection, agent count, active model,
//! last-poll age, context usage, pending approvals, and update
//! availability.
//!
//! Each indicator that has a useful destination is clickable:
//! - connection dot / label → Logs tab (see why disconnected, or
//!   what's happening on the wire)
//! - agent count → Agents tab
//! - context usage → Sessions tab (drill into main session)
//! - pending-approvals chip → Overview (approvals panel lives there)
//! - update chip → `RequestReconnect` (handy after a gateway upgrade
//!   when the operator wants to reconnect without waiting on backoff)
//!
//! Non-actionable readouts (active model, last-poll age) stay as
//! plain text so the row doesn't feel like every pixel demands
//! attention.

use std::time::Instant;

use iced::widget::{button, container, row, text};
use iced::{Border, Color, Element, Length, Padding, Shadow};

use crate::Message;
use crate::app::NavItem;
use crate::ui::theme;

pub struct Snapshot<'a> {
    pub connected: bool,
    pub agents_tracked: usize,
    pub last_poll: Option<Instant>,
    pub active_model: Option<&'a str>,
    pub last_disconnect: Option<&'a str>,
    /// Token usage of the main session (`totalTokens`, `contextTokens`).
    /// `None` until the first sessions.list / session.message snapshot
    /// lands.
    pub main_usage: Option<(i64, i64)>,
    /// Count of exec approvals awaiting operator decision.
    pub pending_approvals: usize,
    /// Gateway-side update notification (`current`, `latest`).
    pub update: Option<(&'a str, &'a str)>,
}

pub fn view(snap: Snapshot<'_>) -> Element<'_, Message> {
    let (dot, label) = connection_line(&snap);
    let dot_color = if snap.connected {
        *theme::TERMINAL_GREEN
    } else {
        *theme::MUTED
    };

    let mut items: Vec<Element<'_, Message>> = Vec::new();

    // Connection dot + label — one clickable chunk so the whole
    // thing lights up on hover together. Always goes to Logs: on
    // disconnect it's diagnostic, on connect it's still useful for
    // seeing recent activity.
    items.push(clickable(
        Message::NavClicked(NavItem::Logs),
        row![
            text(dot).size(12).color(dot_color),
            text(label).size(12).color(*theme::FOREGROUND),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center),
    ));

    items.push(clickable(
        Message::NavClicked(NavItem::Agents),
        text(format!("{} agents tracked", snap.agents_tracked))
            .size(11)
            .color(*theme::MUTED),
    ));

    if let Some(model) = snap.active_model {
        items.push(static_chunk(
            text(format!("· {model}")).size(11).color(*theme::MUTED),
        ));
    }

    if let Some(t) = snap.last_poll {
        let age_text = format!(
            "· last poll {}",
            format_age(Instant::now().saturating_duration_since(t))
        );
        items.push(static_chunk(text(age_text).size(11).color(*theme::MUTED)));
    }

    if let Some((total, ctx)) = snap.main_usage {
        let color = if ctx > 0 && total >= ctx {
            *theme::STATUS_DOWN
        } else if ctx > 0 && total as f64 >= (ctx as f64) * 0.8 {
            *theme::STATUS_DEGRADED
        } else {
            *theme::MUTED
        };
        items.push(clickable(
            Message::NavClicked(NavItem::Sessions),
            text(format!(
                "· ctx {}/{}",
                compact_count(total),
                compact_count(ctx)
            ))
            .size(11)
            .color(color),
        ));
    }

    if snap.pending_approvals > 0 {
        items.push(clickable(
            Message::NavClicked(NavItem::Overview),
            text(format!("· {} approval(s) pending", snap.pending_approvals))
                .size(11)
                .color(*theme::STATUS_DEGRADED),
        ));
    }

    if let Some((cur, new)) = snap.update {
        // Reconnect is the right action after a gateway update —
        // the backoff can otherwise keep the client on the old
        // version for minutes longer than necessary.
        items.push(clickable(
            Message::RequestReconnect,
            text(format!("· update {cur} → {new}"))
                .size(11)
                .color(*theme::STATUS_DEGRADED),
        ));
    }

    let mut strip = row![].spacing(12).align_y(iced::Alignment::Center);
    for item in items {
        strip = strip.push(item);
    }

    container(strip)
        .width(Length::Fill)
        .padding(Padding::from([4, 10]))
        .style(|_| container::Style {
            background: Some((*theme::SURFACE_1).into()),
            border: Border {
                color: *theme::BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Wrap content in a button that still reads as inline status text
/// but picks up a subtle hover glow. The button has no visible
/// border, matches the status-bar background, and only surfaces on
/// hover — operators don't want a row of UI-chrome buttons at the
/// bottom of the screen.
fn clickable<'a, M>(message: Message, content: M) -> Element<'a, Message>
where
    M: Into<Element<'a, Message>>,
{
    button(content)
        .on_press(message)
        .padding(Padding::from([3, 6]))
        .style(|_, status| {
            let bg = match status {
                iced::widget::button::Status::Hovered => Some(
                    Color {
                        a: 0.35,
                        ..*theme::SURFACE_3
                    }
                    .into(),
                ),
                iced::widget::button::Status::Pressed => Some(
                    Color {
                        a: 0.55,
                        ..*theme::SURFACE_3
                    }
                    .into(),
                ),
                _ => None,
            };
            iced::widget::button::Style {
                background: bg,
                text_color: *theme::FOREGROUND,
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 4.0.into(),
                },
                shadow: Shadow::default(),
                ..Default::default()
            }
        })
        .into()
}

/// Same visual padding as `clickable` so non-actionable readouts
/// line up with the clickable ones — otherwise the row jitters
/// vertically as chunks toggle between button and text.
fn static_chunk<'a, M>(content: M) -> Element<'a, Message>
where
    M: Into<Element<'a, Message>>,
{
    container(content).padding(Padding::from([3, 6])).into()
}

fn connection_line<'a>(snap: &Snapshot<'a>) -> (&'static str, String) {
    if snap.connected {
        ("●", "connected".to_string())
    } else {
        match snap.last_disconnect {
            Some(reason) => ("○", format!("disconnected: {}", truncate(reason, 60))),
            None => ("○", "connecting…".to_string()),
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

/// Short-form token counts for the status bar (`34K`, `1.2M`, `420`).
fn compact_count(n: i64) -> String {
    let abs = n.unsigned_abs();
    if abs >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if abs >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        n.to_string()
    }
}

fn format_age(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_line_connected() {
        let snap = Snapshot {
            connected: true,
            agents_tracked: 0,
            last_poll: None,
            active_model: None,
            last_disconnect: None,
            main_usage: None,
            pending_approvals: 0,
            update: None,
        };
        let (dot, label) = connection_line(&snap);
        assert_eq!(dot, "●");
        assert_eq!(label, "connected");
    }

    #[test]
    fn connection_line_disconnected_with_reason() {
        let snap = Snapshot {
            connected: false,
            agents_tracked: 0,
            last_poll: None,
            active_model: None,
            last_disconnect: Some("auth failed"),
            main_usage: None,
            pending_approvals: 0,
            update: None,
        };
        let (dot, label) = connection_line(&snap);
        assert_eq!(dot, "○");
        assert_eq!(label, "disconnected: auth failed");
    }

    #[test]
    fn compact_count_scales() {
        assert_eq!(compact_count(500), "500");
        assert_eq!(compact_count(12_400), "12K");
        assert_eq!(compact_count(2_300_000), "2.3M");
    }

    #[test]
    fn truncate_respects_max() {
        assert_eq!(truncate("short", 10), "short");
        let out = truncate("a very long disconnect reason here", 10);
        assert_eq!(out, "a very lon…");
    }
}
