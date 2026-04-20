//! The "Sessions" nav tab — a two-pane layout:
//!
//! - **Left pane**: scrollable list of sessions with per-row token
//!   summary + freshness badge. The currently-selected session is
//!   highlighted. Clicking a row fires `Message::SessionSelected`.
//! - **Right pane**: drill-in detail for the selected session —
//!   header (model / thinking level), transcript via `chat_bubble`,
//!   and any token metadata not shown in the list. The detail pane
//!   is fetched lazily via `chat.history` the first time an entry
//!   is opened per connection.
//!
//! The gateway names sessions as `agent:<agentId>:<sessionId>`. We
//! strip the `agent:` prefix on display since it's noise (every row
//! has it). Sort newest-first by `updatedAt`, then by key so tests
//! and empty-timestamp rows stay deterministic.

use std::collections::{HashMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use iced::widget::{Space, button, canvas, column, container, row, scrollable, text};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use crate::Message;
use crate::net::rpc::{SessionInfo, SessionUsagePoint};
use crate::ui::chat_view::{self, ChatMessage};
use crate::ui::sparkline::{self, TokenSparkline};
use crate::ui::{chat_bubble, theme};

pub struct SessionsViewSnapshot<'a> {
    pub sessions: &'a HashMap<String, SessionInfo>,
    pub active_session_key: Option<&'a str>,
    pub transcripts: &'a HashMap<String, VecDeque<ChatMessage>>,
    /// Session keys we've asked the gateway about since the last
    /// connect. Used to tell "empty transcript" (hydrated, actually
    /// empty) apart from "not yet fetched" (show a loading label).
    pub hydrated: &'a std::collections::HashSet<String>,
    /// Per-session usage timeseries. Absence = "not fetched yet";
    /// empty Vec = "fetched, session has no recorded points".
    pub usage: &'a HashMap<String, Vec<SessionUsagePoint>>,
    pub sparkline_cache: &'a iced::widget::canvas::Cache,
    pub connected: bool,
}

pub fn view<'a>(snap: SessionsViewSnapshot<'a>) -> Element<'a, Message> {
    let mut entries: Vec<&SessionInfo> = snap.sessions.values().collect();
    // Newest first; rows without a timestamp sink to the bottom but
    // stay key-ordered among themselves.
    entries.sort_by(|a, b| {
        b.updated_at_ms
            .cmp(&a.updated_at_ms)
            .then_with(|| a.key.cmp(&b.key))
    });

    let list = list_pane(&entries, snap.active_session_key);
    let detail = detail_pane(&entries, &snap);

    row![list, detail]
        .spacing(0)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn list_pane<'a>(entries: &[&'a SessionInfo], active_key: Option<&'a str>) -> Element<'a, Message> {
    let header = column![
        text("Sessions").size(16).color(*theme::FOREGROUND),
        text(format!("{} tracked", entries.len()))
            .size(11)
            .color(*theme::MUTED),
    ]
    .spacing(2);

    let body: Element<'a, Message> = if entries.is_empty() {
        text("no session data yet — waiting for sessions.list")
            .size(12)
            .color(*theme::MUTED)
            .into()
    } else {
        entries
            .iter()
            .fold(column![].spacing(6), |acc, info| {
                acc.push(session_card(info, Some(info.key.as_str()) == active_key))
            })
            .into()
    };

    let outer = column![
        container(header).padding(Padding::from([14, 16])),
        scrollable(container(body).padding(Padding::from([0, 12]))).height(Length::Fill),
    ]
    .spacing(0);

    container(outer)
        .width(Length::Fixed(320.0))
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

fn detail_pane<'a>(
    entries: &[&'a SessionInfo],
    snap: &SessionsViewSnapshot<'a>,
) -> Element<'a, Message> {
    let Some(key) = snap.active_session_key else {
        return placeholder("Select a session to see its transcript and usage.");
    };
    let info = entries.iter().find(|i| i.key == key);

    let title_el: Element<'a, Message> = match info {
        Some(info) => detail_header(info),
        None => text(format!("session {key} not in list (waiting?)"))
            .size(12)
            .color(*theme::MUTED)
            .into(),
    };

    let transcript = snap.transcripts.get(key);
    let hydrated = snap.hydrated.contains(key);

    // Copy button goes next to the title when there's a transcript
    // to copy. Uses the same helper as the Chat tab for a single
    // source of truth on styling and dispatch.
    let header: Element<'a, Message> = match transcript {
        Some(messages) if !messages.is_empty() => {
            let title_text = display_key(key).to_string();
            let slice: Vec<ChatMessage> = messages.iter().cloned().collect();
            let markdown = crate::transcript::to_markdown(&title_text, &slice);
            row![
                title_el,
                Space::new().width(Length::Fill),
                chat_view::copy_button("Copy transcript", markdown),
            ]
            .align_y(Alignment::Center)
            .into()
        }
        _ => title_el,
    };

    let body: Element<'a, Message> = match transcript {
        Some(messages) if !messages.is_empty() => messages
            .iter()
            .fold(column![].spacing(8), |acc, msg| {
                acc.push(chat_bubble::view(msg))
            })
            .into(),
        Some(_) if hydrated => placeholder_text("session is empty"),
        _ if !snap.connected => placeholder_text("disconnected — reconnect to load transcript"),
        _ => placeholder_text("loading transcript…"),
    };

    // Sparkline sits between header and transcript — always
    // occupies the same vertical slot so the transcript doesn't
    // reflow when data arrives; renders a placeholder when the
    // series isn't known yet.
    let spark_points = snap.usage.get(key).map(Vec::as_slice).unwrap_or(&[]);
    let context_budget = info.and_then(|i| i.context_tokens);
    let sparkline_widget = canvas(TokenSparkline {
        points: spark_points,
        context_budget,
        cache: snap.sparkline_cache,
    })
    .width(Length::Fill)
    .height(Length::Fixed(sparkline::MIN_HEIGHT));

    let outer = column![
        container(header)
            .width(Length::Fill)
            .padding(Padding::from([16, 24])),
        container(sparkline_widget)
            .width(Length::Fill)
            .padding(Padding::from([0, 24])),
        container(scrollable(body).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding::from([8, 24])),
    ]
    .spacing(0);

    container(outer)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn detail_header<'a>(info: &'a SessionInfo) -> Element<'a, Message> {
    let title = display_key(&info.key);
    let subtitle_parts: Vec<String> = [
        info.model
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        info.thinking_level
            .as_deref()
            .filter(|s| !s.is_empty() && *s != "off")
            .map(|s| format!("thinking: {s}")),
        Some(tokens_summary(info)),
        info.updated_at_ms
            .map(|ms| format!("updated {}", format_time_ago(ms))),
    ]
    .into_iter()
    .flatten()
    .collect();

    column![
        text(title.to_string()).size(16).color(*theme::FOREGROUND),
        text(subtitle_parts.join(" · "))
            .size(11)
            .color(*theme::MUTED),
    ]
    .spacing(3)
    .into()
}

