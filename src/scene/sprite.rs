//! Pixel-art sprites for Agent Office inhabitants.
//!
//! Sprites are tiny char-grids (`&'static str` per row). Each cell
//! maps to a semantic palette role so the actual color comes from the
//! agent's theme at draw time — one sprite template is reused for
//! every cron with a different hue, for every chat-agent persona, etc.
//!
//! Characters:
//! - `' '` or `'.'` — transparent
//! - `'X'` — primary color (agent-specific)
//! - `'x'` — primary dimmed (shadow / body edge)
//! - `'W'` — light (eye whites, highlights)
//! - `'K'` — dark outline (eye pupils, lineart)
//!
//! Sprites render nearest-neighbor style: each cell is a filled square
//! of `scale` pixels. No interpolation — that's the look.
//!
//! The grids are hand-designed for legibility at scale ≥ 2. Keep them
//! small (≤ 16 cells per side) so nothing crushes under the office
//! layout's available per-slot area.

use iced::widget::canvas;
use iced::{Color, Point, Size};

use crate::domain::{Agent, AgentKind};
use crate::ui::theme;

/// One animated sprite — 1+ frames of equal dimensions. A single-
/// frame sprite is a static pose; 2+ frames produce a simple walk /
/// idle cycle when cycled by clock phase.
pub struct Sprite {
    pub frames: &'static [&'static [&'static str]],
}

impl Sprite {
    pub fn width(&self) -> usize {
        self.frames
            .first()
            .and_then(|f| f.first())
            .map(|r| r.chars().count())
            .unwrap_or(0)
    }

    pub fn height(&self) -> usize {
        self.frames.first().map(|f| f.len()).unwrap_or(0)
    }

    /// Pick a frame for the current clock phase. Two-frame sprites
    /// alternate at `hz` Hz; single-frame sprites always return the
    /// only frame. A sprite's scenes-wide rhythm is set by the
    /// caller (running agents cycle faster than idle ones).
    pub fn frame(&self, seconds: f32, hz: f32) -> &'static [&'static str] {
        if self.frames.len() <= 1 || hz <= 0.0 {
            return self.frames[0];
        }
        let phase = (seconds * hz).rem_euclid(self.frames.len() as f32);
        let idx = phase as usize % self.frames.len();
        self.frames[idx]
    }
}

// =====================================================================
// Room-decor sprites.
//
// Static single-frame figures rendered into each room *before* the
// agent sprites, so the office reads as inhabited. Intentionally
// low-contrast (rendered in MUTED) so they recede behind the colorful
// agent sprites — think "set dressing," not "gameplay elements."
// =====================================================================

/// Telescope for the Observatory — tripod + barrel aimed skyward.
pub const TELESCOPE: Sprite = Sprite {
    frames: &[&[
        ".....XXX", "....XXX.", "...XXX..", "..XXX...", ".XXXX...", "XXXXX...", ".X.X.X..",
        "X..X..X.", "X..X..X.",
    ]],
};

/// Command HQ desk — a console with a couple of monitors.
pub const CONSOLE: Sprite = Sprite {
    frames: &[&[
        "XX.XX.XX.XXX",
        "XX.XX.XX.XXX",
        "XX.XX.XX.XXX",
        "XXXXXXXXXXXX",
        ".X........X.",
        ".XXXXXXXXXX.",
    ]],
};

/// Security filing cabinet — stacked drawers with handles.
pub const CABINET: Sprite = Sprite {
    frames: &[&[
        "XXXXXXXX", "X......X", "X.XXXX.X", "X......X", "XXXXXXXX", "X......X", "X.XXXX.X",
        "X......X", "XXXXXXXX", "X......X", "X.XXXX.X", "X......X", "XXXXXXXX",
    ]],
};

/// Research Lab beakers — two flasks on a shelf.
pub const BEAKERS: Sprite = Sprite {
    frames: &[&[
        ".XX..XXXX.",
        ".XX..X..X.",
        ".XX..X..X.",
        "XXXX.X..X.",
        "X..X.XXXX.",
        "X..X.XXXX.",
        "XXXX.XXXX.",
        "XXXX.XXXX.",
    ]],
};

/// Memory Vault — stacked archive boxes.
pub const ARCHIVE: Sprite = Sprite {
    frames: &[&[
        "XXXXXXXXXX",
        "X...XX...X",
        "XXXXXXXXXX",
        "..........",
        "XXXXXX....",
        "X....X....",
        "XXXXXX....",
        "....XXXXXX",
        "....X....X",
        "....XXXXXX",
    ]],
};

