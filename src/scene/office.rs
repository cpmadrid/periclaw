//! Canvas Program rendering the Agent Office.
//!
//! Draws the 3×2 grid of rooms with labels, then places one colored
//! circle per agent at its assigned slot. Real sprites (PNG atlas
//! with nearest-neighbor filtering) land in M4.

use std::collections::HashMap;
use std::time::Instant;

use iced::mouse;
use iced::widget::canvas::{self, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

use crate::Message;
use crate::domain::{Agent, AgentId, AgentStatus, RoomId, room_for};
use crate::scene::{RoomLayout, ThoughtBubble};
use crate::ui::theme;

/// Snapshot of what to draw. Cheap to clone; recreated each `view()`.
pub struct OfficeScene<'a> {
    pub roster: &'a [Agent],
    pub statuses: &'a HashMap<AgentId, AgentStatus>,
    pub bubbles: &'a [ThoughtBubble],
    /// Instant of each agent's most recent status change, used to
    /// drive the ring-pulse flash.
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

            let now = Instant::now();
            let seconds = clock_phase(now);
            let mut sprite_positions: HashMap<AgentId, Point> = HashMap::new();
            for (room, agents) in per_room {
                let room_rect = layout.room_rect(room);
                for (idx, agent) in agents.iter().enumerate() {
                    let base_pos = layout.sprite_slot(room, idx);
                    let status = self
                        .statuses
                        .get(&agent.id)
                        .copied()
                        .unwrap_or(AgentStatus::Unknown);
                    let flash = self
                        .transition_moments
                        .get(&agent.id)
                        .map(|t| transition_flash(now.saturating_duration_since(*t)))
                        .unwrap_or(0.0);
                    // Ok/Unknown agents slow-wander around their home
                    // slot. Running adds a vertical bob on top of the
                    // base position (stays where they're working).
                    // Errored/Disabled stay anchored to their slot.
                    let (wander, facing_left) =
                        wander_offset(&agent.id, &room_rect, seconds, status);
                    let wander_pos = Point::new(base_pos.x + wander.x, base_pos.y + wander.y);
                    let pos = animated_position(wander_pos, status, now);
                    draw_sprite(frame, pos, agent, status, flash, seconds, facing_left);
                    // Bubble anchor stays on the wander-adjusted
                    // position but not the bob — the tail shouldn't
                    // whip around while the sprite's bouncing.
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
        });

        vec![geometry]
    }
}

