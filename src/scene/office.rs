//! Canvas Program rendering the Agent Office.
//!
//! Agents are pinned to their configured home room. Jobs don't render
//! as sprites at all — instead, when any job is `Running` for an
//! agent's room, a floating "power-up" sparkle appears above the
//! agent and a `BubbleKind::Work` bubble names the job. This keeps
//! the scene uncluttered: the only inhabitants are actual chat
//! agents, and ambient signal comes from overlays rather than
//! peer-sprites.

use std::collections::HashMap;
use std::time::Instant;

use iced::mouse;
use iced::widget::canvas::{self, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

use crate::Message;
use crate::domain::room::MAIN_ROOM;
use crate::domain::{Agent, AgentId, AgentStatus, Job, JobId, Room};
use crate::scene::{RoomLayout, ThoughtBubble, sprite};
use crate::ui::theme;

/// Snapshot of what to draw. Cheap to clone; recreated each `view()`.
pub struct OfficeScene<'a> {
    pub roster: &'a [Agent],
    pub jobs: &'a HashMap<JobId, Job>,
    pub statuses: &'a HashMap<AgentId, AgentStatus>,
    pub rooms: &'a [Room],
    /// Per-agent home-room override. Agents absent here fall back to
    /// `MAIN_ROOM`. Missing from the scene entirely when their room
    /// id no longer exists in `rooms`.
    pub agent_rooms: &'a HashMap<AgentId, String>,
    pub bubbles: &'a [ThoughtBubble],
    pub transition_moments: &'a HashMap<AgentId, Instant>,
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
            let layout = RoomLayout::new(scene_bounds, self.rooms);

            for (room, rect) in layout.iter() {
                draw_room(frame, room, rect);
            }

            // Group agents by their home room.
            let mut per_room: HashMap<&str, Vec<&Agent>> = HashMap::new();
            for agent in self.roster {
                let room_id = resolve_agent_room(agent, self.agent_rooms);
                per_room.entry(room_id).or_default().push(agent);
            }

            // Is any job routed to this room currently Running? We
            // don't render jobs themselves, but Running state drives
            // the power-up overlay on the agent in that room.
            let running_rooms: std::collections::HashSet<&str> = self
                .jobs
                .values()
                .filter(|j| matches!(j.status, AgentStatus::Running))
                .map(|_j| MAIN_ROOM)
                .collect();

            let now = Instant::now();
            let seconds = clock_phase(now);
            let mut sprite_positions: HashMap<AgentId, Point> = HashMap::new();

            for (room_id, agents) in per_room {
                let Some(room_rect) = layout.room_rect(room_id) else {
                    // Agent's configured home no longer exists — skip.
                    continue;
                };
                for (idx, agent) in agents.iter().enumerate() {
                    let status = self
                        .statuses
                        .get(&agent.id)
                        .copied()
                        .unwrap_or(AgentStatus::Unknown);
                    let base_pos = layout.sprite_slot(room_rect, idx);
                    let flash = self
                        .transition_moments
                        .get(&agent.id)
                        .map(|t| transition_flash(now.saturating_duration_since(*t)))
                        .unwrap_or(0.0);
                    let sprite_half = sprite::LOBSTER_SIZE;
                    let (wander, facing_left) = wander_offset(
                        agent.id.as_str(),
                        &room_rect,
                        base_pos,
                        Size::new(sprite_half.width / 2.0, sprite_half.height / 2.0),
                        seconds,
                        status,
                    );
                    let wander_pos = Point::new(base_pos.x + wander.x, base_pos.y + wander.y);
                    let pos = animated_position(wander_pos, status, now);
                    let working = running_rooms.contains(room_id);
                    draw_agent(
                        frame,
                        pos,
                        agent,
                        status,
                        flash,
                        seconds,
                        facing_left,
                        working,
                    );
                    sprite_positions.insert(agent.id.clone(), wander_pos);
                }
            }

            for bubble in self.bubbles {
                let Some(alpha) = bubble.alpha(now) else {
                    continue;
                };
                let Some(anchor) = sprite_positions.get(&bubble.agent).copied() else {
                    continue;
                };
                draw_bubble(frame, anchor, &bubble.text, bubble.kind, alpha);
            }

            if *crate::scene::sprite::DEBUG_SPRITES {
                sprite::draw_lobster(
                    frame,
                    Point::new(80.0, 80.0),
                    AgentStatus::Running,
                    seconds,
                    false,
                );
            }
        });

        vec![geometry]
    }
}

