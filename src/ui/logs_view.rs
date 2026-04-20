//! The "Logs" nav tab — a scrollable tail of the gateway's rolling
//! log file with severity coloring, level filter chips, substring
//! search, and pause-on-scroll auto-tail behavior.
//!
//! Lines stream in via a periodic `logs.tail` RPC (see
//! `net::openclaw::LOG_TAIL_INTERVAL`), are classified by severity
//! at ingest time (see `logs::LogLine`), and land in
//! `App::log_lines` — a bounded ring buffer.
//!
//! The scrollable is `anchor_bottom`'d so new lines push the view
//! forward automatically. When the operator scrolls up, a "Jump to
//! latest" pill surfaces; clicking it snaps back to the bottom and
//! resumes auto-tail.

use std::sync::LazyLock;

use iced::widget::{button, column, container, row, scrollable, stack, text, text_input};
use iced::{Alignment, Border, Element, Length, Padding};

use crate::Message;
use crate::logs::{LogFilters, LogLine, LogSeverity};
use crate::ui::theme;

/// Scrollable id used by `Message::LogsJumpToLatest` to target the
/// right widget from `snap_to_end`. Stable across renders.
static SCROLL_ID: LazyLock<iced::widget::Id> =
    LazyLock::new(|| iced::widget::Id::new("logs-scroll"));

pub fn scroll_id() -> iced::widget::Id {
    SCROLL_ID.clone()
}

pub fn view<'a>(
    lines: impl IntoIterator<Item = &'a LogLine>,
    filters: &'a LogFilters,
    auto_tail: bool,
) -> Element<'a, Message> {
    let header = column![
        text("Logs").size(20).color(*theme::FOREGROUND),
        text("live tail of ~/.openclaw/logs/openclaw-<date>.log · 3s refresh")
            .size(11)
            .color(*theme::MUTED),
    ]
    .spacing(4);

    let chips = row![
        severity_chip(LogSeverity::Error, filters.show_error),
        severity_chip(LogSeverity::Warn, filters.show_warn),
        severity_chip(LogSeverity::Info, filters.show_info),
        severity_chip(LogSeverity::Debug, filters.show_debug),
    ]
    .spacing(6);

    let search = text_input("filter (case-insensitive substring)", &filters.search)
        .on_input(Message::LogsSearchChanged)
        .size(12)
        .padding(Padding::from([4, 8]))
        .width(Length::Fill);

    let filters_bar = row![
        container(chips).width(Length::Shrink),
        container(search).width(Length::Fill),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let mut visible_rows: Vec<Element<'a, Message>> = Vec::new();
    let mut visible_count = 0usize;
    let mut total_count = 0usize;
    for line in lines {
        total_count += 1;
        if filters.matches(line) {
            visible_count += 1;
            visible_rows.push(render_line(line));
        }
    }

    let body: Element<'a, Message> = if visible_rows.is_empty() {
        let msg = if total_count == 0 {
            "waiting for gateway log lines…".to_string()
        } else {
            format!(
                "no matches ({total_count} line{} hidden by filters)",
                if total_count == 1 { "" } else { "s" },
            )
        };
        text(msg).size(12).color(*theme::MUTED).into()
    } else {
        visible_rows
            .into_iter()
            .fold(column![].spacing(0), |acc, row| acc.push(row))
            .into()
    };

    let framed = container(body)
        .width(Length::Fill)
        .padding(Padding::from([10, 12]))
        .style(|_| container::Style {
            background: Some((*theme::SURFACE_1).into()),
            border: Border {
                color: *theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        });

    // `anchor_bottom` keeps the scrollable pinned to the tail when
    // content grows — new lines push into view without a Task hop.
    // `on_scroll` still fires when the operator drags the scrollbar,
    // letting us flip `auto_tail` off and surface the pill.
    let log_scroll = scrollable(framed)
        .id(scroll_id())
        .height(Length::Fill)
        .anchor_bottom()
        .on_scroll(Message::LogsScrolled);

    // Pill only shows when the operator has scrolled up — it floats
    // above the scrollable via a stack, anchored bottom-right.
    let hit_count_line = text(format!("showing {visible_count} of {total_count}",))
        .size(10)
        .color(*theme::MUTED);

    let scroll_region: Element<'a, Message> = if auto_tail {
        log_scroll.into()
    } else {
        stack![
            log_scroll,
            container(jump_to_latest_pill())
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::End)
                .align_y(Alignment::End)
                .padding(Padding::from(12)),
        ]
        .into()
    };

    column![header, filters_bar, hit_count_line, scroll_region]
        .spacing(10)
        .padding(Padding::from(24))
        .into()
}

