//! Left-side nav: Overview / Agents / Logs / Settings.

use iced::widget::{button, column, container, text};
use iced::{Element, Length, Padding};

use crate::Message;
use crate::app::NavItem;
use crate::ui::theme;

pub fn view(active: NavItem) -> Element<'static, Message> {
    let items = [
        NavItem::Overview,
        NavItem::Chat,
        NavItem::Agents,
        NavItem::Sessions,
        NavItem::Logs,
        NavItem::Settings,
    ];

    let nav = items.into_iter().fold(column![], |col, item| {
        col.push(nav_button(item, item == active))
    });

    let header = column![
        container(text("MISSION").size(12).color(*theme::MUTED)).padding(Padding::from([12, 16])),
        container(text("CONTROL").size(18).color(*theme::TERMINAL_GREEN),)
            .padding(Padding::from([0, 16])),
    ];

    container(column![header, nav.spacing(4)].spacing(24))
        .width(Length::Fixed(220.0))
        .height(Length::Fill)
        .style(|_| container::Style {
            background: Some((*theme::SURFACE_1).into()),
            border: iced::Border {
                color: *theme::BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn nav_button(item: NavItem, active: bool) -> Element<'static, Message> {
    let label = match item {
        NavItem::Overview => "Overview",
        NavItem::Chat => "Chat",
        NavItem::Agents => "Agents",
        NavItem::Sessions => "Sessions",
        NavItem::Logs => "Logs",
        NavItem::Settings => "Settings",
    };

    button(text(label).size(14).color(if active {
        *theme::TERMINAL_GREEN
    } else {
        *theme::FOREGROUND
    }))
    .on_press(Message::NavClicked(item))
    .width(Length::Fill)
    .padding(Padding::from([10, 16]))
    .style(move |_, status| {
        let bg = if active {
            *theme::SURFACE_2
        } else {
            match status {
                button::Status::Hovered => *theme::SURFACE_2,
                _ => iced::Color::TRANSPARENT,
            }
        };
        button::Style {
            background: Some(bg.into()),
            text_color: if active {
                *theme::TERMINAL_GREEN
            } else {
                *theme::FOREGROUND
            },
            border: iced::Border::default(),
            shadow: iced::Shadow::default(),
            ..Default::default()
        }
    })
    .into()
}
