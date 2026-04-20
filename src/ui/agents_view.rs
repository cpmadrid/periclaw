//! The "Agents" nav tab — a scrollable card list that gives each
//! agent a full row of detail the Overview sprite can't surface:
//!
//! - crons: schedule-adjacent metadata (`nextRunAtMs`, `lastRunAtMs`,
//!   duration, last error) + "Run now" button
//! - channels: configured / connected flags + last error string
//! - main: active model + session metadata + "Reset session" button
//!
//! The Reset button uses a two-click arm pattern: the first click
//! turns the button red and relabels to "Confirm reset?"; a second
//! click within the confirmation window dispatches `sessions.reset`.
//! Auto-disarms if the operator walks away — arm state is pruned on
//! every Tick.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Border, Element, Length, Padding};

use crate::Message;
use crate::domain::{Agent, AgentId, AgentKind, AgentStatus};
use crate::net::rpc::{Channel, CronState, SessionInfo};
use crate::ui::theme;

pub struct AgentsViewSnapshot<'a> {
    pub roster: &'a [Agent],
    pub statuses: &'a HashMap<AgentId, AgentStatus>,
    pub cron_details: &'a HashMap<AgentId, CronState>,
    /// UUIDs keyed by AgentId for crons that came from the snapshot.
    /// Used to gate the "Run" button — if we don't know the id we
    /// can't fire `cron.run`, so the button stays hidden.
    pub cron_ids: &'a HashMap<AgentId, String>,
    pub channel_details: &'a HashMap<AgentId, Channel>,
    pub active_model: Option<&'a str>,
    pub sessions: &'a HashMap<String, SessionInfo>,
    /// Main agents whose Reset button is armed — the row renders
    /// the button in red "Confirm reset?" form instead of the neutral
    /// "Reset session". Absence means the neutral form is shown.
    pub pending_resets: &'a HashMap<AgentId, Instant>,
    /// Agent ids whose error row is expanded (full text shown
    /// instead of truncated). Toggled by `Message::ToggleAgentError`.
    pub expanded_errors: &'a std::collections::HashSet<AgentId>,
}

/// Detail lines split so the caller can render the `error` row
/// differently (expandable toggle, red color, wrap) from the
/// neutral schedule / session lines.
struct AgentDetail {
    lines: Vec<String>,
    error: Option<String>,
}

