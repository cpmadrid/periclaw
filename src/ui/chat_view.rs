//! Multi-agent chat tab.
//!
//! Layout: left column picker (one row per chat-capable agent, rows
//! styled after the main sidebar's active-item state), right pane
//! with header (agent display + optional subtitle), scrollable
//! transcript, and the shared `chat_input::view` pinned at the bottom.
//!
//! History is accumulated in-session (operator prompts + `session.
//! message` assistant events) and bootstrapped per-agent from
//! `chat.history` the first time the operator opens that agent.
//! Logs persist across reconnects; only the "hydrated this
//! connection" flag is reset on disconnect.

use std::collections::VecDeque;
use std::time::SystemTime;

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Border, Color, Element, Length, Padding, Shadow};

use crate::Message;
use crate::app::{ChatActivity, ChatActivityState};
use crate::domain::AgentId;
use crate::net::rpc::AgentInfo;
use crate::ui::{chat_bubble, chat_input, theme};

/// In-memory representation of a chat-log entry. Role drives
/// bubble styling; text is the normalized single-string body
/// (already flattened from gateway content chunks).
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub text: String,
    pub at: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    /// System / tool output — rendered muted, kept separate from
    /// the primary back-and-forth so it doesn't dominate.
    Other,
}

impl ChatMessage {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            text: text.into(),
            at: SystemTime::now(),
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            text: text.into(),
            at: SystemTime::now(),
        }
    }
}

pub fn view<'a>(
    agents: &'a [AgentInfo],
    selected: &AgentId,
    log: Option<&'a VecDeque<ChatMessage>>,
    activity: Option<&'a ChatActivityState>,
    input: &'a str,
    connected: bool,
    unread: &'a std::collections::HashMap<AgentId, usize>,
) -> Element<'a, Message> {
    let picker = picker_column(agents, selected, unread);
    let pane = right_pane(agents, selected, log, activity, input, connected);

    row![picker, pane]
        .spacing(0)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn picker_column<'a>(
    agents: &'a [AgentInfo],
    selected: &AgentId,
    unread: &'a std::collections::HashMap<AgentId, usize>,
) -> Element<'a, Message> {
    // Always include the seeded `main` placeholder even if agents.list
    // hasn't returned yet, so the column isn't empty on first paint.
    let mut rows = column![].spacing(2);
    if agents.is_empty() {
        rows = rows.push(
            container(text("discovering agents…").size(11).color(*theme::MUTED))
                .padding(Padding::from([10, 16])),
        );
    } else {
        for info in agents {
            let count = unread
                .get(&AgentId::new(info.id.clone()))
                .copied()
                .unwrap_or(0);
            rows = rows.push(picker_row(info, selected, count));
        }
    }

    container(
        column![
            container(text("AGENTS").size(10).color(*theme::MUTED),)
                .padding(Padding::from([14, 16])),
            rows,
        ]
        .spacing(4),
    )
    .width(Length::Fixed(200.0))
    .height(Length::Fill)
    .style(|_| container::Style {
        background: Some((*theme::SURFACE_1).into()),
        border: Border {
            color: *theme::BORDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .into()
}

fn picker_row<'a>(info: &'a AgentInfo, selected: &AgentId, unread: usize) -> Element<'a, Message> {
    let active = info.id == selected.as_str();
    let label = info.display_with_emoji();
    let agent_id = AgentId::new(info.id.clone());

    let label_el = text(label).size(13).color(if active {
        *theme::TERMINAL_GREEN
    } else {
        *theme::FOREGROUND
    });
    let content: Element<'a, Message> = if unread > 0 {
        row![
            label_el,
            Space::new().width(Length::Fill),
            crate::ui::sidebar::unread_badge(unread),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    } else {
        label_el.into()
    };

    button(content)
        .on_press(Message::SelectChatAgent(agent_id))
        .width(Length::Fill)
        .padding(Padding::from([8, 16]))
        .style(move |_, status| {
            let bg = if active {
                *theme::SURFACE_2
            } else {
                match status {
                    iced::widget::button::Status::Hovered => *theme::SURFACE_2,
                    _ => Color::TRANSPARENT,
                }
            };
            iced::widget::button::Style {
                background: Some(bg.into()),
                text_color: if active {
                    *theme::TERMINAL_GREEN
                } else {
                    *theme::FOREGROUND
                },
                border: Border::default(),
                shadow: iced::Shadow::default(),
                ..Default::default()
            }
        })
        .into()
}

