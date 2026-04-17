//! Top-level app state, Message, update, view.

use std::collections::HashMap;

use iced::widget::{Canvas, canvas, center, column, container, row, text};
use iced::{Element, Length, Padding, Subscription};

use crate::domain::{Agent, AgentId, AgentStatus, agent};
use crate::net::{WsEvent, events, mock, openclaw};
use crate::scene::OfficeScene;
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
    Ws(WsEvent),
}

pub struct App {
    pub nav: NavItem,
    pub roster: Vec<Agent>,
    pub statuses: HashMap<AgentId, AgentStatus>,
    pub connected: bool,
    pub last_disconnect: Option<String>,
    pub scene_cache: canvas::Cache,
}

impl Default for App {
    fn default() -> Self {
        Self {
            nav: NavItem::Overview,
            roster: agent::roster(),
            statuses: HashMap::new(),
            connected: false,
            last_disconnect: None,
            scene_cache: canvas::Cache::default(),
        }
    }
}

impl App {
    pub fn update(&mut self, message: Message) {
        match message {
            Message::NavClicked(item) => {
                self.nav = item;
            }
            Message::Ws(event) => {
                self.apply_ws(event);
                // Invalidate canvas cache so sprites re-render at new positions.
                self.scene_cache.clear();
            }
        }
    }

    fn apply_ws(&mut self, event: WsEvent) {
        match event {
            WsEvent::Connected => {
                self.connected = true;
                self.last_disconnect = None;
                tracing::info!("WS connected");
            }
            WsEvent::Disconnected(reason) => {
                self.connected = false;
                self.last_disconnect = Some(reason.clone());
                tracing::warn!(%reason, "WS disconnected");
            }
            WsEvent::CronSnapshot(crons) => {
                for cron in &crons {
                    let id = events::cron_agent_id(cron);
                    let status = events::cron_status(cron);
                    self.statuses.insert(id, status);
                }
            }
            WsEvent::ChannelSnapshot(channels) => {
                for ch in &channels {
                    let id = events::channel_agent_id(ch);
                    let status = events::channel_status(ch);
                    self.statuses.insert(id, status);
                }
            }
            WsEvent::MainAgent(main) => {
                let id = AgentId::new(&main.id);
                let status = events::main_agent_status(&main);
                self.statuses.insert(id, status);
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let main = match self.nav {
            NavItem::Overview => self.overview(),
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

    fn overview(&self) -> Element<'_, Message> {
        let scene = OfficeScene {
            roster: &self.roster,
            statuses: &self.statuses,
            cache: &self.scene_cache,
        };

        let canvas = Canvas::new(scene).width(Length::Fill).height(Length::Fill);

        let status_line = if self.connected {
            format!("● connected · {} agents tracked", self.statuses.len())
        } else {
            match &self.last_disconnect {
                Some(reason) => format!("○ disconnected: {reason}"),
                None => "○ connecting…".to_string(),
            }
        };

        let status_bar = container(text(status_line).size(12).color(if self.connected {
            *theme::TERMINAL_GREEN
        } else {
            *theme::MUTED
        }))
        .padding(Padding::from([8, 16]))
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some((*theme::SURFACE_1).into()),
            border: iced::Border {
                color: *theme::BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        });

        column![
            container(canvas)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(Padding::from(16)),
            status_bar,
        ]
        .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if mock::enabled() {
            Subscription::run(mock::connect).map(Message::Ws)
        } else {
            Subscription::run(openclaw::connect).map(Message::Ws)
        }
    }

    pub fn theme(&self) -> iced::Theme {
        theme::mission_control_theme()
    }
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