pub fn view<'a>(snap: AgentsViewSnapshot<'a>) -> Element<'a, Message> {
    let rows = snap
        .roster
        .iter()
        .map(|agent| agent_row(agent, &snap))
        .fold(column![].spacing(10), |acc, el| acc.push(el));

    let header = text("Agents").size(20).color(*theme::FOREGROUND);

    let body = column![
        header,
        text(format!("{} tracked", snap.roster.len()))
            .size(12)
            .color(*theme::MUTED),
        rows,
    ]
    .spacing(14)
    .padding(Padding::from(24));

    scrollable(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn agent_row<'a>(agent: &'a Agent, snap: &AgentsViewSnapshot<'a>) -> Element<'a, Message> {
    let status = snap
        .statuses
        .get(&agent.id)
        .copied()
        .unwrap_or(AgentStatus::Unknown);
    let (badge_label, badge_color) = status_badge(status);

    // Colored sprite dot
    let dot = container(text(""))
        .width(Length::Fixed(10.0))
        .height(Length::Fixed(10.0))
        .style(move |_| container::Style {
            background: Some(agent.color().into()),
            border: Border {
                color: agent.color(),
                width: 0.0,
                radius: 5.0.into(),
            },
            ..Default::default()
        });

    // Status badge
    let badge = container(text(badge_label).size(10).color(badge_color))
        .padding(Padding::from([2, 6]))
        .style(move |_| container::Style {
            background: Some(
                iced::Color {
                    a: 0.15,
                    ..badge_color
                }
                .into(),
            ),
            border: Border {
                color: iced::Color {
                    a: 0.45,
                    ..badge_color
                },
                width: 1.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        });

    // Per-kind action button — rendered in the header's right gutter
    // before the status badge. Keeps a single affordance per row so
    // the card stays scannable.
    let action_button: Option<Element<'a, Message>> = match agent.kind {
        AgentKind::Cron if snap.cron_ids.contains_key(&agent.id) => Some(
            button(text("Run now").size(11))
                .padding(Padding::from([4, 10]))
                .on_press(Message::RunCron(agent.id.clone()))
                .into(),
        ),
        AgentKind::Main => Some(reset_session_button(
            agent.id.clone(),
            snap.pending_resets.contains_key(&agent.id),
        )),
        _ => None,
    };

    let mut header = row![
        dot,
        text(agent.display.as_str())
            .size(14)
            .color(*theme::FOREGROUND),
        text(format!("{:?}", agent.kind).to_lowercase())
            .size(11)
            .color(*theme::MUTED),
        Space::new().width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    if let Some(btn) = action_button {
        header = header.push(btn);
    }
    header = header.push(badge);

    let detail = match agent.kind {
        AgentKind::Cron => cron_detail(&agent.id, snap),
        AgentKind::Channel => channel_detail(&agent.id, snap),
        AgentKind::Main => main_detail(snap),
    };

    let mut detail_col = detail
        .lines
        .into_iter()
        .fold(column![].spacing(3), |acc, line| {
            acc.push(text(line).size(11).color(*theme::MUTED))
        });
    if let Some(err) = detail.error {
        let expanded = snap.expanded_errors.contains(&agent.id);
        detail_col = detail_col.push(error_row(agent.id.clone(), err, expanded));
    }

    container(column![header, detail_col].spacing(8))
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

fn cron_detail(id: &AgentId, snap: &AgentsViewSnapshot<'_>) -> AgentDetail {
    let Some(state) = snap.cron_details.get(id) else {
        return AgentDetail {
            lines: vec!["no state yet".into()],
            error: None,
        };
    };
    let mut lines = Vec::new();
    if let Some(last_status) = state.last_status.as_deref() {
        let dur = state
            .last_duration_ms
            .map(format_duration_ms)
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        let ago = state
            .last_run_at_ms
            .map(format_time_ago)
            .map(|s| format!(" {s}"))
            .unwrap_or_default();
        lines.push(format!("last: {last_status}{dur}{ago}"));
    }
    if let Some(next_ms) = state.next_run_at_ms {
        lines.push(format!("next: {}", format_time_until(next_ms)));
    }
    if lines.is_empty() && state.last_error.is_none() {
        lines.push("no recent runs".into());
    }
    AgentDetail {
        lines,
        error: state.last_error.clone(),
    }
}

fn channel_detail(id: &AgentId, snap: &AgentsViewSnapshot<'_>) -> AgentDetail {
    let Some(ch) = snap.channel_details.get(id) else {
        return AgentDetail {
            lines: vec!["no state yet".into()],
            error: None,
        };
    };
    let lines = vec![format!(
        "configured: {} · connected: {}",
        yes_no(ch.enabled),
        yes_no(ch.connected),
    )];
    AgentDetail {
        lines,
        error: ch.last_error.clone(),
    }
}

fn main_detail<'a>(snap: &AgentsViewSnapshot<'a>) -> AgentDetail {
    let mut lines = Vec::new();
    if let Some(model) = snap.active_model {
        lines.push(format!("model: {model}"));
    }
    if let Some(info) = snap.sessions.get("agent:main:main")
        && let (Some(total), Some(ctx)) = (info.total_tokens, info.context_tokens)
    {
        lines.push(format!("main session: {total} / {ctx} tokens"));
    }
    if lines.is_empty() {
        lines.push("no session data yet".into());
    }
    AgentDetail { lines, error: None }
}

/// Expandable error row — truncated one-liner by default, click the
/// chevron (▸/▾) to swap in the full text. Uses the app's
/// `expanded_errors` set as the source of truth; toggle dispatches
/// a Message that updates the set and re-renders.
fn error_row(agent_id: AgentId, err: String, expanded: bool) -> Element<'static, Message> {
    let indicator = if expanded { "▾" } else { "▸" };
    let body = if expanded {
        err.clone()
    } else {
        truncate(&err, 100)
    };
    button(
        row![
            text(indicator).size(10).color(*theme::STATUS_DOWN),
            text(format!("error: {body}"))
                .size(11)
                .color(*theme::STATUS_DOWN),
        ]
        .spacing(6)
        .align_y(Alignment::Start),
    )
    .on_press(Message::ToggleAgentError(agent_id))
    .width(Length::Fill)
    .padding(Padding::from([0, 0]))
    .style(|_, status| {
        let bg = matches!(status, iced::widget::button::Status::Hovered).then(|| {
            iced::Color {
                a: 0.12,
                ..*theme::STATUS_DOWN
            }
            .into()
        });
        iced::widget::button::Style {
            background: bg,
            text_color: *theme::STATUS_DOWN,
            border: Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 3.0.into(),
            },
            shadow: iced::Shadow::default(),
            ..Default::default()
        }
    })
    .into()
}

