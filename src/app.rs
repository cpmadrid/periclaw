//! Top-level app state, Message, update, view.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use iced::widget::{Canvas, canvas};
use iced::{Element, Length, Subscription, time};

use crate::domain::{Agent, AgentId, AgentKind, AgentStatus, agent};
use crate::net::events::{ActivityKind, GatewayUpdate};
use crate::net::rpc::{ApprovalEventPayload, Channel, CronState};
use crate::net::{WsEvent, events, mock, openclaw};
use crate::scene::{OfficeScene, ThoughtBubble, transition_text};
use crate::ui::{agent_card, agents_view, approvals, sidebar, status_bar, theme};

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
    /// Operator resolved a pending exec approval from the UI. The
    /// decision string matches OpenClaw's `ExecApprovalDecision`
    /// (`"allow-once" | "deny"` — `allow-always` intentionally not
    /// surfaced from the desktop to keep blast radius low).
    ResolveApproval {
        id: String,
        decision: &'static str,
    },
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
    /// Per-session token usage (`totalTokens` / `contextTokens`).
    /// Populated from the sessions.list snapshot and kept fresh via
    /// session.message events. Lets the status bar warn when the
    /// main session is near its context ceiling.
    pub session_usage: HashMap<String, SessionUsage>,
    /// Gateway-side update notification, when one is pending.
    pub gateway_update: Option<GatewayUpdate>,
    /// Full cron state per cron agent — keeps schedule-adjacent fields
    /// (`nextRunAtMs`, `lastRunAtMs`, `lastDurationMs`, `lastError`)
    /// that the Agents tab shows but the Overview sprite doesn't need.
    /// Populated from both the `cron.list` snapshot and the `cron`
    /// delta stream.
    pub cron_details: HashMap<AgentId, CronState>,
    /// Full channel state per channel agent (connected, configured,
    /// last error). Refreshed by the 30s `channels.status` heartbeat.
    pub channel_details: HashMap<AgentId, Channel>,
    pub scene_cache: canvas::Cache,
}

