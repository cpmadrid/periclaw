//! Canvas Program rendering the Agent Office.
//!
//! Agents are pinned to their configured home room. Jobs don't render
//! as sprites at all — instead, when any job is `Running` for an
//! agent's room, the scene can show a floating "power-up" sparkle
//! above the agent or a `BubbleKind::Work` bubble naming the job.
//! Visible bubbles win over the sparkle for the same agent so the
//! scene stays legible: sparkle is reserved for silent work. This
//! keeps the office uncluttered — the only inhabitants are actual
//! chat agents, and ambient signal comes from overlays rather than
//! peer-sprites.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use iced::mouse;
use iced::widget::canvas::{self, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

use crate::Message;
use crate::domain::room::MAIN_ROOM;
use crate::domain::{Agent, AgentId, AgentStatus, Job, JobId, Room};
use crate::scene::{RoomLayout, ThoughtBubble, sprite};
use crate::ui::theme;

const BUBBLE_TAIL_LENGTH: f32 = 6.0;
const BUBBLE_STACK_GAP: f32 = 7.0;
const BUBBLE_SPRITE_CLEARANCE: f32 = 8.0;

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
    /// Per-job room override. Jobs do not render as sprites, but a
    /// running job lights whichever room is configured here.
    pub job_rooms: &'a HashMap<JobId, String>,
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

            // Which rooms have any kind of activity right now? We
            // don't render jobs themselves, but a running job in an
            // agent's room — OR the agent itself being Running (chat
            // prompt in flight, tool call, etc.) — is the base
            // signal for the power-up sparkle. Per-agent visible
            // bubbles can still suppress it below.
            let mut running_rooms: HashSet<&str> = HashSet::new();
            for job in self.jobs.values() {
                if matches!(job.status, AgentStatus::Running) {
                    running_rooms.insert(resolve_job_room(job, self.job_rooms, self.rooms));
                }
            }

            let now = Instant::now();
            let visible_bubble_agents: HashSet<AgentId> = self
                .bubbles
                .iter()
                .filter(|bubble| bubble.alpha(now).is_some())
                .map(|bubble| bubble.agent.clone())
                .collect();

            for agent in self.roster {
                if matches!(
                    self.statuses
                        .get(&agent.id)
                        .copied()
                        .unwrap_or(AgentStatus::Unknown),
                    AgentStatus::Running,
                ) {
                    running_rooms.insert(resolve_agent_room(agent, self.agent_rooms));
                }
            }

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
                    let working = should_render_power_up(
                        &agent.id,
                        room_id,
                        &running_rooms,
                        &visible_bubble_agents,
                    );
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
                    sprite_positions.insert(agent.id.clone(), pos);
                }
            }

            let mut bubble_bottoms: HashMap<AgentId, f32> = HashMap::new();
            for bubble in self.bubbles.iter().rev() {
                let Some(alpha) = bubble.alpha(now) else {
                    continue;
                };
                let Some(anchor) = sprite_positions.get(&bubble.agent).copied() else {
                    continue;
                };
                let bubble_bottom = bubble_bottoms
                    .entry(bubble.agent.clone())
                    .or_insert_with(|| initial_bubble_bottom(anchor));
                let bubble_height = draw_bubble(
                    frame,
                    anchor,
                    scene_bounds,
                    &bubble.text,
                    bubble.kind,
                    alpha,
                    *bubble_bottom,
                );
                *bubble_bottom -= bubble_height + BUBBLE_TAIL_LENGTH + BUBBLE_STACK_GAP;
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

fn resolve_job_room<'a>(
    job: &'a Job,
    overrides: &'a HashMap<JobId, String>,
    rooms: &'a [Room],
) -> &'a str {
    let configured = overrides
        .get(&job.id)
        .map(String::as_str)
        .unwrap_or(MAIN_ROOM);
    if rooms.iter().any(|room| room.id == configured) {
        configured
    } else {
        MAIN_ROOM
    }
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
    bounds: Rectangle,
    text: &str,
    kind: crate::scene::BubbleKind,
    alpha: f32,
    bubble_bottom: f32,
) -> f32 {
    const CHAR_WIDTH: f32 = 6.5;
    const H_PADDING: f32 = 10.0;
    const V_PADDING: f32 = 7.0;
    const LINE_HEIGHT: f32 = 13.0;
    const SCENE_PAD: f32 = 8.0;

    let layout = bubble_layout(text, bounds, CHAR_WIDTH, H_PADDING, V_PADDING, LINE_HEIGHT);
    let usable_width = (bounds.width - SCENE_PAD * 2.0).max(0.0);
    let left = if layout.width <= usable_width {
        (anchor.x - layout.width / 2.0).clamp(
            bounds.x + SCENE_PAD,
            bounds.x + bounds.width - layout.width - SCENE_PAD,
        )
    } else {
        anchor.x - layout.width / 2.0
    };
    let bubble_origin = Point::new(left, bubble_bottom - layout.height);

    let accent = match kind {
        crate::scene::BubbleKind::Tool => *theme::STATUS_DEGRADED,
        crate::scene::BubbleKind::Outgoing => *theme::MUTED,
        _ => *theme::TERMINAL_GREEN,
    };
    let fill = *theme::SURFACE_0;
    let border = Color {
        a: alpha * 0.9,
        ..accent
    };
    let text_col = Color { a: alpha, ..accent };

    let rect = Path::rectangle(bubble_origin, Size::new(layout.width, layout.height));
    frame.fill(&rect, fill);
    frame.stroke(&rect, Stroke::default().with_color(border).with_width(1.0));

    let tail_x = anchor
        .x
        .clamp(bubble_origin.x + 6.0, bubble_origin.x + layout.width - 6.0);
    let tail_top = Point::new(tail_x, bubble_bottom);
    let tail_bottom = Point::new(tail_x, bubble_bottom + BUBBLE_TAIL_LENGTH);
    let tail = Path::line(tail_top, tail_bottom);
    frame.stroke(&tail, Stroke::default().with_color(border).with_width(2.0));

    for (idx, line) in layout.lines.iter().enumerate() {
        frame.fill_text(Text {
            content: line.clone(),
            position: Point::new(
                bubble_origin.x + H_PADDING,
                bubble_origin.y + V_PADDING + idx as f32 * LINE_HEIGHT,
            ),
            color: text_col,
            size: 11.0.into(),
            font: iced::Font::MONOSPACE,
            ..Text::default()
        });
    }

    layout.height
}

