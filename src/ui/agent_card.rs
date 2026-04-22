//! Agent card row rendered below the Office scene in Overview.
//!
//! One card per agent in the static roster; each shows a colored sprite
//! dot, the display name, and a status badge. Cards are laid out in a
//! horizontally scrollable row so the full roster always fits.

use std::collections::HashMap;

use iced::widget::{column, container, row, scrollable, text};
use iced::{Element, Length, Padding};

use crate::Message;
use crate::domain::{Agent, AgentId, AgentStatus};
use crate::ui::theme;
use crate::ui::widgets::{card_style, colored_dot, status_pill};

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
    let header = row![
        colored_dot(agent.color()),
        text(agent.display.as_str())
            .size(13)
            .color(*theme::FOREGROUND),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    container(column![header, status_pill(status)].spacing(6))
        .width(Length::Fixed(160.0))
        .padding(Padding::from([10, 12]))
        .style(card_style(6.0))
        .into()
}
