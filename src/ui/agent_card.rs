//! Agent card row rendered below the Office scene in Overview.
//!
//! One card per agent in the static roster; each shows a colored sprite
//! dot, the display name, and a status badge. Cards are laid out in a
//! horizontally scrollable row so the full 8-agent roster always fits.

use std::collections::HashMap;

use iced::widget::{column, container, row, scrollable, text};
use iced::{Border, Element, Length, Padding};

use crate::Message;
use crate::domain::{Agent, AgentId, AgentStatus};
use crate::ui::theme;

pub fn row_view<'a>(
    roster: &'a [Agent],
    statuses: &'a HashMap<AgentId, AgentStatus>,
) -> Element<'a, Message> {
    let cards = roster.iter().fold(row![].spacing(10), |acc, agent| {
        let status = statuses
            .get(&agent.id)
            .copied()
            .unwrap_or(AgentStatus::Unknown);
        acc.push(card(agent, status))
    });

    container(
        scrollable(cards)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::new().width(4).scroller_width(4),
            ))
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .padding(Padding::from([12, 16]))
    .style(|_| container::Style {
        background: Some((*theme::SURFACE_0).into()),
        ..Default::default()
    })
    .into()
}

fn card<'a>(agent: &'a Agent, status: AgentStatus) -> Element<'a, Message> {
    let (badge_label, badge_color) = status_badge(status);
    let dot = container(text(""))
        .width(Length::Fixed(10.0))
        .height(Length::Fixed(10.0))
        .style(move |_| container::Style {
            background: Some(agent.color().into()),
            border: Border {
                color: agent.color(),
                width: 0.0,
                radius: 5.0.into(),
            },
            ..Default::default()
        });

    let header = row![
        dot,
        text(agent.display.as_str())
            .size(13)
            .color(*theme::FOREGROUND),
    ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

    let badge = container(text(badge_label).size(10).color(badge_color))
        .padding(Padding::from([2, 6]))
        .style(move |_| container::Style {
            background: Some(
                iced::Color {
                    a: 0.15,
                    ..badge_color
                }
                .into(),
            ),
            border: Border {
                color: iced::Color {
                    a: 0.45,
                    ..badge_color
                },
                width: 1.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        });

    container(column![header, badge].spacing(6))
        .width(Length::Fixed(160.0))
        .padding(Padding::from([10, 12]))
        .style(|_| container::Style {
            background: Some((*theme::SURFACE_1).into()),
            border: Border {
                color: *theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn status_badge(status: AgentStatus) -> (&'static str, iced::Color) {
    match status {
        AgentStatus::Running => ("RUNNING", *theme::TERMINAL_GREEN),
        AgentStatus::Ok => ("OK", *theme::STATUS_UP),
        AgentStatus::Error => ("ERROR", *theme::STATUS_DOWN),
        AgentStatus::Disabled => ("OFF", *theme::MUTED),
        AgentStatus::Unknown => ("?", *theme::STATUS_UNKNOWN),
    }
}
