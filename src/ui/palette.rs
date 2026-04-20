//! Command palette overlay — search input + ranked action list.
//!
//! Rendered on top of the main view via `stack` when
//! `App::palette_open` is true. Keyboard events for navigation
//! (Up/Down/Enter/Escape) are handled by `app::subscription` so the
//! widget itself doesn't need to be focus-stealing — only the text
//! input needs focus, and it's auto-focused on open via a
//! `text_input::focus` task.

use std::sync::LazyLock;

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding, Shadow, Vector};

use crate::Message;
use crate::palette::{PaletteEntry, PaletteGroup};
use crate::ui::theme;

/// Id for the palette's text_input so `app.rs` can issue a focus
/// task when the palette opens.
pub static INPUT_ID: LazyLock<iced::widget::Id> =
    LazyLock::new(|| iced::widget::Id::new("palette-input"));

pub fn input_id() -> iced::widget::Id {
    INPUT_ID.clone()
}

pub fn view<'a>(
    query: &'a str,
    entries: Vec<PaletteEntry>,
    ranked: Vec<(usize, u32)>,
    selected: usize,
) -> Element<'a, Message> {
    let input = text_input("Type a command…", query)
        .id(input_id())
        .on_input(Message::PaletteInputChanged)
        .on_submit(Message::PaletteExecute)
        .size(14)
        .padding(Padding::from([10, 12]))
        .width(Length::Fill);

    let total_count = entries.len();
    let ranked_count = ranked.len();
    let body: Element<'a, Message> = if ranked.is_empty() {
        container(text("no matches").size(12).color(*theme::MUTED))
            .padding(Padding::from(16))
            .into()
    } else {
        build_list(entries, ranked, selected, query.trim().is_empty())
    };

    let footer = row![
        text("↑↓ navigate").size(10).color(*theme::MUTED),
        Space::new().width(Length::Fixed(16.0)),
        text("↵ run").size(10).color(*theme::MUTED),
        Space::new().width(Length::Fixed(16.0)),
        text("esc close").size(10).color(*theme::MUTED),
        Space::new().width(Length::Fill),
        text(format!("{ranked_count} of {total_count}"))
            .size(10)
            .color(*theme::MUTED),
    ]
    .align_y(Alignment::Center);

    // The palette pane itself — dark card, subtle border, slight
    // shadow so it reads as "floating above" the base layout.
    let pane = container(
        column![
            input,
            container(body)
                .width(Length::Fill)
                .height(Length::Fixed(360.0)),
            container(footer).padding(Padding::from([8, 12])),
        ]
        .spacing(0),
    )
    .width(Length::Fixed(640.0))
    .style(|_| container::Style {
        background: Some((*theme::SURFACE_2).into()),
        border: Border {
            color: *theme::TERMINAL_GREEN,
            width: 1.0,
            radius: 8.0.into(),
        },
        shadow: Shadow {
            color: Color {
                a: 0.6,
                ..Color::BLACK
            },
            offset: Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
        ..Default::default()
    });

    // Backdrop — translucent scrim that also catches clicks outside
    // the pane to dismiss the palette. The button's `on_press` is
    // the dismissal hook; the pane above absorbs clicks inside via
    // its own layering.
    let scrim = button(
        container(pane)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::padding::top(96.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Start),
    )
    .on_press(Message::PaletteClose)
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(0)
    .style(|_, _| button::Style {
        background: Some(Background::Color(Color {
            a: 0.55,
            ..Color::BLACK
        })),
        text_color: *theme::FOREGROUND,
        border: Border::default(),
        shadow: Shadow::default(),
        ..Default::default()
    });

    scrim.into()
}

fn build_list<'a>(
    entries: Vec<PaletteEntry>,
    ranked: Vec<(usize, u32)>,
    selected: usize,
    show_group_headers: bool,
) -> Element<'a, Message> {
    let mut col = column![].spacing(0);
    let mut last_group: Option<PaletteGroup> = None;
    for (list_idx, (entry_idx, _score)) in ranked.iter().enumerate() {
        let entry = &entries[*entry_idx];
        if show_group_headers && last_group != Some(entry.group) {
            col = col.push(group_header(entry.group));
            last_group = Some(entry.group);
        }
        col = col.push(entry_row_owned(
            entry.clone(),
            list_idx,
            list_idx == selected,
        ));
    }
    scrollable(col).height(Length::Fill).into()
}

fn group_header(group: PaletteGroup) -> Element<'static, Message> {
    container(
        text(group.label())
            .size(10)
            .color(*theme::MUTED)
            .font(iced::Font::MONOSPACE),
    )
    .padding(Padding {
        top: 8.0,
        right: 14.0,
        bottom: 4.0,
        left: 14.0,
    })
    .into()
}

fn entry_row_owned<'a>(entry: PaletteEntry, list_idx: usize, active: bool) -> Element<'a, Message> {
    let label_color = if active {
        *theme::TERMINAL_GREEN
    } else {
        *theme::FOREGROUND
    };
    let content: Element<'a, Message> = match entry.subtitle {
        Some(sub) => column![
            text(entry.label).size(13).color(label_color),
            text(sub).size(10).color(*theme::MUTED),
        ]
        .spacing(2)
        .into(),
        None => text(entry.label).size(13).color(label_color).into(),
    };

    let bg = if active {
        Some(Background::Color(*theme::SURFACE_3))
    } else {
        None
    };

    button(content)
        .on_press(Message::PaletteSelectAndExecute(list_idx))
        .width(Length::Fill)
        .padding(Padding::from([6, 14]))
        .style(move |_, status| {
            let hovered = matches!(status, iced::widget::button::Status::Hovered);
            let resolved_bg = bg.or_else(|| hovered.then(|| Background::Color(*theme::SURFACE_3)));
            iced::widget::button::Style {
                background: resolved_bg,
                text_color: label_color,
                border: Border::default(),
                shadow: Shadow::default(),
                ..Default::default()
            }
        })
        .into()
}
