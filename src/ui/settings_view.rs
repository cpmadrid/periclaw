//! The "Settings" nav tab — connection configuration + (masked)
//! token management. This is also the first-run entry point: if no
//! gateway URL has ever been configured and the operator isn't in
//! mock mode, `App::new` force-selects this tab and we render a
//! banner explaining what's needed.
//!
//! ## What lives here vs. elsewhere
//!
//! - URL + mode → persisted alongside `UiState` on Save
//!   (`src/ui_state.rs::Settings`, written by `ui_state::save`).
//! - Token → routed through `crate::secret_store` so it lands in the
//!   OS keychain on release builds and the `0600` plaintext fallback
//!   file on debug builds. See `src/secret_store.rs` for why.
//!
//! The token input is **write-only**: we never read a stored value
//! back into the form, and Save clears the field on success. The UI
//! instead shows a boolean "token present / not set" indicator plus
//! a Clear button. That way the secret isn't exposed to the view
//! layer or to any subsequent re-render of the widget tree.

use std::collections::HashMap;

use iced::widget::{Space, button, column, container, pick_list, radio, row, text, text_input};
use iced::{Alignment, Border, Element, Length, Padding};

use crate::Message;
use crate::app::{ConnectionStatus, SettingsForm};
use crate::domain::{Agent, AgentId, Room};
use crate::ui::theme;
use crate::ui::widgets::card_style;
use crate::ui_state::Settings;

pub struct Snapshot<'a> {
    pub settings: &'a Settings,
    pub form: &'a SettingsForm,
    /// `true` when no URL is configured (persisted, env, or mock
    /// opted into). Drives the "first-run required" banner and the
    /// emphasis on the URL field.
    pub first_run_incomplete: bool,
    pub token_present: bool,
    /// Short label for the storage backend in use — shown next to
    /// the token input so the operator can see *where* their secret
    /// actually lives under the current build flavor.
    pub storage_location: &'static str,
    /// Result of the most recent connection attempt. Drives the
    /// status line under the Gateway URL field so the operator gets
    /// live feedback on Save instead of having to infer from the
    /// status bar whether their settings actually work.
    pub connection_status: &'a ConnectionStatus,
    /// Live room list for the Rooms editor section.
    pub rooms: &'a [Room],
    /// Current chat-capable agents and their per-agent home-room
    /// overrides. Used by the Agent rooms section to render one row
    /// per agent with a `pick_list` of room labels.
    pub roster: &'a [Agent],
    pub agent_rooms: &'a HashMap<AgentId, String>,
}

pub fn view<'a>(snap: Snapshot<'a>) -> Element<'a, Message> {
    let mut body: iced::widget::Column<'a, Message> = column![]
        .spacing(20)
        .padding(Padding::from([18, 24]))
        .width(Length::Fill);

    body = body.push(
        column![
            text("Settings").size(20).color(*theme::FOREGROUND),
            text("Connection configuration + token management.")
                .size(11)
                .color(*theme::MUTED),
        ]
        .spacing(4),
    );

    if snap.first_run_incomplete {
        body = body.push(first_run_banner());
    }

    body = body.push(gateway_url_section(snap.form));
    body = body.push(connection_status_row(snap.connection_status));
    body = body.push(mode_section(snap.form));
    body = body.push(token_section(
        snap.form,
        snap.token_present,
        snap.storage_location,
    ));
    body = body.push(save_row(&snap));
    body = body.push(rooms_section(snap.rooms));
    body = body.push(agent_rooms_section(
        snap.roster,
        snap.rooms,
        snap.agent_rooms,
    ));
    body = body.push(footer_hint(snap.settings));

    container(body).width(Length::Fill).into()
}