fn draw_room(frame: &mut canvas::Frame, layout: &RoomLayout, room: RoomId) {
    use crate::scene::sprite::{self, DEFAULT_SCALE};

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

    // Decor — one themed furniture piece per room, rendered in the
    // muted palette so it reads as scenery and agent sprites remain
    // the focal point. Positioned in the upper-right corner so it
    // doesn't collide with either the room label (upper-left) or
    // the sprite slots (lower half).
    if let Some(decor) = sprite::decor_for_room(room) {
        let size = sprite::sprite_size_px(decor, DEFAULT_SCALE);
        // Center of the upper-right quadrant. `+12` / `-12` give the
        // sprite a small margin from the room edge so it doesn't
        // touch the border stroke.
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
    // Approximate width for monospace at 11pt.
    let width = (text.len() as f32 * 6.5).max(40.0) + 12.0;
    let height = 20.0;
    let bubble_origin = Point::new(anchor.x - width / 2.0, anchor.y - 42.0);

    // Border/text hue per bubble kind so the operator can read the
    // office at a glance: tool calls stand out in amber, chat/status
    // share the signature terminal green.
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

    // Little tail pointing down to the sprite.
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
fn draw_sprite(
    frame: &mut canvas::Frame,
    pos: Point,
    agent: &Agent,
    status: AgentStatus,
    flash: f32,
    seconds: f32,
    flip_h: bool,
) {
    use crate::domain::AgentKind;
    use crate::scene::sprite::{self, DEFAULT_SCALE, LOBSTER_SIZE};

    let color: Color = agent.color();

    // Flash halo size depends on which sprite path we take, since
    // lobsters render at a fixed pixel size and char-grid sprites
    // scale with their template dimensions.
    let halo_size = match agent.kind {
        AgentKind::Main | AgentKind::Cron => LOBSTER_SIZE,
        AgentKind::Channel => sprite::sprite_size_px(&sprite::MONITOR, DEFAULT_SCALE),
    };

    // Transition-flash halo. A brief, widening rectangle behind the
    // sprite briefly brightens the slot when status changes —
    // replaces the old ring-pulse, scaled to the sprite box so the
    // effect frames the whole figure instead of a circle.
    if flash > 0.0 {
        let pad = 6.0 + 4.0 * flash;
        let halo_origin = Point::new(
            pos.x - halo_size.width / 2.0 - pad,
            pos.y - halo_size.height / 2.0 - pad,
        );
        let halo_rect = Size::new(halo_size.width + pad * 2.0, halo_size.height + pad * 2.0);
        let halo = Path::rectangle(halo_origin, halo_rect);
        frame.stroke(
            &halo,
            Stroke::default()
                .with_color(Color {
                    a: 0.25 + 0.55 * flash,
                    ..color
                })
                .with_width(2.0),
        );
    }

    match agent.kind {
        AgentKind::Main | AgentKind::Cron => {
            // Image-based lobster. Per-frame color tinting isn't
            // applied — the sprite artwork is its own identity; the
            // existing per-agent color is used only for the halo
            // flash above. Operator-visible color customization
            // comes back when we ship multiple sheet variants.
            sprite::draw_lobster(frame, pos, status, seconds, flip_h);
        }
        AgentKind::Channel => {
            let template = &sprite::MONITOR;
            let intensity = match status {
                AgentStatus::Disabled => 0.45,
                AgentStatus::Unknown => 0.75,
                _ => 1.0,
            };
            sprite::draw_sprite_pixels(
                frame,
                pos,
                template,
                DEFAULT_SCALE,
                color,
                intensity,
                seconds,
                0.0,
                flip_h,
            );
            // Scrolling scanline while connected — disabled stays
            // dark.
            if !matches!(status, AgentStatus::Disabled) {
                sprite::draw_scanline(
                    frame,
                    pos,
                    template,
                    DEFAULT_SCALE,
                    3..8,
                    3..12,
                    seconds,
                    color,
                );
            }
        }
    }
    // Re-compute size for label offset below.
    let size = halo_size;

    // Name tag under the sprite. We approximate "centered" by offsetting
    // by half the expected text width (monospace ≈ 6px per char @ 11pt).
    // Tag sits below the rendered pixel grid, not the static center.
    let approx_width = (agent.display.len() as f32) * 6.0;
    let label_y = pos.y + size.height / 2.0 + 6.0;
    frame.fill_text(Text {
        content: agent.display.to_string(),
        position: Point::new(pos.x - approx_width / 2.0, label_y),
        color: *theme::FOREGROUND,
        size: 11.0.into(),
        font: iced::Font::MONOSPACE,
        ..Text::default()
    });
}

/// Slow wander pattern — each agent drifts within a soft bounding
/// box around its home slot via two orthogonal sines at slightly
/// different periods, keyed by a hash of the agent id so siblings
/// don't move in lockstep. Disabled and Errored agents stay still
/// (they shouldn't look like they're idling around); Running stays
/// put too (they're "working at their station" and the bob reads
/// as busy). Only Ok / Unknown wander.
///
/// Returns the (x, y) offset from base plus a `facing_left` flag
/// derived from the horizontal-velocity sign so the sprite flips
/// when moving right-to-left.
fn wander_offset(
    agent_id: &AgentId,
    room_rect: &Rectangle,
    seconds: f32,
    status: AgentStatus,
) -> (Point, bool) {
    if !matches!(status, AgentStatus::Ok | AgentStatus::Unknown) {
        return (Point::ORIGIN, false);
    }
    // Keep wander bounds comfortably inside the room so sprites
    // don't clip the border. `amp_x` caps around a third of the
    // half-width; `amp_y` is tighter so sprites stay in the lower
    // half where the slots already sit.
    let amp_x = (room_rect.width * 0.18).clamp(8.0, 60.0);
    let amp_y = (room_rect.height * 0.08).clamp(4.0, 24.0);

    // Stable hash of the id — spreads sprites' phase offsets so
    // two lobsters in the same room don't sine-wave in unison.
    let mut h: u64 = 5381;
    for b in agent_id.as_str().bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    let phase_x = (h % 360) as f32 * std::f32::consts::PI / 180.0;
    let phase_y = ((h / 7) % 360) as f32 * std::f32::consts::PI / 180.0;
    // Speed ∈ [0.10, 0.28] rad/sec — intentionally slow. Different
    // per-agent so the office reads as organic rather than a clock
    // tick.
    let speed = 0.10 + ((h % 40) as f32 * 0.0045);

    let t = seconds * speed;
    let offset_x = (t + phase_x).sin() * amp_x;
    // Y moves slower and with a different period so the path
    // traces a wandering Lissajous rather than a straight diagonal.
    let offset_y = (t * 0.73 + phase_y).cos() * amp_y;
    // Facing: the horizontal velocity at this instant. Cosine of
    // the x-phase is the derivative's sign.
    let facing_left = (t + phase_x).cos() < 0.0;
    (Point::new(offset_x, offset_y), facing_left)
}

/// Running sprites bob subtly to mark them as busy. Other states
/// draw at their resting slot.
fn animated_position(base: Point, status: AgentStatus, now: Instant) -> Point {
    if !matches!(status, AgentStatus::Running) {
        return base;
    }
    // ~1.5 Hz sine, ±3 px amplitude. Plenty of signal, not frantic.
    const BOB_AMPLITUDE_PX: f32 = 3.0;
    const BOB_HZ: f32 = 1.5;
    let t = clock_phase(now);
    let offset = BOB_AMPLITUDE_PX * (t * std::f32::consts::TAU * BOB_HZ).sin();
    Point::new(base.x, base.y + offset)
}

/// Map an `Instant` to a repeating seconds-since-some-epoch value.
/// Using wall-clock would tie every sprite's phase to the current
/// time of day; instead we anchor to the process's first call so
/// animations start at phase 0 when the scene first paints.
fn clock_phase(now: Instant) -> f32 {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let epoch = *EPOCH.get_or_init(|| now);
    now.saturating_duration_since(epoch).as_secs_f32()
}

/// Convert elapsed-since-transition duration into a 0..1 flash
/// intensity that rises fast then fades. Matches
/// `app::TRANSITION_FLASH` for the total envelope.
fn transition_flash(age: std::time::Duration) -> f32 {
    const FLASH_MS: f32 = 600.0;
    let elapsed_ms = age.as_millis() as f32;
    if elapsed_ms >= FLASH_MS {
        return 0.0;
    }
    // Quick attack, slow decay — eye-catch without a hard cut.
    let t = elapsed_ms / FLASH_MS; // 0..1
    (1.0 - t).powi(2)
}