fn right_pane<'a>(
    agents: &'a [AgentInfo],
    selected: &AgentId,
    log: Option<&'a VecDeque<ChatMessage>>,
    activity: Option<&'a ChatActivityState>,
    input: &'a str,
    connected: bool,
) -> Element<'a, Message> {
    let (display, subtitle) = agents
        .iter()
        .find(|a| a.id == selected.as_str())
        .map(|info| {
            let subtitle_parts: Vec<String> = [
                info.primary_model(),
                info.workspace.as_deref().and_then(|p| p.rsplit('/').next()),
            ]
            .iter()
            .filter_map(|o| o.map(|s| s.to_string()))
            .collect();
            (info.display_with_emoji(), subtitle_parts.join(" · "))
        })
        .unwrap_or_else(|| (selected.as_str().to_string(), String::new()));

    let title_col = column![
        text(display.clone()).size(16).color(*theme::FOREGROUND),
        if subtitle.is_empty() {
            text(format!("{} messages", log.map(VecDeque::len).unwrap_or(0)))
                .size(11)
                .color(*theme::MUTED)
        } else {
            text(subtitle).size(11).color(*theme::MUTED)
        },
    ]
    .spacing(2);

    // "Copy transcript" button — only rendered when there's
    // something to copy. Uses the shared `CopyToClipboard` message
    // so the app's existing clipboard plumbing handles the OS call.
    let copy_action: Option<Element<'a, Message>> = log.filter(|m| !m.is_empty()).map(|messages| {
        let slice: Vec<ChatMessage> = messages.iter().cloned().collect();
        let markdown = crate::transcript::to_markdown(&display, &slice);
        copy_button("Copy transcript", markdown)
    });

    let mut header = row![title_col, Space::new().width(Length::Fill)].align_y(Alignment::Center);
    if let Some(btn) = copy_action {
        header = header.push(btn);
    }

    let body: Element<'a, Message> = match log {
        Some(messages) if !messages.is_empty() => messages
            .iter()
            .fold(column![].spacing(8), |acc, msg| {
                acc.push(chat_bubble::view(msg))
            })
            .into(),
        _ => container(
            text("No messages yet. Type below to start.")
                .size(12)
                .color(*theme::MUTED),
        )
        .padding(Padding::from(24))
        .into(),
    };

    let scroll = scrollable(body)
        .anchor_bottom()
        .width(Length::Fill)
        .height(Length::Fill);

    let activity_row = activity.map(|state| activity_indicator(&display, state));

    let mut col = column![
        container(header)
            .width(Length::Fill)
            .padding(Padding::from([16, 24])),
        container(scroll)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding::from([0, 24])),
    ]
    .spacing(0);
    if let Some(row) = activity_row {
        col = col.push(row);
    }
    col.push(chat_input::view(input, connected, display.as_str()))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Compact status row rendered between the transcript and the input:
/// "● Sebastian is thinking…" / "… is using bash" / "… is sending…".
/// Muted so it reads as ambient state, not a message.
fn activity_indicator<'a>(
    agent_display: &str,
    state: &'a ChatActivityState,
) -> Element<'a, Message> {
    // Strip emoji from the display for a cleaner status sentence —
    // "Sebastian is thinking…" reads better than "Sebastian 🦀 is…".
    let speaker = agent_display
        .split_whitespace()
        .next()
        .unwrap_or(agent_display)
        .to_string();
    let label = match &state.kind {
        ChatActivity::Sending => format!("{speaker} is receiving your message…"),
        ChatActivity::Thinking => format!("{speaker} is thinking…"),
        ChatActivity::Tool(name) => format!("{speaker} is using {name}…"),
    };
    container(
        row![
            container(text(""))
                .width(Length::Fixed(6.0))
                .height(Length::Fixed(6.0))
                .style(|_| container::Style {
                    background: Some((*theme::TERMINAL_GREEN).into()),
                    border: Border {
                        color: *theme::TERMINAL_GREEN,
                        width: 0.0,
                        radius: 3.0.into(),
                    },
                    ..Default::default()
                }),
            text(label).size(11).color(*theme::MUTED),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([6, 24]))
    .into()
}

/// Small "Copy" button that dispatches the given text via the
/// shared `CopyToClipboard` message. Shared between the Chat tab's
/// header and the Sessions detail header via the `pub` export.
pub fn copy_button<'a>(label: &'a str, payload: String) -> Element<'a, Message> {
    button(text(label).size(11).color(*theme::FOREGROUND))
        .on_press(Message::CopyToClipboard(payload))
        .padding(Padding::from([4, 10]))
        .style(|_, status| {
            let bg = match status {
                iced::widget::button::Status::Hovered => Some((*theme::SURFACE_3).into()),
                _ => Some((*theme::SURFACE_2).into()),
            };
            iced::widget::button::Style {
                background: bg,
                text_color: *theme::FOREGROUND,
                border: Border {
                    color: *theme::BORDER,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                shadow: Shadow::default(),
                ..Default::default()
            }
        })
        .into()
}
