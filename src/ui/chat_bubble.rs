//! Shared bubble-row rendering for chat transcripts.
//!
//! Used by both the Chat tab's live conversation pane and the
//! Sessions tab's drill-in detail pane. Keeping the bubble shape in
//! one place means visual tweaks (padding, accent colors, alignment)
//! stay consistent across both surfaces.

use iced::widget::{Space, column, container, row, text};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use crate::Message;
use crate::ui::chat_view::{ChatMessage, ChatRole};
use crate::ui::theme;

/// Render a single transcript row — role label + body text inside a
/// padded container, right-aligned for user messages, left-aligned
/// for everything else. Returns a static-lifetime `Element` so it
/// can be composed into either the Chat or Sessions layouts.
pub fn view(msg: &ChatMessage) -> Element<'_, Message> {
    let (label, accent) = match msg.role {
        ChatRole::User => ("you", *theme::MUTED),
        ChatRole::Assistant => ("agent", *theme::TERMINAL_GREEN),
        ChatRole::Other => ("system", *theme::STATUS_DEGRADED),
    };

    let bubble = container(
        column![
            text(label).size(10).color(accent),
            text(msg.text.as_str()).size(13).color(*theme::FOREGROUND),
        ]
        .spacing(3),
    )
    .padding(Padding::from([8, 12]))
    .max_width(720.0)
    .style(move |_| container::Style {
        background: Some((*theme::SURFACE_1).into()),
        border: Border {
            color: Color { a: 0.35, ..accent },
            width: 1.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    });

    match msg.role {
        ChatRole::User => row![Space::new().width(Length::Fill), bubble],
        _ => row![bubble, Space::new().width(Length::Fill)],
    }
    .align_y(Alignment::Start)
    .width(Length::Fill)
    .into()
}
