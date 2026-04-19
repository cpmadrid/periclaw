//! Bottom status bar showing connection, agent count, active model,
//! and last-poll age.

use std::time::Instant;

use iced::widget::{container, row, text};
use iced::{Border, Element, Length, Padding};

use crate::Message;
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
    let agents = format!("{} agents tracked", snap.agents_tracked);
    let model = snap
        .active_model
        .map(|m| format!("· {m}"))
        .unwrap_or_default();
    let age = snap
        .last_poll
        .map(|t| {
            format!(
                "· last poll {}",
                format_age(Instant::now().saturating_duration_since(t))
            )
        })
        .unwrap_or_default();

    let (ctx_text, ctx_color) = match snap.main_usage {
        None => (String::new(), *theme::MUTED),
        Some((total, ctx)) => {
            // Paint yellow when we cross 80% of ctx, red when over.
            let color = if ctx > 0 && total >= ctx {
                *theme::STATUS_DOWN
            } else if ctx > 0 && total as f64 >= (ctx as f64) * 0.8 {
                *theme::STATUS_DEGRADED
            } else {
                *theme::MUTED
            };
            (
                format!("· ctx {}/{}", compact_count(total), compact_count(ctx)),
                color,
            )
        }
    };

    let approvals_text = if snap.pending_approvals > 0 {
        format!("· {} approval(s) pending", snap.pending_approvals)
    } else {
        String::new()
    };

    let update_text = snap
        .update
        .map(|(cur, new)| format!("· update {cur} → {new}"))
        .unwrap_or_default();

    container(
        row![
            text(dot).size(12).color(if snap.connected {
                *theme::TERMINAL_GREEN
            } else {
                *theme::MUTED
            }),
            text(label).size(12).color(*theme::FOREGROUND),
            text(agents).size(11).color(*theme::MUTED),
            text(model).size(11).color(*theme::MUTED),
            text(age).size(11).color(*theme::MUTED),
            text(ctx_text).size(11).color(ctx_color),
            text(approvals_text).size(11).color(*theme::STATUS_DEGRADED),
            text(update_text).size(11).color(*theme::STATUS_DEGRADED),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([8, 16]))
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