fn rooms_section<'a>(rooms: &'a [Room]) -> Element<'a, Message> {
    let row_count = rooms.len();
    let row_iter = rooms.iter().enumerate().map(|(idx, room)| {
        let id_for_label = room.id.clone();
        let id_for_up = room.id.clone();
        let id_for_down = room.id.clone();
        let id_for_del = room.id.clone();
        let label_input = text_input("room name", &room.label)
            .on_input(move |v| Message::RoomLabelChanged(id_for_label.clone(), v))
            .size(13)
            .padding(Padding::from([4, 8]))
            .width(Length::Fill);

        let up = button(text("↑").size(11))
            .padding(Padding::from([2, 8]))
            .on_press_maybe((idx > 0).then(|| Message::RoomMoveUp(id_for_up.clone())));
        let down = button(text("↓").size(11))
            .padding(Padding::from([2, 8]))
            .on_press_maybe(
                (idx + 1 < row_count).then(|| Message::RoomMoveDown(id_for_down.clone())),
            );
        let del = button(text("✕").size(11))
            .padding(Padding::from([2, 8]))
            .on_press_maybe((row_count > 1).then(|| Message::RoomDelete(id_for_del.clone())));

        row![up, down, label_input, del]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
    });

    let list: iced::widget::Column<'a, Message> = row_iter
        .fold(column![].spacing(6), |acc, el: Element<'a, Message>| {
            acc.push(el)
        });

    let add = button(text("+ Add room").size(12))
        .padding(Padding::from([4, 10]))
        .on_press(Message::RoomAdd);

    let card = container(
        column![
            text("Rooms").size(13).color(*theme::FOREGROUND),
            text(
                "Rename, reorder, add, or remove rooms. Changes persist \
                 immediately. At least one room is required.",
            )
            .size(11)
            .color(*theme::MUTED),
            list,
            row![Space::new().width(Length::Fill), add].align_y(Alignment::Center),
        ]
        .spacing(8),
    )
    .padding(Padding::from([12, 14]))
    .style(card_style(6.0));

    card.into()
}

fn agent_rooms_section<'a>(
    roster: &'a [Agent],
    rooms: &'a [Room],
    agent_rooms: &'a HashMap<AgentId, String>,
) -> Element<'a, Message> {
    // `pick_list` values must be Clone + Eq + Display. RoomOption
    // wraps (id, label) so the display string is the human label
    // while the stable id is what we dispatch.
    let options: Vec<RoomOption> = rooms
        .iter()
        .map(|r| RoomOption {
            id: r.id.clone(),
            label: r.label.clone(),
        })
        .collect();

    let row_iter = roster.iter().map(|agent| {
        let current_id = agent_rooms
            .get(&agent.id)
            .cloned()
            .unwrap_or_else(|| crate::domain::room::MAIN_ROOM.to_string());
        let selected = options.iter().find(|o| o.id == current_id).cloned();
        let agent_id = agent.id.clone();
        let pl = pick_list(options.clone(), selected, move |opt: RoomOption| {
            Message::AgentHomeRoomChanged(agent_id.clone(), opt.id)
        })
        .placeholder("choose room")
        .padding(Padding::from([4, 8]))
        .text_size(12);
        row![
            text(agent.display.as_str())
                .size(13)
                .color(*theme::FOREGROUND)
                .width(Length::Fill),
            pl,
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
    });

    let list: iced::widget::Column<'a, Message> = row_iter
        .fold(column![].spacing(6), |acc, el: Element<'a, Message>| {
            acc.push(el)
        });

    let body = if roster.is_empty() {
        column![
            text("Agents").size(13).color(*theme::FOREGROUND),
            text("No agents discovered yet.")
                .size(11)
                .color(*theme::MUTED),
        ]
        .spacing(8)
    } else {
        column![
            text("Agents").size(13).color(*theme::FOREGROUND),
            text("Pick which room each agent calls home.")
                .size(11)
                .color(*theme::MUTED),
            list,
        ]
        .spacing(8)
    };

    container(body)
        .padding(Padding::from([12, 14]))
        .style(card_style(6.0))
        .into()
}

/// Display wrapper for the agent-room `pick_list`. `Display` returns
/// the human label; equality is by stable room id so reordering or
/// renaming doesn't break selection matching.
#[derive(Debug, Clone)]
struct RoomOption {
    id: String,
    label: String,
}

impl PartialEq for RoomOption {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for RoomOption {}

impl std::fmt::Display for RoomOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn first_run_banner<'a>() -> Element<'a, Message> {
    let content = column![
        text("Gateway URL required to connect")
            .size(14)
            .color(*theme::FOREGROUND),
        text(
            "Enter your OpenClaw gateway WebSocket URL below and click Save. \
             Tokens are optional — Tailscale-authenticated gateways don't need \
             one. Run with OPENCLAW_MOCK=1 to preview without connecting.",
        )
        .size(12)
        .color(*theme::MUTED),
    ]
    .spacing(4);

    container(content)
        .width(Length::Fill)
        .padding(Padding::from([10, 14]))
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

fn gateway_url_section<'a>(form: &'a SettingsForm) -> Element<'a, Message> {
    let field = text_input("wss://gateway.example/", &form.gateway_url)
        .on_input(Message::SettingsGatewayUrlChanged)
        .size(13)
        .padding(Padding::from([6, 10]))
        .width(Length::Fill);

    column![
        text("Gateway URL").size(13).color(*theme::FOREGROUND),
        text("WebSocket endpoint — ws:// or wss://. `OPENCLAW_GATEWAY_URL` env var overrides this when set.")
            .size(11)
            .color(*theme::MUTED),
        field,
    ]
    .spacing(6)
    .into()
}

