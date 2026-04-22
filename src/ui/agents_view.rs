//! The "Agents" nav tab — a scrollable card list that gives each
//! agent and job a full row of detail the Overview sprite can't surface:
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
use crate::domain::job::JobKind;
use crate::domain::{Agent, AgentId, AgentStatus, Job, JobId};
use crate::net::rpc::{Channel, CronState, SessionInfo};
use crate::ui::theme;
use crate::ui::widgets::{card_style, colored_dot, status_pill};

pub struct AgentsViewSnapshot<'a> {
    pub roster: &'a [Agent],
    pub jobs: &'a HashMap<JobId, Job>,
    pub statuses: &'a HashMap<AgentId, AgentStatus>,
    pub cron_details: &'a HashMap<AgentId, CronState>,
    /// UUIDs keyed by AgentId for crons that came from the snapshot.
    pub cron_ids: &'a HashMap<AgentId, String>,
    pub channel_details: &'a HashMap<AgentId, Channel>,
    pub active_model: Option<&'a str>,
    pub sessions: &'a HashMap<String, SessionInfo>,
    pub pending_resets: &'a HashMap<AgentId, Instant>,
    pub expanded_errors: &'a std::collections::HashSet<AgentId>,
}

struct AgentDetail {
    lines: Vec<String>,
    error: Option<String>,
}

pub fn view<'a>(snap: AgentsViewSnapshot<'a>) -> Element<'a, Message> {
    let agent_rows = snap
        .roster
        .iter()
        .map(|agent| agent_row(agent, &snap))
        .fold(column![].spacing(10), |acc, el| acc.push(el));

    let mut jobs_sorted: Vec<&Job> = snap.jobs.values().collect();
    jobs_sorted.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    let job_rows = jobs_sorted
        .into_iter()
        .map(|job| job_row(job, &snap))
        .fold(column![].spacing(10), |acc, el| acc.push(el));

    let header = text("Agents").size(20).color(*theme::FOREGROUND);
    let tracked = snap.roster.len() + snap.jobs.len();

    let body = column![
        header,
        text(format!("{tracked} tracked"))
            .size(12)
            .color(*theme::MUTED),
        agent_rows,
        job_rows,
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

    let dot = colored_dot(agent.color());
    let badge = status_pill(status);
    let action: Option<Element<'a, Message>> = Some(reset_session_button(
        agent.id.clone(),
        snap.pending_resets.contains_key(&agent.id),
    ));

    let kind_label = "agent";
    let mut header = row![
        dot,
        text(agent.display.as_str())
            .size(14)
            .color(*theme::FOREGROUND),
        text(kind_label).size(11).color(*theme::MUTED),
        Space::new().width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    if let Some(btn) = action {
        header = header.push(btn);
    }
    header = header.push(badge);

    let detail = main_detail(snap);

    card(&agent.id, header, detail, snap.expanded_errors)
}

fn job_row<'a>(job: &'a Job, snap: &AgentsViewSnapshot<'a>) -> Element<'a, Message> {
    let status = job.status;
    let dot = colored_dot(job.color());
    let badge = status_pill(status);

    let agent_id = AgentId::new(job.id.as_str());
    let action: Option<Element<'a, Message>> = match job.kind {
        JobKind::Cron if snap.cron_ids.contains_key(&agent_id) => Some(
            button(text("Run now").size(11))
                .padding(Padding::from([4, 10]))
                .on_press(Message::RunCron(agent_id.clone()))
                .into(),
        ),
        _ => None,
    };

    let kind_label = match job.kind {
        JobKind::Cron => "cron",
        JobKind::Channel => "channel",
    };
    let mut header = row![
        dot,
        text(job.display.as_str())
            .size(14)
            .color(*theme::FOREGROUND),
        text(kind_label).size(11).color(*theme::MUTED),
        Space::new().width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    if let Some(btn) = action {
        header = header.push(btn);
    }
    header = header.push(badge);

    let detail = match job.kind {
        JobKind::Cron => cron_detail(&agent_id, snap),
        JobKind::Channel => channel_detail(&agent_id, snap),
    };

    card(&agent_id, header, detail, snap.expanded_errors)
}

fn card<'a>(
    row_id: &AgentId,
    header: iced::widget::Row<'a, Message>,
    detail: AgentDetail,
    expanded_errors: &std::collections::HashSet<AgentId>,
) -> Element<'a, Message> {
    let mut detail_col = detail
        .lines
        .into_iter()
        .fold(column![].spacing(3), |acc, line| {
            acc.push(text(line).size(11).color(*theme::MUTED))
        });
    if let Some(err) = detail.error {
        let expanded = expanded_errors.contains(row_id);
        detail_col = detail_col.push(error_row(row_id.clone(), err, expanded));
    }

    container(column![header, detail_col].spacing(8))
        .width(Length::Fill)
        .padding(Padding::from([12, 14]))
        .style(card_style(6.0))
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

fn format_time_ago(ms: i64) -> String {
    let Some(d) = duration_from_now_to(ms) else {
        return "never".into();
    };
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
