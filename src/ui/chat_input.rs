//! Shared chat input used by both the Overview tab (compact row below
//! the sprite cards) and the Chat tab (pinned to the bottom of the
//! transcript). Fires `chat.send` via the UI → WS command channel
//! with the app's `selected_chat_agent` baked in on dispatch; the
//! reply streams back as `session.message` events and renders both
//! as a bubble over the target sprite and as an entry in that
//! agent's chat log.
//!
//! Disabled-when-disconnected: the `connected` flag from the gateway
//! hello-ok gates both the text field and the Send button so the
//! operator can't queue prompts that would be dropped. Empty/whitespace
//! input also disables Send (handled by omitting `on_press`).

use iced::widget::{button, container, row, text, text_input};
use iced::{Alignment, Border, Element, Length, Padding};

use crate::Message;
use crate::ui::theme;

pub fn view<'a>(input: &'a str, connected: bool, target_display: &str) -> Element<'a, Message> {
    let placeholder = if connected {
        format!("Ask {target_display} anything…")
    } else {
        "gateway disconnected".to_string()
    };

    let mut field = text_input(&placeholder, input)
        .padding(Padding::from([6, 10]))
        .size(13)
        .width(Length::Fill);
    if connected {
        field = field
            .on_input(Message::ChatInputChanged)
            .on_submit(Message::SendChat);
    }

    let can_send = connected && !input.trim().is_empty();
    let mut send_btn = button(text("Send").size(12)).padding(Padding::from([6, 14]));
    if can_send {
        send_btn = send_btn.on_press(Message::SendChat);
    }

    let inner = row![field, send_btn]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    container(inner)
        .width(Length::Fill)
        .padding(Padding::from([8, 16]))
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
