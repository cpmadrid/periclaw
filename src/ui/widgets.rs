//! Shared UI primitives used across multiple views.
//!
//! Each helper here is used in at least two views — when a widget
//! only appears in one place it stays local to that module so a
//! consumer doesn't have to chase a helper to understand a view.

use iced::widget::{container, text};
use iced::{Border, Color, Element, Length, Padding};

use crate::Message;
use crate::domain::AgentStatus;
use crate::ui::theme;

/// A small filled-circle "status indicator" chip — sits next to an
/// agent/job name in cards, agent rows, and the Chat picker.
pub fn colored_dot(color: Color) -> Element<'static, Message> {
    container(text(""))
        .width(Length::Fixed(10.0))
        .height(Length::Fixed(10.0))
        .style(move |_| container::Style {
            background: Some(color.into()),
            border: Border {
                color,
                width: 0.0,
                radius: 5.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// "RUNNING" / "OK" / "ERROR" pill shown on the right of agent rows.
pub fn status_pill(status: AgentStatus) -> Element<'static, Message> {
    let (label, color) = status_label(status);
    container(text(label).size(10).color(color))
        .padding(Padding::from([2, 6]))
        .style(move |_| container::Style {
            background: Some(Color { a: 0.15, ..color }.into()),
            border: Border {
                color: Color { a: 0.45, ..color },
                width: 1.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Text + color pair for each [`AgentStatus`]. Callers that want just
/// the label (without the pill chrome — e.g. a status-bar label line)
/// can reuse this instead of re-matching.
pub fn status_label(status: AgentStatus) -> (&'static str, Color) {
    match status {
        AgentStatus::Running => ("RUNNING", *theme::TERMINAL_GREEN),
        AgentStatus::Ok => ("OK", *theme::STATUS_UP),
        AgentStatus::Error => ("ERROR", *theme::STATUS_DOWN),
        AgentStatus::Disabled => ("OFF", *theme::MUTED),
        AgentStatus::Unknown => ("?", *theme::STATUS_UNKNOWN),
    }
}

/// Standard card container style — SURFACE_1 background with the
/// theme's border color at 1px. Used by anything that wants to look
/// like an Agents-tab row or a Settings-tab form section. Radius is
/// a parameter because different surfaces want slightly different
/// curves.
pub fn card_style(radius: f32) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        background: Some((*theme::SURFACE_1).into()),
        border: Border {
            color: *theme::BORDER,
            width: 1.0,
            radius: radius.into(),
        },
        ..Default::default()
    }
}