/// Status line rendered right under the Gateway URL field. Reads the
/// latest connect-attempt outcome: `(not tested)` before any save
/// this session, `⟳ connecting…` immediately after save, `✓ connected`
/// on success, `✗ <reason>` on failure. A reconnect loop on a bad
/// URL will cycle between connecting and failed honestly rather than
/// freezing on the first failure — that's information the operator
/// wants during debugging.
fn connection_status_row<'a>(status: &'a ConnectionStatus) -> Element<'a, Message> {
    let (body, color) = match status {
        ConnectionStatus::Untested => (
            "(not tested — click Save to try)".to_string(),
            *theme::MUTED,
        ),
        ConnectionStatus::Connecting => ("⟳ connecting…".to_string(), *theme::MUTED),
        ConnectionStatus::Ok => ("✓ connected".to_string(), *theme::STATUS_UP),
        ConnectionStatus::Failed(reason) => (format!("✗ {reason}"), *theme::STATUS_DOWN),
    };
    // Stack the label above the body instead of sitting inline with
    // it. Inline `row` kept the message on one line and squeezed the
    // layout when the gateway returned a verbose error; stacking
    // gives the body full width to wrap into, which `text`'s default
    // wrapping handles automatically.
    column![
        text("Connection status").size(11).color(*theme::MUTED),
        text(body).size(12).color(color).width(Length::Fill),
    ]
    .spacing(4)
    .into()
}

fn mode_section<'a>(form: &'a SettingsForm) -> Element<'a, Message> {
    // `radio` values must be `Copy + Eq`; `&'static str` satisfies
    // both and matches how `form.mode` is stored.
    let selected = Some(form.mode);
    let choice = |label: &'static str, value: &'static str| {
        radio(label, value, selected, Message::SettingsModeSelected)
            .size(13)
            .spacing(6)
    };

    column![
        text("Mode").size(13).color(*theme::FOREGROUND),
        text(
            "Auto picks mock when OPENCLAW_MOCK=1 is set, otherwise live WS. \
             Explicit Mock runs the offline fixture; WS forces a gateway connection.",
        )
        .size(11)
        .color(*theme::MUTED),
        row![
            choice("Auto", "auto"),
            choice("Live WS", "ws"),
            choice("Mock (offline)", "mock"),
        ]
        .spacing(18),
    ]
    .spacing(6)
    .into()
}

fn token_section<'a>(
    form: &'a SettingsForm,
    token_present: bool,
    storage_location: &'static str,
) -> Element<'a, Message> {
    let input = text_input("paste token and click Save (write-only)", &form.token)
        .on_input(Message::SettingsTokenChanged)
        .secure(true)
        .size(13)
        .padding(Padding::from([6, 10]))
        .width(Length::Fill);

    let status_text = if token_present {
        format!("✓ token saved to {storage_location}")
    } else {
        format!("(no token saved; storage backend: {storage_location})")
    };
    let status = text(status_text).size(11).color(*theme::MUTED);

    let clear = button(text("Clear token").size(12))
        .padding(Padding::from([4, 10]))
        .on_press_maybe(token_present.then_some(Message::SettingsClearToken));

    column![
        text("Token").size(13).color(*theme::FOREGROUND),
        text(
            "Optional. Required only when the gateway doesn't accept \
             Tailscale-whois auth. Leave blank on Save to keep the current token.",
        )
        .size(11)
        .color(*theme::MUTED),
        input,
        row![status, Space::new().width(Length::Fill), clear].align_y(Alignment::Center),
    ]
    .spacing(6)
    .into()
}

fn save_row<'a>(_snap: &Snapshot<'a>) -> Element<'a, Message> {
    // Labeled "Connect" rather than "Save" because it does both —
    // persists the form and immediately restarts the ws subscription
    // so the Connection status line shows whether the values work.
    let connect = button(text("Connect").size(13))
        .padding(Padding::from([6, 18]))
        .on_press(Message::SettingsSave);
    row![Space::new().width(Length::Fill), connect]
        .align_y(Alignment::Center)
        .into()
}

fn footer_hint<'a>(settings: &'a Settings) -> Element<'a, Message> {
    let currently_saved = match settings.gateway_url.as_deref() {
        Some(url) => format!("Currently saved: {url}"),
        None => "Currently saved: (none — using env var or staying idle)".to_string(),
    };
    column![
        text(currently_saved).size(11).color(*theme::MUTED),
        text(
            "If no token is set, the app also checks `~/.openclaw/openclaw.json` \
             as a last-resort bootstrap (useful for OpenClaw-CLI users).",
        )
        .size(11)
        .color(*theme::MUTED),
    ]
    .spacing(2)
    .into()
}