fn initial_bubble_bottom(anchor: Point) -> f32 {
    anchor.y - sprite::LOBSTER_SIZE.height / 2.0 - BUBBLE_SPRITE_CLEARANCE - BUBBLE_TAIL_LENGTH
}

fn should_render_power_up(
    agent_id: &AgentId,
    room_id: &str,
    running_rooms: &HashSet<&str>,
    visible_bubble_agents: &HashSet<AgentId>,
) -> bool {
    running_rooms.contains(room_id) && !visible_bubble_agents.contains(agent_id)
}

fn bubble_layout(
    text: &str,
    bounds: Rectangle,
    char_width: f32,
    h_padding: f32,
    v_padding: f32,
    line_height: f32,
) -> BubbleLayout {
    const SCENE_PAD: f32 = 8.0;
    const MAX_BUBBLE_WIDTH: f32 = 520.0;
    const MIN_BUBBLE_WIDTH: f32 = 40.0;

    let max_width = (bounds.width - SCENE_PAD * 2.0).clamp(MIN_BUBBLE_WIDTH, MAX_BUBBLE_WIDTH);
    let max_chars = ((max_width - h_padding * 2.0) / char_width)
        .floor()
        .max(8.0) as usize;
    let lines = bubble_display_lines(text, max_chars);
    let longest_line = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0) as f32;
    let width = (longest_line * char_width + h_padding * 2.0).max(MIN_BUBBLE_WIDTH);
    let height = v_padding * 2.0 + lines.len().max(1) as f32 * line_height;

    BubbleLayout {
        lines,
        width,
        height,
    }
}

fn bubble_display_lines(text: &str, max_chars: usize) -> Vec<String> {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();

    for word in compact.split(' ') {
        let current_len = current.chars().count();
        let word_len = word.chars().count();
        let next_len = if current.is_empty() {
            word_len
        } else {
            current_len + 1 + word_len
        };

        if !current.is_empty() && next_len > max_chars {
            lines.push(std::mem::take(&mut current));
        }

        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }

    if !current.is_empty() {
        lines.push(current);
    }

    lines
}

struct BubbleLayout {
    lines: Vec<String>,
    width: f32,
    height: f32,
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
    // the agent's head while they're running silently. Bobs on a 2 Hz
    // sine so it reads as active even if the underlying sprite is
    // idle-cycling.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Job;

    #[test]
    fn job_room_override_is_used_when_room_exists() {
        let rooms = crate::domain::room::default_rooms();
        let job = Job::cron("demo-cron");
        let mut overrides = HashMap::new();
        overrides.insert(job.id.clone(), "engine-room".to_string());

        assert_eq!(resolve_job_room(&job, &overrides, &rooms), "engine-room");
    }

    #[test]
    fn job_room_falls_back_when_override_is_missing_or_stale() {
        let rooms = crate::domain::room::default_rooms();
        let job = Job::cron("demo-cron");
        assert_eq!(resolve_job_room(&job, &HashMap::new(), &rooms), MAIN_ROOM);

        let mut overrides = HashMap::new();
        overrides.insert(job.id.clone(), "deleted-room".to_string());
        assert_eq!(resolve_job_room(&job, &overrides, &rooms), MAIN_ROOM);
    }

    #[test]
    fn bubble_display_lines_compact_and_wrap() {
        assert_eq!(
            bubble_display_lines("one\n\n two   three", 80),
            vec!["one two three".to_string()]
        );
        assert_eq!(
            bubble_display_lines("alpha beta gamma delta", 12),
            vec!["alpha beta".to_string(), "gamma delta".to_string()]
        );
    }

    #[test]
    fn initial_bubble_bottom_clears_sprite_top() {
        let anchor = Point::new(100.0, 200.0);
        let bubble_bottom = initial_bubble_bottom(anchor);
        let sprite_top = anchor.y - sprite::LOBSTER_SIZE.height / 2.0;

        assert!(bubble_bottom + BUBBLE_TAIL_LENGTH <= sprite_top - BUBBLE_SPRITE_CLEARANCE);
    }

    #[test]
    fn power_up_hides_when_agent_has_visible_bubble() {
        let agent = AgentId::new("sebastian");
        let running_rooms = HashSet::from(["main"]);
        let visible_bubble_agents = HashSet::from([agent.clone()]);

        assert!(!should_render_power_up(
            &agent,
            "main",
            &running_rooms,
            &visible_bubble_agents
        ));
    }

    #[test]
    fn power_up_shows_for_silent_running_agent() {
        let agent = AgentId::new("sebastian");
        let running_rooms = HashSet::from(["main"]);

        assert!(should_render_power_up(
            &agent,
            "main",
            &running_rooms,
            &HashSet::new()
        ));
    }
}