/// Studio microphone — condenser mic on a boom arm.
pub const MICROPHONE: Sprite = Sprite {
    frames: &[&[
        "..XXXX..", ".X.XX.X.", ".X.XX.X.", ".X.XX.X.", "..XXXX..", "...XX...", "...XX...",
        "..XXXX..", ".XXXXXX.",
    ]],
};

/// Classic Pac-Man ghost — dome with wavy bottom, big friendly eyes.
/// Two frames: the wavy bottom pattern shifts by one cell between
/// frames, so cycling them reads as the ghost "floating" across the
/// floor. Used for cron agents; the hue comes from each cron's
/// ghost-palette pick so five running crons look like a ghost posse.
pub const GHOST: Sprite = Sprite {
    frames: &[
        &[
            "....XXXXXX....",
            "..XXXXXXXXXX..",
            ".XXXXXXXXXXXX.",
            ".XXWWXXXXWWXX.",
            "XXXWKXXXXWKXXX",
            "XXXWWXXXXWWXXX",
            "XXXXXXXXXXXXXX",
            "XXXXXXXXXXXXXX",
            "XXXXXXXXXXXXXX",
            "XXXXXXXXXXXXXX",
            "XXXXXXXXXXXXXX",
            "XX.XX.XX.XX.XX",
            "X...X...X...X.",
        ],
        &[
            "....XXXXXX....",
            "..XXXXXXXXXX..",
            ".XXXXXXXXXXXX.",
            ".XXWWXXXXWWXX.",
            "XXXWKXXXXWKXXX",
            "XXXWWXXXXWWXXX",
            "XXXXXXXXXXXXXX",
            "XXXXXXXXXXXXXX",
            "XXXXXXXXXXXXXX",
            "XXXXXXXXXXXXXX",
            "XXXXXXXXXXXXXX",
            ".XX.XX.XX.XX.X",
            "X...X...X...X.",
        ],
    ],
};

/// Rounded humanoid — a head + body figure. Two frames:
/// - frame 0: legs even, arms relaxed
/// - frame 1: legs staggered, arms slightly bent — reads as a step
///
/// Used for `Main` (chat-capable) agents; the primary color is the
/// agent's signature hue so Sebastian reads as a bright-green
/// terminal figure, other personas in their own shade.
pub const HUMANOID: Sprite = Sprite {
    frames: &[
        &[
            "....XXXXXX....",
            "...XXXXXXXX...",
            "..XXXWKXXWKXX.",
            "..XXXWWXXWWXX.",
            "..XXXXXXXXXXX.",
            "...XXXXXXXXX..",
            "....XXXXXXX...",
            "..XXXXXXXXXXX.",
            ".XXXXXXXXXXXXX",
            "XXxxXXXXXXxxXX",
            "XXxxXXXXXXxxXX",
            ".xxxXXXXXXxxx.",
            "....XXXXXX....",
            "...XX....XX...",
            "...XX....XX...",
            "..xxx....xxx..",
        ],
        &[
            "....XXXXXX....",
            "...XXXXXXXX...",
            "..XXXWKXXWKXX.",
            "..XXXWWXXWWXX.",
            "..XXXXXXXXXXX.",
            "...XXXXXXXXX..",
            "....XXXXXXX...",
            "..XXXXXXXXXXX.",
            "XXXXXXXXXXXXXX",
            "Xxxx.XXXXxxxx.",
            "Xxxx.XXXXxxxx.",
            ".xxx.XXXX.xxx.",
            "....XXXXXX....",
            "...XX....XX...",
            "...XX.....XX..",
            "..xxx.....xxx.",
        ],
    ],
};

/// CRT monitor — screen + stand. Single-frame because it's a
/// machine; the screen "glow" is added at draw time as a scrolling
/// scanline effect so static data still feels alive.
pub const MONITOR: Sprite = Sprite {
    frames: &[&[
        "xxxxxxxxxxxxxx",
        "xXXXXXXXXXXXXx",
        "xXxxxxxxxxxxXx",
        "xXxXXXXXXXXxXx",
        "xXxXXXXXXXXxXx",
        "xXxXXXXXXXXxXx",
        "xXxXXXXXXXXxXx",
        "xXxxxxxxxxxxXx",
        "xXXXXXXXXXXXXx",
        "xxxxxxxxxxxxxx",
        "....xxXXxx....",
        "...xxxXXxxx...",
        "..xxxxxxxxxx..",
    ]],
};

