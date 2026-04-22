//! Canvas Program rendering the Agent Office.
//!
//! Takes a configured list of rooms and the current roster + jobs map,
//! slots agents and jobs into rooms, and draws their sprites. Room
//! layout is driven by `RoomLayout`, which packs an arbitrary count
//! into the best-fitting grid; room↔entity assignment comes from a
//! mix of per-job preferences (`job_rooms`) and legacy-compat fallbacks
//! in [`crate::domain::room`].

use std::collections::HashMap;
use std::time::Instant;

use iced::mouse;
use iced::widget::canvas::{self, Path, Stroke, Text};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

use crate::Message;
use crate::domain::job::JobKind;
use crate::domain::room::{
    self, CHANNEL_DISABLED_ROOM, CHANNEL_OK_ROOM, ERROR_ROOM, MAIN_ROOM, RUNNING_ROOM,
};
use crate::domain::{Agent, AgentId, AgentStatus, Job, JobId, Room};
use crate::scene::{Flourish, RoomLayout, ThoughtBubble, sprite};
use crate::ui::theme;

/// Snapshot of what to draw. Cheap to clone; recreated each `view()`.
pub struct OfficeScene<'a> {
    pub roster: &'a [Agent],
    pub jobs: &'a HashMap<JobId, Job>,
    pub statuses: &'a HashMap<AgentId, AgentStatus>,
    pub rooms: &'a [Room],
    pub job_rooms: &'a HashMap<JobId, String>,
    pub bubbles: &'a [ThoughtBubble],
    pub flourishes: &'a [Flourish],
    /// Instant of each agent's most recent status change, used to
    /// drive the ring-pulse flash.
    pub transition_moments: &'a HashMap<AgentId, Instant>,
    pub job_transition_moments: &'a HashMap<JobId, Instant>,
    pub cache: &'a canvas::Cache,
}

