//! Top-level app state, Message, update, view.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use iced::widget::{Canvas, canvas};
use iced::{Element, Length, Subscription, time};

use crate::domain::{Agent, AgentId, AgentKind, AgentStatus, agent};
use crate::net::events::ActivityKind;
use crate::net::rpc::ApprovalEventPayload;
use crate::net::{WsEvent, events, mock, openclaw};
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
    /// Timestamp of the most recent state update (push event OR bootstrap
    /// RPC). Despite the name, not a polling indicator — kept for the
    /// status-bar "last activity" readout.
    pub last_poll: Option<Instant>,
    pub connected: bool,
    pub last_disconnect: Option<String>,
    /// Pending exec approvals keyed by approval id. Populated by
    /// `exec.approval.requested`, cleared by `.resolved`. Scope-gated
    /// (empty unless the gateway granted `operator.read`+approvals).
    pub pending_approvals: HashMap<String, ApprovalEventPayload>,
    pub scene_cache: canvas::Cache,
}

impl Default for App {
    fn default() -> Self {
        Self {
            nav: NavItem::Overview,
            roster: agent::seed_roster(),
            statuses: HashMap::new(),
            bubbles: Vec::new(),
            active_model: None,
            last_poll: None,
            connected: false,
            last_disconnect: None,
            pending_approvals: HashMap::new(),
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
                    self.ensure_agent(&id, AgentKind::Cron);
                    let status = events::cron_status(cron);
                    self.apply_status_update(id, status);
                }
            }
            WsEvent::CronDelta(cron) => {
                self.last_poll = Some(Instant::now());
                let id = events::cron_agent_id(&cron);
                self.ensure_agent(&id, AgentKind::Cron);
                let status = events::cron_status(&cron);
                self.apply_status_update(id, status);
            }
            WsEvent::ChannelSnapshot(channels) => {
                self.last_poll = Some(Instant::now());
                for ch in &channels {
                    let id = events::channel_agent_id(ch);
                    self.ensure_agent(&id, AgentKind::Channel);
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
            WsEvent::AgentMessage { agent_id, text } => {
                self.last_poll = Some(Instant::now());
                // Real agent text goes straight into a bubble — bypasses
                // `apply_status_update` since this isn't a status change.
                // Trim to a reasonable bubble length; the canvas renderer
                // sizes the bubble by text.len().
                let snippet = truncate(&text, 80);
                tracing::info!(
                    agent = %agent_id.as_str(),
                    preview = %snippet,
                    "agent message → bubble",
                );
                self.bubbles
                    .push(ThoughtBubble::message(agent_id, snippet));
            }
            WsEvent::AgentActivity { agent_id, kind } => {
                self.last_poll = Some(Instant::now());
                let status = match kind {
                    ActivityKind::Thinking | ActivityKind::ToolCalling => AgentStatus::Running,
                    ActivityKind::Errored => AgentStatus::Error,
                };
                self.apply_status_update(agent_id, status);
            }
            WsEvent::SessionsChanged => {
                self.last_poll = Some(Instant::now());
                tracing::trace!("sessions.changed");
            }
            WsEvent::ApprovalRequested(payload) => {
                self.last_poll = Some(Instant::now());
                let key = payload.id.clone().unwrap_or_else(|| {
                    // No id — best-effort key from tool+summary so
                    // resolved(null-id) still matches something.
                    format!(
                        "{}:{}",
                        payload.tool.as_deref().unwrap_or("?"),
                        payload.summary.as_deref().unwrap_or(""),
                    )
                });
                self.pending_approvals.insert(key, payload);
            }
            WsEvent::ApprovalResolved { id } => {
                self.last_poll = Some(Instant::now());
                if let Some(id) = id.as_deref() {
                    self.pending_approvals.remove(id);
                } else {
                    // Unidentified resolve — safest to clear all since we
                    // can't tell which survived.
                    self.pending_approvals.clear();
                }
            }
        }
    }

    /// Ensure a sprite exists in the roster for `id`. First-time seen
    /// IDs (cron rename on the gateway, a new channel provider) get a
    /// fresh `Agent` with a deterministic color and the canvas cache is
    /// cleared so the sprite is painted this frame.
    fn ensure_agent(&mut self, id: &AgentId, kind: AgentKind) {
        if self.roster.iter().any(|a| a.id == *id) {
            return;
        }
        let agent = match kind {
            AgentKind::Cron => Agent::cron(id.as_str()),
            AgentKind::Channel => Agent::channel(id.as_str()),
            AgentKind::Main => {
                // `main` is already in the seed roster; any drift is a bug.
                tracing::warn!(id = %id.as_str(), "unexpected Main agent add");
                return;
            }
        };
        tracing::info!(id = %id.as_str(), kind = ?kind, "roster: new agent");
        self.roster.push(agent);
        self.scene_cache.clear();
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
        // `OPENCLAW_MOCK=1` routes to the scripted fixture stream for UI
        // work without a live gateway; otherwise we run the native WS.
        let ws = if mock::enabled() {
            Subscription::run(mock::connect).map(Message::Ws)
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

/// Clip a string to at most `max` chars (by Unicode scalar value),
/// appending `…` when truncated. Used to keep chat bubbles legible.
fn truncate(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
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