#[derive(Debug, Clone, Copy)]
pub struct SessionUsage {
    pub total_tokens: i64,
    pub context_tokens: i64,
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
            session_usage: HashMap::new(),
            gateway_update: None,
            cron_details: HashMap::new(),
            channel_details: HashMap::new(),
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
            Message::ResolveApproval { id, decision } => {
                tracing::info!(id = %id, decision, "UI: resolve approval");
                // Optimistically drop the entry so the panel collapses
                // immediately; the gateway's `exec.approval.resolved`
                // event will arrive shortly and confirm. If the RPC
                // fails, the operator can retry when/if the event
                // re-fires (real rare case).
                self.pending_approvals.remove(&id);
                if let Err(e) = crate::net::commands::sender().send(
                    crate::net::commands::GatewayCommand::ResolveApproval {
                        id,
                        decision: decision.to_string(),
                    },
                ) {
                    tracing::warn!(error = %e, "could not dispatch ResolveApproval command");
                }
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
                    self.cron_details.insert(id.clone(), cron.state.clone());
                    let status = events::cron_status(cron);
                    self.apply_status_update(id, status);
                }
            }
            WsEvent::CronDelta(cron) => {
                self.last_poll = Some(Instant::now());
                let id = events::cron_agent_id(&cron);
                self.ensure_agent(&id, AgentKind::Cron);
                // Merge rather than replace — push events only carry
                // the fields that changed, so a `finished` delta
                // shouldn't wipe the unrelated `nextRunAtMs` from the
                // previous snapshot.
                merge_cron_state(
                    self.cron_details.entry(id.clone()).or_default(),
                    &cron.state,
                );
                let status = events::cron_status(&cron);
                self.apply_status_update(id, status);
            }
            WsEvent::ChannelSnapshot(channels) => {
                self.last_poll = Some(Instant::now());
                for ch in &channels {
                    let id = events::channel_agent_id(ch);
                    self.ensure_agent(&id, AgentKind::Channel);
                    self.channel_details.insert(id.clone(), ch.clone());
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
            WsEvent::AgentsIdentity(agents) => {
                for info in &agents {
                    // Only entries with an explicit identity name
                    // override the display; others keep their
                    // roster-seeded label.
                    let Some(name) = info.name.as_deref().filter(|s| !s.is_empty()) else {
                        continue;
                    };
                    let new_display = match info.emoji.as_deref() {
                        Some(emoji) if !emoji.is_empty() => format!("{name} {emoji}"),
                        _ => name.to_string(),
                    };
                    let Some(entry) = self.roster.iter_mut().find(|a| a.id.as_str() == info.id)
                    else {
                        continue;
                    };
                    if entry.display != new_display {
                        tracing::info!(id = %info.id, display = %new_display, "roster: identity rename");
                        entry.display = new_display;
                        self.scene_cache.clear();
                    }
                }
            }
            WsEvent::AgentMessage { agent_id, text } => {
                self.last_poll = Some(Instant::now());
                // Real agent text goes straight into a bubble — bypasses
                // `apply_status_update` since this isn't a status change.
                // `clean_bubble_text` strips markdown fences and collapses
                // whitespace so multi-line code-block replies read as a
                // single legible line.
                let snippet = clean_bubble_text(&text, 80);
                if snippet.is_empty() {
                    tracing::debug!(
                        agent = %agent_id.as_str(),
                        "agent message empty after cleanup, skipping bubble",
                    );
                } else {
                    tracing::info!(
                        agent = %agent_id.as_str(),
                        preview = %snippet,
                        "agent message → bubble",
                    );
                    self.bubbles.push(ThoughtBubble::message(agent_id, snippet));
                }
            }
            WsEvent::AgentToolInvoked { agent_id, text } => {
                self.last_poll = Some(Instant::now());
                let snippet = clean_bubble_text(&text, 80);
                if !snippet.is_empty() {
                    tracing::info!(
                        agent = %agent_id.as_str(),
                        preview = %snippet,
                        "tool invoke → bubble",
                    );
                    self.bubbles.push(ThoughtBubble::tool(agent_id, snippet));
                }
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
            WsEvent::SessionUsage {
                session_key,
                total_tokens,
                context_tokens,
            } => {
                tracing::debug!(
                    session = %session_key,
                    total = total_tokens,
                    ctx = context_tokens,
                    "session usage",
                );
                self.session_usage.insert(
                    session_key,
                    SessionUsage {
                        total_tokens,
                        context_tokens,
                    },
                );
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
            WsEvent::UpdateAvailable(update) => {
                self.gateway_update = update;
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
            NavItem::Agents => agents_view::view(agents_view::AgentsViewSnapshot {
                roster: &self.roster,
                statuses: &self.statuses,
                cron_details: &self.cron_details,
                channel_details: &self.channel_details,
                active_model: self.active_model.as_deref(),
                session_usage: &self.session_usage,
            }),
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

        let main_usage = self
            .session_usage
            .get("agent:main:main")
            .map(|u| (u.total_tokens, u.context_tokens));
        let status = status_bar::view(status_bar::Snapshot {
            connected: self.connected,
            agents_tracked: self.statuses.len(),
            last_poll: self.last_poll,
            active_model: self.active_model.as_deref(),
            last_disconnect: self.last_disconnect.as_deref(),
            main_usage,
            pending_approvals: self.pending_approvals.len(),
            update: self
                .gateway_update
                .as_ref()
                .map(|u| (u.current.as_str(), u.latest.as_str())),
        });

        // The approvals panel is a no-op row (empty iterator) when
        // nothing's pending, so we can always include it in the
        // layout without case-splitting on length.
        let approvals_panel = if self.pending_approvals.is_empty() {
            None
        } else {
            Some(approvals::view(self.pending_approvals.iter()))
        };

        let mut col = iced::widget::column![
            iced::widget::container(canvas)
                .width(Length::Fill)
                .height(Length::FillPortion(3))
                .padding(iced::Padding::from(16)),
            cards,
        ]
        .spacing(0);
        if let Some(panel) = approvals_panel {
            col = col.push(panel);
        }
        col.push(status).into()
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

/// Turn raw assistant text into something that reads cleanly on a
/// single-line bubble. Strips markdown code fences, collapses
/// whitespace, drops common emphasis markers, then clips to `max`
/// Unicode scalars with a trailing `…` when truncated.
/// Merge a cron-state delta onto the stored value — push events only
/// carry fields that changed, so a bare `finished` delta must not
/// wipe `nextRunAtMs` from the previous snapshot. `running` is the
/// one field we always copy since the lifecycle transition is the
/// whole point of the event.
fn merge_cron_state(dst: &mut CronState, src: &CronState) {
    dst.running = src.running;
    if src.next_run_at_ms.is_some() {
        dst.next_run_at_ms = src.next_run_at_ms;
    }
    if src.last_run_at_ms.is_some() {
        dst.last_run_at_ms = src.last_run_at_ms;
    }
    if src.last_status.is_some() {
        dst.last_status = src.last_status.clone();
    }
    if src.last_duration_ms.is_some() {
        dst.last_duration_ms = src.last_duration_ms;
    }
    if src.last_error.is_some() {
        dst.last_error = src.last_error.clone();
    }
}

fn clean_bubble_text(raw: &str, max: usize) -> String {
    let mut body: Vec<&str> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            // Fence open or close — drop entirely; the language tag
            // after ``` isn't interesting on a bubble either.
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        body.push(trimmed);
    }
    // Collapse runs of whitespace across the joined text and drop
    // markdown emphasis asterisks/underscores that would read as
    // literal characters in the bubble font.
    let joined = body.join(" ");
    let mut compact = String::with_capacity(joined.len());
    let mut prev_space = false;
    for ch in joined.chars() {
        match ch {
            // Drop markdown emphasis (`*bold*`) and inline-code
            // backticks. Leave `_` alone — legitimate identifiers
            // like `x86_64` or `session_key` contain it.
            '*' | '`' => continue,
            c if c.is_whitespace() => {
                if !prev_space && !compact.is_empty() {
                    compact.push(' ');
                }
                prev_space = true;
            }
            c => {
                compact.push(c);
                prev_space = false;
            }
        }
    }
    let trimmed = compact.trim();
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

#[cfg(test)]
mod bubble_cleanup_tests {
    use super::clean_bubble_text;

    #[test]
    fn strips_code_fences_and_joins_lines() {
        let input = "```\nLinux ubu-3xdv 6.8.0-110-generic\nx86_64 GNU/Linux\n```";
        assert_eq!(
            clean_bubble_text(input, 80),
            "Linux ubu-3xdv 6.8.0-110-generic x86_64 GNU/Linux"
        );
    }

    #[test]
    fn drops_markdown_emphasis_and_collapses_whitespace() {
        let input = "Done with **step 1** and    *step 2*.";
        assert_eq!(clean_bubble_text(input, 80), "Done with step 1 and step 2.");
    }

    #[test]
    fn truncates_with_ellipsis() {
        let input = "a".repeat(200);
        let out = clean_bubble_text(&input, 10);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn empty_after_cleanup_returns_empty() {
        assert_eq!(clean_bubble_text("```\n```", 80), "");
        assert_eq!(clean_bubble_text("   ", 80), "");
    }
}