/// One drawable entity inside a room — abstracts agent vs job so the
/// per-room slot loop doesn't have to branch twice.
enum Drawable<'a> {
    Agent(&'a Agent),
    Job(&'a Job),
}

impl Drawable<'_> {
    fn display(&self) -> &str {
        match self {
            Drawable::Agent(a) => a.display.as_str(),
            Drawable::Job(j) => j.display.as_str(),
        }
    }

    fn color(&self) -> Color {
        match self {
            Drawable::Agent(a) => a.color(),
            Drawable::Job(j) => j.color(),
        }
    }

    /// A stable string id used for wander-phase hashing. Agent ids
    /// and job ids live in different type namespaces but the string
    /// form is unique across the union (agents come from
    /// `agents.list`, jobs from cron/channel snapshots).
    fn phase_key(&self) -> &str {
        match self {
            Drawable::Agent(a) => a.id.as_str(),
            Drawable::Job(j) => j.id.as_str(),
        }
    }
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

            // Group drawables by room id.
            let mut per_room: HashMap<&str, Vec<(Drawable<'_>, AgentStatus)>> = HashMap::new();
            for agent in self.roster {
                let status = self
                    .statuses
                    .get(&agent.id)
                    .copied()
                    .unwrap_or(AgentStatus::Unknown);
                let room_id = resolve_agent_room(agent, status);
                per_room
                    .entry(room_id)
                    .or_default()
                    .push((Drawable::Agent(agent), status));
            }
            for job in self.jobs.values() {
                let room_id = resolve_job_room(job, self.job_rooms);
                per_room
                    .entry(room_id)
                    .or_default()
                    .push((Drawable::Job(job), job.status));
            }

            let now = Instant::now();
            let seconds = clock_phase(now);
            let mut sprite_positions: HashMap<String, Point> = HashMap::new();

            for (room_id, entries) in per_room {
                let Some(room_rect) = layout.room_rect(room_id) else {
                    // Room referenced by a job but not in the configured
                    // list — drop so the sprite doesn't appear outside
                    // any panel. This is the "operator removed a room
                    // that jobs were assigned to" case; we fall back to
                    // whatever the first room is next tick once
                    // `resolve_job_room` re-computes.
                    continue;
                };
                for (idx, (drawable, status)) in entries.iter().enumerate() {
                    let base_pos = layout.sprite_slot(room_rect, idx);
                    let flash = transition_moment_for(drawable, self)
                        .map(|t| transition_flash(now.saturating_duration_since(t)))
                        .unwrap_or(0.0);
                    let sprite_half = sprite_half_size(drawable);
                    let (wander, facing_left) = wander_offset(
                        drawable.phase_key(),
                        &room_rect,
                        base_pos,
                        sprite_half,
                        seconds,
                        *status,
                    );
                    let wander_pos = Point::new(base_pos.x + wander.x, base_pos.y + wander.y);
                    let pos = animated_position(wander_pos, *status, now);
                    draw_drawable(frame, pos, drawable, *status, flash, seconds, facing_left);
                    sprite_positions.insert(drawable.phase_key().to_string(), wander_pos);
                }
            }

            // Work flourishes — expanding ring on top of the job's
            // sprite position. Skipped when the job isn't in the scene
            // (e.g. its room was removed).
            for flourish in self.flourishes {
                let Some(anchor) = sprite_positions.get(flourish.job_id.as_str()).copied() else {
                    continue;
                };
                draw_flourish(frame, anchor, flourish.progress(now));
            }

            for bubble in self.bubbles {
                let Some(alpha) = bubble.alpha(now) else {
                    continue;
                };
                let Some(anchor) = sprite_positions.get(bubble.agent.as_str()).copied() else {
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

fn transition_moment_for(drawable: &Drawable<'_>, scene: &OfficeScene<'_>) -> Option<Instant> {
    match drawable {
        Drawable::Agent(a) => scene.transition_moments.get(&a.id).copied(),
        Drawable::Job(j) => scene.job_transition_moments.get(&j.id).copied(),
    }
}

/// Where a chat agent sprite lives. Errored agents jump to the alert
/// room; everything else stays in the main room.
fn resolve_agent_room(_agent: &Agent, status: AgentStatus) -> &'static str {
    if matches!(status, AgentStatus::Error) {
        ERROR_ROOM
    } else {
        MAIN_ROOM
    }
}

/// Where a job sprite lives this frame. Job→room preference wins when
/// present; otherwise we fall back to the legacy thematic mapping so
/// operators who haven't customized anything keep their existing scene.
fn resolve_job_room<'a>(job: &'a Job, overrides: &'a HashMap<JobId, String>) -> &'a str {
    // Error wins unconditionally — red-alert room regardless of kind.
    if matches!(job.status, AgentStatus::Error) {
        return ERROR_ROOM;
    }
    match (job.kind, job.status) {
        // Running crons converge on the common work room so "who's
        // working right now?" is a single glance.
        (JobKind::Cron, AgentStatus::Running) => RUNNING_ROOM,
        (JobKind::Cron, _) => {
            if let Some(id) = overrides.get(&job.id) {
                id.as_str()
            } else {
                room::default_cron_room(job.id.as_str())
            }
        }
        (JobKind::Channel, AgentStatus::Disabled) => CHANNEL_DISABLED_ROOM,
        (JobKind::Channel, _) => {
            if let Some(id) = overrides.get(&job.id) {
                id.as_str()
            } else {
                CHANNEL_OK_ROOM
            }
        }
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

fn draw_flourish(frame: &mut canvas::Frame, anchor: Point, progress: f32) {
    // Expanding ring fading to transparent — subtle enough not to
    // steal the scene, visible enough to signal "job fired."
    let max_radius = 38.0;
    let radius = 8.0 + max_radius * progress;
    let alpha = (1.0 - progress).powi(2);
    let origin = Point::new(anchor.x - radius, anchor.y - radius);
    let ring = Path::circle(Point::new(anchor.x, anchor.y), radius);
    // `Path::circle` creates a filled disk path — use stroke with a
    // fading alpha so we get a ring not a disk. Ignoring `origin` here;
    // the calc above would matter for a rectangular ring.
    let _ = origin;
    frame.stroke(
        &ring,
        Stroke::default()
            .with_color(Color {
                a: alpha * 0.85,
                ..*theme::TERMINAL_GREEN
            })
            .with_width(2.0),
    );
}

fn draw_drawable(
    frame: &mut canvas::Frame,
    pos: Point,
    drawable: &Drawable<'_>,
    status: AgentStatus,
    flash: f32,
    seconds: f32,
    flip_h: bool,
) {
    use crate::scene::sprite::{self, DEFAULT_SCALE, LOBSTER_SIZE};

    let color = drawable.color();

    let halo_size = match drawable {
        Drawable::Agent(_) => LOBSTER_SIZE,
        Drawable::Job(j) => match j.kind {
            JobKind::Cron => LOBSTER_SIZE,
            JobKind::Channel => sprite::sprite_size_px(&sprite::MONITOR, DEFAULT_SCALE),
        },
    };

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

    match drawable {
        Drawable::Agent(_) => {
            sprite::draw_lobster(frame, pos, status, seconds, flip_h);
        }
        Drawable::Job(j) => match j.kind {
            JobKind::Cron => {
                sprite::draw_lobster(frame, pos, status, seconds, flip_h);
            }
            JobKind::Channel => {
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
        },
    }

    let label = drawable.display();
    let approx_width = (label.len() as f32) * 6.0;
    let label_y = pos.y + halo_size.height / 2.0 + 6.0;
    frame.fill_text(Text {
        content: label.to_string(),
        position: Point::new(pos.x - approx_width / 2.0, label_y),
        color: *theme::FOREGROUND,
        size: 11.0.into(),
        font: iced::Font::MONOSPACE,
        ..Text::default()
    });
}

fn sprite_half_size(drawable: &Drawable<'_>) -> Size {
    use crate::scene::sprite::{self, DEFAULT_SCALE, LOBSTER_SIZE};
    let full = match drawable {
        Drawable::Agent(_) => LOBSTER_SIZE,
        Drawable::Job(j) => match j.kind {
            JobKind::Cron => LOBSTER_SIZE,
            JobKind::Channel => sprite::sprite_size_px(&sprite::MONITOR, DEFAULT_SCALE),
        },
    };
    Size::new(full.width / 2.0, full.height / 2.0)
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