/// Where a chat agent sprite lives. Reads the operator's per-agent
/// override first; falls back to `MAIN_ROOM`. Status no longer
/// participates — errored agents stay in place and signal via a red
/// halo (see `draw_agent`).
fn resolve_agent_room<'a>(agent: &'a Agent, overrides: &'a HashMap<AgentId, String>) -> &'a str {
    overrides
        .get(&agent.id)
        .map(String::as_str)
        .unwrap_or(MAIN_ROOM)
}

fn draw_room(frame: &mut canvas::Frame, room: &Room, rect: Rectangle) {
    use crate::scene::sprite::{self, DEFAULT_SCALE};

    let panel = Path::rectangle(rect.position(), rect.size());
    frame.fill(&panel, *theme::SURFACE_1);
    frame.stroke(
        &panel,
        Stroke::default().with_color(*theme::BORDER).with_width(1.0),
    );

    frame.fill_text(Text {
        content: room.label.clone(),
        position: Point::new(rect.x + 12.0, rect.y + 10.0),
        color: *theme::TERMINAL_GREEN,
        size: 13.0.into(),
        font: iced::Font::MONOSPACE,
        ..Text::default()
    });

    if let Some(decor) = sprite::decor_for_room(room.id.as_str()) {
        let size = sprite::sprite_size_px(decor, DEFAULT_SCALE);
        let decor_center = Point::new(
            rect.x + rect.width - size.width / 2.0 - 16.0,
            rect.y + size.height / 2.0 + 20.0,
        );
        sprite::draw_sprite_pixels(
            frame,
            decor_center,
            decor,
            DEFAULT_SCALE,
            *theme::MUTED,
            0.75,
            0.0,
            0.0,
            false,
        );
    }
}