fn session_card(info: &SessionInfo, active: bool) -> Element<'_, Message> {
    let title = display_key(&info.key);
    let model = info
        .model
        .as_deref()
        .filter(|m| !m.is_empty())
        .unwrap_or("—");

    let header = row![
        text(title).size(13).color(*theme::FOREGROUND),
        Space::new().width(Length::Fill),
        freshness_badge(info.updated_at_ms),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    let details = column![
        text(model).size(11).color(*theme::MUTED),
        text(tokens_summary(info)).size(11).color(*theme::MUTED),
    ]
    .spacing(2);

    let card_bg = if active {
        *theme::SURFACE_3
    } else {
        *theme::SURFACE_2
    };
    let border_color = if active {
        *theme::TERMINAL_GREEN
    } else {
        *theme::BORDER
    };

    let session_key = info.key.clone();
    button(column![header, details].spacing(6))
        .on_press(Message::SessionSelected(session_key))
        .width(Length::Fill)
        .padding(Padding::from([10, 12]))
        .style(move |_, _| button::Style {
            background: Some(card_bg.into()),
            text_color: *theme::FOREGROUND,
            border: Border {
                color: border_color,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn placeholder(msg: &'static str) -> Element<'static, Message> {
    container(text(msg).size(12).color(*theme::MUTED))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .padding(Padding::from(24))
        .into()
}

fn placeholder_text(msg: &'static str) -> Element<'static, Message> {
    text(msg).size(12).color(*theme::MUTED).into()
}

fn freshness_badge(updated_at_ms: Option<i64>) -> Element<'static, Message> {
    let (label, color) = match updated_at_ms {
        Some(ms) => (format_time_ago(ms), *theme::MUTED),
        None => ("—".to_string(), *theme::STATUS_UNKNOWN),
    };
    container(text(label).size(10).color(color))
        .padding(Padding::from([2, 6]))
        .style(move |_| container::Style {
            background: Some(Color { a: 0.10, ..color }.into()),
            border: Border {
                color: Color { a: 0.35, ..color },
                width: 1.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn tokens_summary(info: &SessionInfo) -> String {
    match (info.total_tokens, info.context_tokens) {
        (Some(total), Some(ctx)) => {
            let pct = if ctx > 0 {
                (total as f64 / ctx as f64 * 100.0).clamp(0.0, 999.0)
            } else {
                0.0
            };
            format!(
                "tokens: {} / {} ({:.0}%)",
                fmt_tokens(total),
                fmt_tokens(ctx),
                pct
            )
        }
        (Some(total), None) => format!("tokens: {}", fmt_tokens(total)),
        (None, Some(ctx)) => format!("context: {}", fmt_tokens(ctx)),
        (None, None) => "tokens: —".into(),
    }
}

/// `agent:main:main` → `main:main`. Leave other shapes untouched so an
/// unexpected key still renders (better than silently hiding it).
fn display_key(key: &str) -> &str {
    key.strip_prefix("agent:").unwrap_or(key)
}

fn fmt_tokens(n: i64) -> String {
    if n.abs() >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n.abs() >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn format_time_ago(ms: i64) -> String {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let delta = now_ms - ms;
    if delta < 0 {
        return "now".into();
    }
    let secs = (delta / 1000) as u64;
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_key_strips_agent_prefix() {
        assert_eq!(display_key("agent:main:main"), "main:main");
        assert_eq!(display_key("foo:bar"), "foo:bar");
    }

    #[test]
    fn fmt_tokens_scales() {
        assert_eq!(fmt_tokens(500), "500");
        assert_eq!(fmt_tokens(12_400), "12.4k");
        assert_eq!(fmt_tokens(2_300_000), "2.3M");
    }

    #[test]
    fn tokens_summary_shapes() {
        let mut info = SessionInfo {
            key: "agent:main:main".into(),
            total_tokens: Some(50_000),
            context_tokens: Some(200_000),
            input_tokens: None,
            output_tokens: None,
            updated_at_ms: None,
            age_ms: None,
            model: None,
            kind: None,
            thinking_level: None,
            agent_id: None,
        };
        let s = tokens_summary(&info);
        assert!(s.contains("50.0k"), "{}", s);
        assert!(s.contains("200.0k"), "{}", s);
        assert!(s.contains("25%"), "{}", s);

        info.total_tokens = None;
        info.context_tokens = None;
        assert_eq!(tokens_summary(&info), "tokens: —");
    }
}
