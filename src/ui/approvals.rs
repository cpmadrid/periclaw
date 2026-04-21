//! Panel of pending exec-approval requests with Allow / Deny buttons.
//!
//! Rendered above the status bar when `pending_approvals` is non-empty.
//! Each row shows the command summary and two buttons; clicking emits
//! a `Message::ResolveApproval` which the app dispatches over the
//! UI → WS command channel.
//!
//! We do NOT surface `allow-always`. Desktop one-clicks should stay
//! narrow-scope — persistent policy changes belong in a more
//! deliberate UI.

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Border, Element, Length, Padding};

use crate::Message;
use crate::net::rpc::ApprovalEventPayload;
use crate::ui::theme;

/// Render a "scope-upgrade pair-request pending" notice. Shows the
/// CLI command in a selectable field + a Copy button so the operator
/// doesn't have to manually retype or dig through logs.
pub fn scope_upgrade_notice(request_id: &str) -> Element<'_, Message> {
    let command = format!("openclaw devices approve {request_id}");

    // text_input needs an on_input to stay interactive/selectable;
    // we wire it to the InputDiscard sink so any edits are dropped.
    let field = iced::widget::text_input("", &command)
        .on_input(Message::InputDiscard)
        .font(iced::Font::MONOSPACE)
        .size(11)
        .padding(Padding::from([4, 8]))
        .width(Length::Fill);

    let copy = iced::widget::button(text("Copy").size(11))
        .padding(Padding::from([4, 10]))
        .on_press(Message::CopyToClipboard(command.clone()));

    let command_row = row![field, copy]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    // Without this, the WS session sleeps up to 5 minutes between
    // reconnect attempts (see `SCOPE_UPGRADE_BACKOFF`) — the operator
    // would approve the pair-request and then watch the UI stay
    // disconnected for minutes. The button sends a Reconnect command
    // that interrupts that sleep.
    let retry = iced::widget::button(text("Retry now").size(11))
        .padding(Padding::from([4, 10]))
        .on_press(Message::RequestReconnect);

    let body = column![
        text("Scope upgrade pending").size(12).color(*theme::MUTED),
        text(
            "The gateway has filed a pair-request to grant this \
             desktop approvals permission. Run this on the gateway \
             host, then click Retry now:",
        )
        .size(12)
        .color(*theme::FOREGROUND),
        command_row,
        row![Space::new().width(Length::Fill), retry]
            .align_y(Alignment::Center)
            .width(Length::Fill),
    ]
    .spacing(6);

    container(body)
        .width(Length::Fill)
        .padding(Padding::from([10, 16]))
        .style(|_| container::Style {
            background: Some((*theme::SURFACE_1).into()),
            border: Border {
                color: *theme::STATUS_DEGRADED,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Render the approvals panel. Returns an empty container when there
/// are no pending approvals so callers can always include it in the
/// layout without special-casing.
pub fn view<'a>(
    pending: impl IntoIterator<Item = (&'a String, &'a ApprovalEventPayload)>,
) -> Element<'a, Message> {
    let rows = pending
        .into_iter()
        .map(row_for)
        .fold(column![].spacing(6), |acc, el| acc.push(el));

    let body: iced::widget::Column<'a, Message> = column![
        text("Pending approvals").size(12).color(*theme::MUTED),
        rows,
    ]
    .spacing(8);

    container(body)
        .width(Length::Fill)
        .padding(Padding::from([10, 16]))
        .style(|_| container::Style {
            background: Some((*theme::SURFACE_1).into()),
            border: Border {
                color: *theme::STATUS_DEGRADED,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn row_for<'a>(entry: (&'a String, &'a ApprovalEventPayload)) -> Element<'a, Message> {
    let (id, payload) = entry;
    let summary_line = summary_for(payload);
    let tool_label = payload.tool.as_deref().unwrap_or("exec");

    let header = row![
        text(tool_label).size(13).color(*theme::FOREGROUND),
        text(summary_line).size(12).color(*theme::MUTED),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    let allow = button(text("Allow").size(12))
        .padding(Padding::from([4, 10]))
        .on_press(Message::ResolveApproval {
            id: id.clone(),
            decision: "allow-once",
        });
    let deny = button(text("Deny").size(12))
        .padding(Padding::from([4, 10]))
        .on_press(Message::ResolveApproval {
            id: id.clone(),
            decision: "deny",
        });

    row![header, Space::new().width(Length::Fill), allow, deny]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
}

fn summary_for(payload: &ApprovalEventPayload) -> String {
    // Prefer the server-provided summary; fall back to a best-effort
    // shape so even un-summarized requests render as something.
    payload
        .summary
        .clone()
        .or_else(|| payload.session_key.clone().map(|s| format!("session: {s}")))
        .unwrap_or_else(|| "pending approval".to_string())
}
