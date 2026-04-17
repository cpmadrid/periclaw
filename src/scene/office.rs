//! Canvas Program rendering the Agent Office.
//!
//! Draws the 3×2 grid of rooms with labels, then places one colored
//! circle per agent at its assigned slot. Real sprites (PNG atlas
//! with nearest-neighbor filtering) land in M4.

use std::collections::HashMap;

use iced::mouse;
use iced::widget::canvas::{self, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Theme};

use crate::Message;
use crate::domain::{Agent, AgentId, AgentStatus, RoomId, room_for};
use crate::scene::RoomLayout;
use crate::ui::theme;

/// Snapshot of what to draw. Cheap to clone; recreated each `view()`.
pub struct OfficeScene<'a> {
    pub roster: &'a [Agent],
    pub statuses: &'a HashMap<AgentId, AgentStatus>,
    pub cache: &'a canvas::Cache,
}

impl<'a> canvas::Program<Message> for OfficeScene<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            let scene_bounds = Rectangle::new(Point::ORIGIN, bounds.size());
            let layout = RoomLayout::new(scene_bounds);

            for room in [
                RoomId::Observatory,
                RoomId::CommandHq,
                RoomId::Security,
                RoomId::ResearchLab,
                RoomId::MemoryVault,
                RoomId::Studio,
            ] {
                draw_room(frame, &layout, room);
            }

            // Group sprites by room so we can slot them inside.
            let mut per_room: HashMap<RoomId, Vec<&Agent>> = HashMap::new();
            for agent in self.roster {
                let status = self
                    .statuses
                    .get(&agent.id)
                    .copied()
                    .unwrap_or(AgentStatus::Unknown);
                let room = room_for(&agent.id, agent.kind, status);
                per_room.entry(room).or_default().push(agent);
            }

            for (room, agents) in per_room {
                for (idx, agent) in agents.iter().enumerate() {
                    let pos = layout.sprite_slot(room, idx);
                    draw_sprite(frame, pos, agent);
                }
            }
        });

        vec![geometry]
    }
}

fn draw_room(frame: &mut canvas::Frame, layout: &RoomLayout, room: RoomId) {
    let rect = layout.room_rect(room);

    // Background panel — slightly elevated from the page surface.
    let panel = Path::rectangle(rect.position(), rect.size());
    frame.fill(&panel, *theme::SURFACE_1);

    // Border — faint terminal green outline.
    frame.stroke(
        &panel,
        Stroke::default().with_color(*theme::BORDER).with_width(1.0),
    );

    // Room label — small, top-left corner of the panel.
    frame.fill_text(Text {
        content: room.label().to_string(),
        position: Point::new(rect.x + 12.0, rect.y + 10.0),
        color: *theme::TERMINAL_GREEN,
        size: 13.0.into(),
        font: iced::Font::MONOSPACE,
        ..Text::default()
    });
}

fn draw_sprite(frame: &mut canvas::Frame, pos: Point, agent: &Agent) {
    let color: Color = agent.color();

    // Body: small filled circle (placeholder for pixel sprite).
    let body = Path::circle(pos, 12.0);
    frame.fill(&body, color);

    // Soft glow ring.
    frame.stroke(
        &body,
        Stroke::default()
            .with_color(Color { a: 0.35, ..color })
            .with_width(4.0),
    );

    // Name tag under the sprite. We approximate "centered" by offsetting
    // by half the expected text width (monospace ≈ 6px per char @ 11pt).
    let approx_width = (agent.display.len() as f32) * 6.0;
    frame.fill_text(Text {
        content: agent.display.to_string(),
        position: Point::new(pos.x - approx_width / 2.0, pos.y + 18.0),
        color: *theme::FOREGROUND,
        size: 11.0.into(),
        font: iced::Font::MONOSPACE,
        ..Text::default()
    });
}