fn draw_bubble(
    frame: &mut canvas::Frame,
    anchor: Point,
    text: &str,
    kind: crate::scene::BubbleKind,
    alpha: f32,
) {
    let width = (text.len() as f32 * 6.5).max(40.0) + 12.0;
    let height = 20.0;
    let bubble_origin = Point::new(anchor.x - width / 2.0, anchor.y - 42.0);

    let accent = match kind {
        crate::scene::BubbleKind::Tool => *theme::STATUS_DEGRADED,
        crate::scene::BubbleKind::Outgoing => *theme::MUTED,
        _ => *theme::TERMINAL_GREEN,
    };
    let fill = Color {
        a: alpha * 0.95,
        ..(*theme::SURFACE_3)
    };
    let border = Color {
        a: alpha * 0.9,
        ..accent
    };
    let text_col = Color { a: alpha, ..accent };

    let rect = Path::rectangle(bubble_origin, Size::new(width, height));
    frame.fill(&rect, fill);
    frame.stroke(&rect, Stroke::default().with_color(border).with_width(1.0));

    let tail_top = Point::new(anchor.x, bubble_origin.y + height);
    let tail_bottom = Point::new(anchor.x, bubble_origin.y + height + 6.0);
    let tail = Path::line(tail_top, tail_bottom);
    frame.stroke(&tail, Stroke::default().with_color(border).with_width(2.0));

    frame.fill_text(Text {
        content: text.to_string(),
        position: Point::new(bubble_origin.x + 6.0, bubble_origin.y + 4.0),
        color: text_col,
        size: 11.0.into(),
        font: iced::Font::MONOSPACE,
        ..Text::default()
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_agent(
    frame: &mut canvas::Frame,
    pos: Point,
    agent: &Agent,
    status: AgentStatus,
    flash: f32,
    seconds: f32,
    flip_h: bool,
    working: bool,
) {
    use crate::scene::sprite::{self, LOBSTER_SIZE, POWER_UP};

    // Errored agents render a persistent red halo to signal the state
    // in place (rather than being routed to a dedicated error room).
    // Transition flashes continue to use the agent's own color as they
    // did before — the two halos layer cleanly because the error halo
    // sits behind the flash pad.
    if matches!(status, AgentStatus::Error) {
        let pad = 4.0;
        let halo_origin = Point::new(
            pos.x - LOBSTER_SIZE.width / 2.0 - pad,
            pos.y - LOBSTER_SIZE.height / 2.0 - pad,
        );
        let halo_rect = Size::new(
            LOBSTER_SIZE.width + pad * 2.0,
            LOBSTER_SIZE.height + pad * 2.0,
        );
        frame.stroke(
            &Path::rectangle(halo_origin, halo_rect),
            Stroke::default()
                .with_color(Color {
                    a: 0.6,
                    ..*theme::STATUS_DOWN
                })
                .with_width(1.5),
        );
    }

    if flash > 0.0 {
        let pad = 6.0 + 4.0 * flash;
        let halo_origin = Point::new(
            pos.x - LOBSTER_SIZE.width / 2.0 - pad,
            pos.y - LOBSTER_SIZE.height / 2.0 - pad,
        );
        let halo_rect = Size::new(
            LOBSTER_SIZE.width + pad * 2.0,
            LOBSTER_SIZE.height + pad * 2.0,
        );
        let halo = Path::rectangle(halo_origin, halo_rect);
        frame.stroke(
            &halo,
            Stroke::default()
                .with_color(Color {
                    a: 0.25 + 0.55 * flash,
                    ..agent.color()
                })
                .with_width(2.0),
        );
    }

    sprite::draw_lobster(frame, pos, status, seconds, flip_h);

    // Power-up sparkle — a small animated indicator floating above
    // the agent's head while any of their jobs is running. Bobs on a
    // 2 Hz sine so it reads as active even if the underlying sprite
    // is idle-cycling.
    if working {
        let bob = (seconds * std::f32::consts::TAU * 2.0).sin() * 2.0;
        let anchor = Point::new(pos.x, pos.y - LOBSTER_SIZE.height / 2.0 - 14.0 + bob);
        sprite::draw_sprite_pixels(
            frame,
            anchor,
            &POWER_UP,
            3.0,
            *theme::TERMINAL_GREEN,
            1.0,
            seconds,
            4.0,
            false,
        );
    }

    let approx_width = (agent.display.len() as f32) * 6.0;
    let label_y = pos.y + LOBSTER_SIZE.height / 2.0 + 6.0;
    frame.fill_text(Text {
        content: agent.display.to_string(),
        position: Point::new(pos.x - approx_width / 2.0, label_y),
        color: *theme::FOREGROUND,
        size: 11.0.into(),
        font: iced::Font::MONOSPACE,
        ..Text::default()
    });
}

fn wander_offset(
    phase_key: &str,
    room_rect: &Rectangle,
    base_pos: Point,
    sprite_half: Size,
    seconds: f32,
    status: AgentStatus,
) -> (Point, bool) {
    if !matches!(status, AgentStatus::Ok | AgentStatus::Unknown) {
        return (Point::ORIGIN, false);
    }
    const EDGE_MARGIN: f32 = 4.0;
    let left_room = (base_pos.x - room_rect.x) - sprite_half.width - EDGE_MARGIN;
    let right_room = (room_rect.x + room_rect.width - base_pos.x) - sprite_half.width - EDGE_MARGIN;
    let top_room = (base_pos.y - room_rect.y) - sprite_half.height - EDGE_MARGIN;
    let bottom_room =
        (room_rect.y + room_rect.height - base_pos.y) - sprite_half.height - EDGE_MARGIN;
    let headroom_x = left_room.min(right_room).max(0.0);
    let headroom_y = top_room.min(bottom_room).max(0.0);

    let amp_x = (room_rect.width * 0.18).clamp(0.0, 60.0).min(headroom_x);
    let amp_y = (room_rect.height * 0.08).clamp(0.0, 24.0).min(headroom_y);

    let mut h: u64 = 5381;
    for b in phase_key.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    let phase_x = (h % 360) as f32 * std::f32::consts::PI / 180.0;
    let phase_y = ((h / 7) % 360) as f32 * std::f32::consts::PI / 180.0;
    let speed = 0.10 + ((h % 40) as f32 * 0.0045);

    let t = seconds * speed;
    let offset_x = (t + phase_x).sin() * amp_x;
    let offset_y = (t * 0.73 + phase_y).cos() * amp_y;
    let facing_left = (t + phase_x).cos() < 0.0;
    (Point::new(offset_x, offset_y), facing_left)
}

fn animated_position(base: Point, status: AgentStatus, now: Instant) -> Point {
    if !matches!(status, AgentStatus::Running) {
        return base;
    }
    const BOB_AMPLITUDE_PX: f32 = 3.0;
    const BOB_HZ: f32 = 1.5;
    let t = clock_phase(now);
    let offset = BOB_AMPLITUDE_PX * (t * std::f32::consts::TAU * BOB_HZ).sin();
    Point::new(base.x, base.y + offset)
}

fn clock_phase(now: Instant) -> f32 {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let epoch = *EPOCH.get_or_init(|| now);
    now.saturating_duration_since(epoch).as_secs_f32()
}

fn transition_flash(age: std::time::Duration) -> f32 {
    const FLASH_MS: f32 = 600.0;
    let elapsed_ms = age.as_millis() as f32;
    if elapsed_ms >= FLASH_MS {
        return 0.0;
    }
    let t = elapsed_ms / FLASH_MS;
    (1.0 - t).powi(2)
}
