//! Top-level app state, Message, update, view.

use iced::widget::{center, column, container, row, text};
use iced::{Element, Length, Padding};

use crate::ui::{sidebar, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItem {
    Overview,
    Agents,
    Logs,
    Settings,
}

#[derive(Debug, Clone)]
pub enum Message {
    NavClicked(NavItem),
}

pub struct App {
    pub nav: NavItem,
}

impl Default for App {
    fn default() -> Self {
        Self {
            nav: NavItem::Overview,
        }
    }
}

impl App {
    pub fn update(&mut self, message: Message) {
        match message {
            Message::NavClicked(item) => {
                self.nav = item;
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let main = match self.nav {
            NavItem::Overview => overview_placeholder(),
            NavItem::Agents => coming_soon("Agents"),
            NavItem::Logs => coming_soon("Logs"),
            NavItem::Settings => coming_soon("Settings"),
        };

        container(row![sidebar::view(self.nav), main].spacing(0))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style {
                background: Some((*theme::SURFACE_0).into()),
                ..Default::default()
            })
            .into()
    }

    pub fn theme(&self) -> iced::Theme {
        theme::mission_control_theme()
    }
}

fn overview_placeholder() -> Element<'static, Message> {
    center(
        column![
            text("AGENT OFFICE").size(32).color(*theme::TERMINAL_GREEN),
            text("pixel-art scene renders here in M2")
                .size(14)
                .color(*theme::MUTED),
        ]
        .spacing(12)
        .align_x(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(Padding::from(24))
    .style(|_| container::Style {
        background: Some((*theme::SURFACE_0).into()),
        ..Default::default()
    })
    .into()
}

fn coming_soon(title: &'static str) -> Element<'static, Message> {
    center(
        column![
            text(title).size(24).color(*theme::FOREGROUND),
            text("coming soon").size(13).color(*theme::MUTED),
        ]
        .spacing(8)
        .align_x(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(Padding::from(24))
    .into()
}