fn severity_chip(severity: LogSeverity, active: bool) -> Element<'static, Message> {
    let label = severity.label();
    let (fg, bg, border) = if active {
        let (fg, glow) = severity_colors(severity);
        (fg, glow, fg)
    } else {
        (*theme::MUTED, *theme::SURFACE_1, *theme::BORDER)
    };
    button(text(label).size(10).font(iced::Font::MONOSPACE).color(fg))
        .padding(Padding::from([3, 8]))
        .style(move |_, _| button::Style {
            background: Some(bg.into()),
            text_color: fg,
            border: Border {
                color: border,
                width: 1.0,
                radius: 12.0.into(),
            },
            ..Default::default()
        })
        .on_press(Message::LogsToggleSeverity(severity))
        .into()
}

fn jump_to_latest_pill() -> Element<'static, Message> {
    button(text("↓ jump to latest").size(11).color(*theme::FOREGROUND))
        .padding(Padding::from([6, 12]))
        .style(|_, _| button::Style {
            background: Some((*theme::SURFACE_2).into()),
            text_color: *theme::FOREGROUND,
            border: Border {
                color: *theme::TERMINAL_GREEN,
                width: 1.0,
                radius: 16.0.into(),
            },
            ..Default::default()
        })
        .on_press(Message::LogsJumpToLatest)
        .into()
}

fn render_line(line: &LogLine) -> Element<'static, Message> {
    let (fg, _) = severity_colors(line.severity);
    text(strip_ansi(&line.text))
        .size(11)
        .font(iced::Font::MONOSPACE)
        .color(fg)
        .into()
}

/// Strip ANSI color escape sequences from a log line before
/// rendering — `tracing` emits them when stdout is a TTY; if the
/// gateway's captured log was produced that way, we'd otherwise
/// render `[2m` etc. as literal text.
fn strip_ansi(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // CSI introducer — skip until the final alpha terminator.
            if chars.next() == Some('[') {
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Map a severity to its (foreground, glow) color pair. Pulled from
/// the shared theme palette so it matches the Overview status dots
/// and the status-bar indicators.
fn severity_colors(severity: LogSeverity) -> (iced::Color, iced::Color) {
    match severity {
        LogSeverity::Error => (*theme::STATUS_DOWN, subtle_bg(*theme::STATUS_DOWN)),
        LogSeverity::Warn => (*theme::STATUS_DEGRADED, subtle_bg(*theme::STATUS_DEGRADED)),
        LogSeverity::Info => (*theme::FOREGROUND, *theme::SURFACE_1),
        LogSeverity::Debug => (*theme::MUTED, *theme::SURFACE_1),
    }
}

fn subtle_bg(c: iced::Color) -> iced::Color {
    iced::Color { a: 0.12, ..c }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_csi_sequences() {
        // Representative tracing line: timestamp, level token wrapped
        // in ANSI color codes, module, then body.
        let raw = "\u{1b}[2m2026-04-20T00:53:42Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m message";
        assert_eq!(strip_ansi(raw), "2026-04-20T00:53:42Z  INFO message",);
    }

    #[test]
    fn strip_ansi_passes_through_plain_text() {
        let raw = "no escapes here";
        assert_eq!(strip_ansi(raw), raw);
    }
}
