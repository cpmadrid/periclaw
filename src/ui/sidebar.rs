//! Left-side nav: Overview / Chat / Agents / Sessions / Logs /
//! Settings. The Chat row shows an unread-count badge when any
//! agent has messages the operator hasn't seen yet.

use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use crate::Message;
use crate::app::NavItem;
use crate::ui::theme;

pub fn view(active: NavItem, unread_chat: usize) -> Element<'static, Message> {
    let items = [
        NavItem::Overview,
        NavItem::Chat,
        NavItem::Agents,
        NavItem::Sessions,
        NavItem::Logs,
        NavItem::Settings,
    ];

    let nav = items.into_iter().fold(column![], |col, item| {
        let badge = if item == NavItem::Chat && unread_chat > 0 {
            Some(unread_chat)
        } else {
            None
        };
        col.push(nav_button(item, item == active, badge))
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

fn nav_button(item: NavItem, active: bool, badge: Option<usize>) -> Element<'static, Message> {
    let label = match item {
        NavItem::Overview => "Overview",
        NavItem::Chat => "Chat",
        NavItem::Agents => "Agents",
        NavItem::Sessions => "Sessions",
        NavItem::Logs => "Logs",
        NavItem::Settings => "Settings",
    };
    let fg = if active {
        *theme::TERMINAL_GREEN
    } else {
        *theme::FOREGROUND
    };

    let label_el = text(label).size(14).color(fg);
    let content: Element<'static, Message> = match badge {
        Some(n) => row![label_el, unread_badge(n)]
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
        None => label_el.into(),
    };

    button(content)
        .on_press(Message::NavClicked(item))
        .width(Length::Fill)
        .padding(Padding::from([10, 16]))
        .style(move |_, status| {
            let bg = if active {
                *theme::SURFACE_2
            } else {
                match status {
                    button::Status::Hovered => *theme::SURFACE_2,
                    _ => Color::TRANSPARENT,
                }
            };
            button::Style {
                background: Some(bg.into()),
                text_color: fg,
                border: Border::default(),
                shadow: iced::Shadow::default(),
                ..Default::default()
            }
        })
        .into()
}

/// Small count pill shown next to the Chat row when unread > 0.
/// Exported so the Chat-tab picker can render the same shape for
/// per-agent counts.
pub fn unread_badge(count: usize) -> Element<'static, Message> {
    // Collapse large counts to "99+" so the pill stays narrow.
    let label = if count > 99 {
        "99+".to_string()
    } else {
        count.to_string()
    };
    container(
        text(label)
            .size(10)
            .color(*theme::SURFACE_0)
            .font(iced::Font::MONOSPACE),
    )
    .padding(Padding::from([1, 6]))
    .style(|_| container::Style {
        background: Some((*theme::TERMINAL_GREEN).into()),
        border: Border {
            color: *theme::TERMINAL_GREEN,
            width: 0.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    })
    .into()
}
