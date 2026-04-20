//! The "Sessions" nav tab — per-session token usage and freshness.
//!
//! The gateway names sessions as `agent:<agentId>:<sessionId>`. We
//! strip the `agent:` prefix on display since it's noise (every row
//! has it). Sort newest-first by `updatedAt`, then by key so tests and
//! empty-timestamp rows stay deterministic.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use iced::widget::{Space, column, container, row, scrollable, text};
use iced::{Alignment, Border, Element, Length, Padding};

use crate::Message;
use crate::net::rpc::SessionInfo;
use crate::ui::theme;

pub fn view<'a>(sessions: &'a HashMap<String, SessionInfo>) -> Element<'a, Message> {
    let mut entries: Vec<&SessionInfo> = sessions.values().collect();
    // Newest first; rows without a timestamp sink to the bottom but
    // stay key-ordered among themselves.
    entries.sort_by(|a, b| {
        b.updated_at_ms
            .cmp(&a.updated_at_ms)
            .then_with(|| a.key.cmp(&b.key))
    });

    let header = column![
        text("Sessions").size(20).color(*theme::FOREGROUND),
        text(format!("{} tracked", entries.len()))
            .size(12)
            .color(*theme::MUTED),
    ]
    .spacing(4);

    let body: Element<'a, Message> = if entries.is_empty() {
        text("no session data yet — waiting for sessions.list")
            .size(12)
            .color(*theme::MUTED)
            .into()
    } else {
        entries
            .into_iter()
            .fold(column![].spacing(10), |acc, info| {
                acc.push(session_card(info))
            })
            .into()
    };

    let outer = column![header, scrollable(body).height(Length::Fill)]
        .spacing(14)
        .padding(Padding::from(24));

    outer.into()
}

fn session_card(info: &SessionInfo) -> Element<'_, Message> {
    let title = display_key(&info.key);
    let model = info
        .model
        .as_deref()
        .filter(|m| !m.is_empty())
        .unwrap_or("—");

    let header = row![
        text(title).size(14).color(*theme::FOREGROUND),
        text(model).size(11).color(*theme::MUTED),
        Space::new().width(Length::Fill),
        freshness_badge(info.updated_at_ms),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    let tokens_line = tokens_summary(info);
    let io_line = io_summary(info);

    let mut details = column![text(tokens_line).size(11).color(*theme::MUTED)].spacing(3);
    if let Some(line) = io_line {
        details = details.push(text(line).size(11).color(*theme::MUTED));
    }
    if let Some(level) = info
        .thinking_level
        .as_deref()
        .filter(|s| !s.is_empty() && *s != "off")
    {
        details = details.push(
            text(format!("thinking: {level}"))
                .size(11)
                .color(*theme::MUTED),
        );
    }

    container(column![header, details].spacing(8))
        .width(Length::Fill)
        .padding(Padding::from([12, 14]))
        .style(|_| container::Style {
            background: Some((*theme::SURFACE_1).into()),
            border: Border {
                color: *theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn freshness_badge(updated_at_ms: Option<i64>) -> Element<'static, Message> {
    let (label, color) = match updated_at_ms {
        Some(ms) => (format_time_ago(ms), *theme::MUTED),
        None => ("—".to_string(), *theme::STATUS_UNKNOWN),
    };
    container(text(label).size(10).color(color))
        .padding(Padding::from([2, 6]))
        .style(move |_| container::Style {
            background: Some(iced::Color { a: 0.10, ..color }.into()),
            border: Border {
                color: iced::Color { a: 0.35, ..color },
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

fn io_summary(info: &SessionInfo) -> Option<String> {
    match (info.input_tokens, info.output_tokens) {
        (Some(inp), Some(out)) => Some(format!(
            "io: {} in · {} out",
            fmt_tokens(inp),
            fmt_tokens(out)
        )),
        _ => None,
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
