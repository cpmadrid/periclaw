//! The "Logs" nav tab — a scrollable tail of the gateway's rolling
//! log file. Lines stream in via a periodic `logs.tail` RPC (see
//! `net::openclaw::LOG_TAIL_INTERVAL`) and land in
//! `App::log_lines`, a bounded ring buffer.
//!
//! Minimal v1:
//! - monospace text, newest at the bottom
//! - empty-state placeholder until the first response lands
//! - no manual filtering / severity coloring yet (single crate so
//!   `tracing`'s own formatting carries the INFO/WARN/ERROR cues)

use iced::widget::{column, container, scrollable, text};
use iced::{Border, Element, Length, Padding};

use crate::Message;
use crate::ui::theme;

pub fn view<'a>(lines: impl IntoIterator<Item = &'a String>) -> Element<'a, Message> {
    let rendered: Vec<Element<'a, Message>> = lines
        .into_iter()
        .map(|line| {
            text(line.as_str())
                .size(11)
                .font(iced::Font::MONOSPACE)
                .color(*theme::FOREGROUND)
                .into()
        })
        .collect();

    let body: Element<'a, Message> = if rendered.is_empty() {
        text("waiting for gateway log lines…")
            .size(12)
            .color(*theme::MUTED)
            .into()
    } else {
        rendered
            .into_iter()
            .fold(column![].spacing(0), |acc, row| acc.push(row))
            .into()
    };

    let header = column![
        text("Logs").size(20).color(*theme::FOREGROUND),
        text("live tail of ~/.openclaw/logs/openclaw-<date>.log · 3s refresh")
            .size(11)
            .color(*theme::MUTED),
    ]
    .spacing(4);

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

    let outer = column![header, scrollable(framed).height(Length::Fill)]
        .spacing(12)
        .padding(Padding::from(24));

    outer.into()
}