/// Pick the decor sprite for a given room. Returned as `Option` so
/// callers can skip rooms we haven't designed decor for yet.
pub fn decor_for_room(room: crate::domain::RoomId) -> Option<&'static Sprite> {
    use crate::domain::RoomId;
    Some(match room {
        RoomId::Observatory => &TELESCOPE,
        RoomId::CommandHq => &CONSOLE,
        RoomId::Security => &CABINET,
        RoomId::ResearchLab => &BEAKERS,
        RoomId::MemoryVault => &ARCHIVE,
        RoomId::Studio => &MICROPHONE,
    })
}

/// Pick the template for an agent. Main agents get the humanoid,
/// crons get the ghost, channels get the monitor. Distinct silhouettes
/// at a glance — the operator can tell three crons + two channels
/// apart from two chat-capable agents without reading labels.
pub fn sprite_for(agent: &Agent) -> &'static Sprite {
    match agent.kind {
        AgentKind::Main => &HUMANOID,
        AgentKind::Cron => &GHOST,
        AgentKind::Channel => &MONITOR,
    }
}

/// Pick a reasonable scale (pixels per cell) for a sprite that fits
/// within the room slot. 3px per cell is the sweet spot — any larger
/// and a 16-wide sprite overruns neighboring slots; any smaller and
/// the eye details blur out.
pub const DEFAULT_SCALE: f32 = 3.0;

/// Rendered size of a sprite at the default scale, in canvas px.
/// Used by callers to offset text labels below the sprite without
/// overlapping the art.
pub fn sprite_size_px(sprite: &Sprite, scale: f32) -> Size {
    Size::new(
        sprite.width() as f32 * scale,
        sprite.height() as f32 * scale,
    )
}

/// Draw `sprite`'s current-phase frame centered on `center`, using
/// `primary` as the dominant color. Each `X` cell fills with
/// `primary`; `x` cells use a dimmed variant; `W` is a light
/// highlight; `K` is the outline / pupil. Transparent cells are
/// skipped (no fill at all).
///
/// `intensity` ∈ [0, 1] modulates alpha globally — disabled channels
/// render muted, running agents stay bright.
///
/// `seconds` and `frame_hz` select which animation frame to render;
/// the draw function itself is stateless so the caller owns the
/// clock — Running agents pass a higher `frame_hz`, idle ones pass
/// a slower one, and fully-static sprites pass `0.0`.
#[allow(clippy::too_many_arguments)]
pub fn draw_sprite_pixels(
    frame: &mut canvas::Frame,
    center: Point,
    sprite: &Sprite,
    scale: f32,
    primary: Color,
    intensity: f32,
    seconds: f32,
    frame_hz: f32,
) {
    let width = sprite.width() as f32 * scale;
    let height = sprite.height() as f32 * scale;
    let origin = Point::new(center.x - width / 2.0, center.y - height / 2.0);

    // Derived colors. We don't reach for the theme module per-cell
    // because these are cheap struct copies.
    let alpha = intensity.clamp(0.0, 1.0);
    let primary = with_alpha(primary, alpha);
    let dim = with_alpha(darken(primary, 0.55), alpha);
    let light = with_alpha(Color::from_rgb(0.95, 0.97, 0.92), alpha);
    let outline = with_alpha(*theme::SURFACE_0, alpha);

    let rows = sprite.frame(seconds, frame_hz);
    for (row_idx, row) in rows.iter().enumerate() {
        for (col_idx, ch) in row.chars().enumerate() {
            let color = match ch {
                'X' => Some(primary),
                'x' => Some(dim),
                'W' => Some(light),
                'K' => Some(outline),
                _ => None,
            };
            let Some(color) = color else { continue };
            // Draw each cell as a solid rectangle. Nearest-neighbor
            // comes for free with axis-aligned rects at integer-ish
            // positions — no filtering, no sub-pixel smear.
            let cell = canvas::Path::rectangle(
                Point::new(
                    origin.x + col_idx as f32 * scale,
                    origin.y + row_idx as f32 * scale,
                ),
                Size::new(scale, scale),
            );
            frame.fill(&cell, color);
        }
    }
}