/// "Reset session" button with two-click confirmation. The armed
/// state is driven by the app's `pending_resets` map — flipping
/// between states is handled by the `ResetMainSession` handler in
/// `app.rs`, not by local widget state.
fn reset_session_button(agent_id: AgentId, armed: bool) -> Element<'static, Message> {
    let (label, fg, bg, border) = if armed {
        (
            "Confirm reset?",
            *theme::STATUS_DOWN,
            iced::Color {
                a: 0.18,
                ..*theme::STATUS_DOWN
            },
            *theme::STATUS_DOWN,
        )
    } else {
        (
            "Reset session",
            *theme::FOREGROUND,
            *theme::SURFACE_2,
            *theme::BORDER,
        )
    };
    button(text(label).size(11).color(fg))
        .padding(Padding::from([4, 10]))
        .style(move |_, _| button::Style {
            background: Some(bg.into()),
            text_color: fg,
            border: Border {
                color: border,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        })
        .on_press(Message::ResetMainSession(agent_id))
        .into()
}

fn status_badge(status: AgentStatus) -> (&'static str, iced::Color) {
    match status {
        AgentStatus::Running => ("RUNNING", *theme::TERMINAL_GREEN),
        AgentStatus::Ok => ("OK", *theme::STATUS_UP),
        AgentStatus::Error => ("ERROR", *theme::STATUS_DOWN),
        AgentStatus::Disabled => ("OFF", *theme::MUTED),
        AgentStatus::Unknown => ("?", *theme::STATUS_UNKNOWN),
    }
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn format_duration_ms(ms: i64) -> String {
    if ms < 0 {
        return "?".into();
    }
    let secs = ms as u64 / 1000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Show a Unix-ms timestamp as `Ns ago` / `Nm ago` / `Nh ago` /
/// `Nd ago` / `never` (clock-skew or missing).
fn format_time_ago(ms: i64) -> String {
    let Some(d) = duration_from_now_to(ms) else {
        return "never".into();
    };
    // Past: d is positive; future handled by format_time_until.
    if d.as_secs() < 60 {
        format!("({}s ago)", d.as_secs())
    } else if d.as_secs() < 3600 {
        format!("({}m ago)", d.as_secs() / 60)
    } else if d.as_secs() < 86400 {
        format!("({}h ago)", d.as_secs() / 3600)
    } else {
        format!("({}d ago)", d.as_secs() / 86400)
    }
}

fn format_time_until(ms: i64) -> String {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let delta = ms - now_ms;
    if delta <= 0 {
        return "overdue".into();
    }
    let secs = (delta / 1000) as u64;
    if secs < 60 {
        format!("in {secs}s")
    } else if secs < 3600 {
        format!("in {}m", secs / 60)
    } else if secs < 86400 {
        format!("in {}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("in {}d", secs / 86400)
    }
}

fn duration_from_now_to(past_ms: i64) -> Option<Duration> {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    let delta = now_ms - past_ms;
    (delta >= 0).then(|| Duration::from_millis(delta as u64))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars()
        .take(max.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_buckets() {
        assert_eq!(format_duration_ms(500), "0s");
        assert_eq!(format_duration_ms(45_000), "45s");
        assert_eq!(format_duration_ms(95_000), "1m 35s");
        assert_eq!(format_duration_ms(3_700_000), "1h 1m");
    }

    #[test]
    fn truncate_adds_ellipsis() {
        assert_eq!(truncate("hello", 10), "hello");
        let out = truncate(&"x".repeat(120), 10);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('…'));
    }
}
