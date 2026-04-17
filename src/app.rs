//! Top-level app state, Message, update, view.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use iced::widget::{Canvas, canvas};
use iced::{Element, Length, Subscription, time};

use crate::domain::{Agent, AgentId, AgentStatus, agent};
use crate::net::{WsEvent, events, mock, openclaw, openclaw_ssh};
use crate::scene::{OfficeScene, ThoughtBubble, transition_text};
use crate::ui::{agent_card, sidebar, status_bar, theme};

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
    Tick,
}

pub struct App {
    pub nav: NavItem,
    pub roster: Vec<Agent>,
    pub statuses: HashMap<AgentId, AgentStatus>,
    pub bubbles: Vec<ThoughtBubble>,
    pub active_model: Option<String>,
    pub last_poll: Option<Instant>,
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
            bubbles: Vec::new(),
            active_model: None,
            last_poll: None,
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
            Message::Tick => {
                let now = Instant::now();
                let before = self.bubbles.len();
                self.bubbles.retain(|b| !b.expired(now));
                if self.bubbles.len() != before {
                    self.scene_cache.clear();
                } else if !self.bubbles.is_empty() {
                    // Force a redraw while bubbles exist so their alpha animates.
                    self.scene_cache.clear();
                }
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
                self.last_poll = Some(Instant::now());
                for cron in &crons {
                    let id = events::cron_agent_id(cron);
                    let status = events::cron_status(cron);
                    self.apply_status_update(id, status);
                }
            }
            WsEvent::ChannelSnapshot(channels) => {
                self.last_poll = Some(Instant::now());
                for ch in &channels {
                    let id = events::channel_agent_id(ch);
                    let status = events::channel_status(ch);
                    self.apply_status_update(id, status);
                }
            }
            WsEvent::MainAgent(main) => {
                if let Some(model) = main.model.as_ref() {
                    self.active_model = Some(model.clone());
                }
                let id = AgentId::new(&main.id);
                let status = events::main_agent_status(&main);
                self.apply_status_update(id, status);
            }
        }
    }

    fn apply_status_update(&mut self, id: AgentId, next: AgentStatus) {
        let prev = self.statuses.get(&id).copied();
        if prev == Some(next) {
            return;
        }
        self.statuses.insert(id.clone(), next);
        if let Some(text) = transition_text(prev, next) {
            self.bubbles.push(ThoughtBubble::new(id, text));
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let main = match self.nav {
            NavItem::Overview => self.overview(),
            NavItem::Agents => coming_soon("Agents"),
            NavItem::Logs => coming_soon("Logs"),
            NavItem::Settings => coming_soon("Settings"),
        };

        iced::widget::container(iced::widget::row![sidebar::view(self.nav), main].spacing(0))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| iced::widget::container::Style {
                background: Some((*theme::SURFACE_0).into()),
                ..Default::default()
            })
            .into()
    }

    fn overview(&self) -> Element<'_, Message> {
        let scene = OfficeScene {
            roster: &self.roster,
            statuses: &self.statuses,
            bubbles: &self.bubbles,
            cache: &self.scene_cache,
        };

        let canvas = Canvas::new(scene).width(Length::Fill).height(Length::Fill);

        let cards = agent_card::row_view(&self.roster, &self.statuses);

        let status = status_bar::view(status_bar::Snapshot {
            connected: self.connected,
            agents_tracked: self.statuses.len(),
            last_poll: self.last_poll,
            active_model: self.active_model.as_deref(),
            last_disconnect: self.last_disconnect.as_deref(),
        });

        iced::widget::column![
            iced::widget::container(canvas)
                .width(Length::Fill)
                .height(Length::FillPortion(3))
                .padding(iced::Padding::from(16)),
            cards,
            status,
        ]
        .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        // Selector: mock > ssh+cli > native ws (scaffolded, handshake TBD).
        let ws = if mock::enabled() {
            Subscription::run(mock::connect).map(Message::Ws)
        } else if openclaw_ssh::host().is_some() {
            Subscription::run(openclaw_ssh::connect).map(Message::Ws)
        } else {
            Subscription::run(openclaw::connect).map(Message::Ws)
        };

        // Idle-aware tick: 33ms while bubbles are animating, 500ms otherwise.
        let tick_interval = if self.bubbles.is_empty() {
            Duration::from_millis(500)
        } else {
            Duration::from_millis(33)
        };
        let tick = time::every(tick_interval).map(|_| Message::Tick);

        Subscription::batch([ws, tick])
    }

    pub fn theme(&self) -> iced::Theme {
        theme::mission_control_theme()
    }
}

fn coming_soon(title: &'static str) -> Element<'static, Message> {
    iced::widget::center(
        iced::widget::column![
            iced::widget::text(title).size(24).color(*theme::FOREGROUND),
            iced::widget::text("coming soon")
                .size(13)
                .color(*theme::MUTED),
        ]
        .spacing(8)
        .align_x(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(iced::Padding::from(24))
    .into()
}