/// Overlay a scrolling scanline on top of a sprite's screen area to
/// sell "CRT is on". Only the monitor sprite uses this; callers pick
/// which rows are "screen" (typically rows 3..8 of the monitor).
#[allow(clippy::too_many_arguments)]
pub fn draw_scanline(
    frame: &mut canvas::Frame,
    center: Point,
    sprite: &Sprite,
    scale: f32,
    screen_rows: std::ops::Range<usize>,
    screen_cols: std::ops::Range<usize>,
    seconds: f32,
    color: Color,
) {
    if screen_rows.is_empty() {
        return;
    }
    let width = sprite.width() as f32 * scale;
    let height = sprite.height() as f32 * scale;
    let origin = Point::new(center.x - width / 2.0, center.y - height / 2.0);

    // Row offset cycles through the screen rows at ~2Hz. Not so fast
    // it flickers, slow enough that the eye registers it as motion.
    let row_span = (screen_rows.end - screen_rows.start).max(1) as f32;
    let progress = (seconds * 2.0).rem_euclid(row_span);
    let row = screen_rows.start + (progress as usize);
    let line_origin = Point::new(
        origin.x + screen_cols.start as f32 * scale,
        origin.y + row as f32 * scale,
    );
    let line_size = Size::new(
        (screen_cols.end - screen_cols.start) as f32 * scale,
        scale * 0.5,
    );
    frame.fill(
        &canvas::Path::rectangle(line_origin, line_size),
        with_alpha(color, 0.35),
    );
}

fn with_alpha(color: Color, a: f32) -> Color {
    Color {
        a: color.a * a,
        ..color
    }
}

/// Produce a darker variant by blending toward black. `t=0` keeps
/// the color, `t=1` turns it black. Cheap per-channel lerp.
fn darken(color: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color {
        r: color.r * (1.0 - t),
        g: color.g * (1.0 - t),
        b: color.b * (1.0 - t),
        a: color.a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_sprites_are_rectangular() {
        // Every row across every frame must have the same width and
        // frame height must match. A ragged row would mis-align the
        // render; a short frame would pop the sprite shorter on
        // alternate ticks.
        for sprite in [
            &GHOST,
            &HUMANOID,
            &MONITOR,
            &TELESCOPE,
            &CONSOLE,
            &CABINET,
            &BEAKERS,
            &ARCHIVE,
            &MICROPHONE,
        ] {
            let w = sprite.width();
            let h = sprite.height();
            assert!(w > 0 && h > 0, "sprite has zero dimensions");
            for (f_idx, frame) in sprite.frames.iter().enumerate() {
                assert_eq!(frame.len(), h, "frame {f_idx} height mismatch");
                for (r_idx, row) in frame.iter().enumerate() {
                    assert_eq!(
                        row.chars().count(),
                        w,
                        "frame {f_idx} row {r_idx} width mismatch: expected {w}, got {}",
                        row.chars().count(),
                    );
                }
            }
        }
    }

    #[test]
    fn frame_phase_cycles_through_all_frames() {
        // `hz` reads as "frames per second" — at 2 Hz, each frame
        // lasts 0.5 s, so a 2-frame sprite is on frame 0 during
        // [0, 0.5) and on frame 1 during [0.5, 1.0), then wraps.
        assert!(std::ptr::eq(
            GHOST.frame(0.0, 2.0).as_ptr(),
            GHOST.frames[0].as_ptr()
        ));
        assert!(std::ptr::eq(
            GHOST.frame(0.25, 2.0).as_ptr(),
            GHOST.frames[0].as_ptr()
        ));
        assert!(std::ptr::eq(
            GHOST.frame(0.5, 2.0).as_ptr(),
            GHOST.frames[1].as_ptr()
        ));
        assert!(std::ptr::eq(
            GHOST.frame(1.0, 2.0).as_ptr(),
            GHOST.frames[0].as_ptr()
        ));
        // Hz = 0 pins frame 0 regardless of time (fully static
        // sprites like the monitor use this).
        assert!(std::ptr::eq(
            GHOST.frame(10.0, 0.0).as_ptr(),
            GHOST.frames[0].as_ptr()
        ));
    }

    #[test]
    fn sprite_size_scales() {
        let s = sprite_size_px(&GHOST, 4.0);
        assert_eq!(s.width, GHOST.width() as f32 * 4.0);
        assert_eq!(s.height, GHOST.height() as f32 * 4.0);
    }
}
