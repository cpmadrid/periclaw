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

/// Space Lobster — PeriClaw's mascot sprite. Top-down view with two
/// raised claws, segmented body, fan tail, and three leg pairs on
/// each side. Three frames total:
///   - frame 0: relaxed — legs spread, tail open
///   - frame 1: step-left — legs shift left, claws wider
///   - frame 2: step-right — legs shift right, claws narrower
///
/// Cycling 0→1→0→2 at a slow Hz reads as a patient scuttle.
///
/// Palette roles:
///   X primary (shell color — green by default, operator-configurable)
///   x primary-dim (underbody shading)
///   W light (eye highlights, claw tip gleam)
///   K dark outline (eyes, joints)
///
/// Used for every agent and job sprite. Color is picked from the
/// agent's theme at draw time so the same template can paint a
/// green Sebastian or a red cron without duplicating the shape.
pub const LOBSTER: Sprite = Sprite {
    frames: &[
        // Frame 0 — idle / neutral pose
        &[
            "..K........K..",
            "..X........X..",
            "..X........X..",
            ".XXX......XXX.",
            "XXWXX.KK.XXWXX",
            "XXXXXXXXXXXXXX",
            ".XXXXXXXXXXXX.",
            "XX.XXXXXXXX.XX",
            "X..X..XX..X..X",
            "...XXXXXXXX...",
            "....xXXXXx....",
        ],
        // Frame 1 — step-left: legs slide left, left claw reaches
        &[
            ".K.........K..",
            ".X.........X..",
            ".X.........X..",
            "XXX.......XXX.",
            "XXWX..KK..XWXX",
            "XXXXXXXXXXXXXX",
            ".XXXXXXXXXXXX.",
            "XX.XXXXXXXX.XX",
            ".X..XX..X..X.X",
            "...XXXXXXXX...",
            "...xXXXXx.....",
        ],
        // Frame 2 — step-right: legs slide right, right claw reaches
        &[
            "..K.........K.",
            "..X.........X.",
            "..X.........X.",
            ".XXX.......XXX",
            "XXWX..KK..XWXX",
            "XXXXXXXXXXXXXX",
            ".XXXXXXXXXXXX.",
            "XX.XXXXXXXX.XX",
            "X.X..X..XX..X.",
            "...XXXXXXXX...",
            ".....xXXXXx...",
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

/// Pick the template for an agent. Main and cron both render as
/// space lobsters — the idea is that crons are jobs the main lobster
/// handles, so visually the office is a colony of lobsters at
/// different workstations. Channel providers still render as CRT
/// monitors until the agent/job split lands and channels move out
/// of the scene entirely.
pub fn sprite_for(agent: &Agent) -> &'static Sprite {
    match agent.kind {
        AgentKind::Main | AgentKind::Cron => &LOBSTER,
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
///
/// `flip_h` mirrors the frame horizontally — a lobster facing left
/// on its way back across the room without needing a second set of
/// frames.
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
    flip_h: bool,
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
    let sprite_w = sprite.width();
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
            // Mirror column index when flipping so the sprite faces
            // the opposite direction without an alternate frame.
            let draw_col = if flip_h {
                sprite_w.saturating_sub(1).saturating_sub(col_idx)
            } else {
                col_idx
            };
            // Draw each cell as a solid rectangle. Nearest-neighbor
            // comes for free with axis-aligned rects at integer-ish
            // positions — no filtering, no sub-pixel smear.
            let cell = canvas::Path::rectangle(
                Point::new(
                    origin.x + draw_col as f32 * scale,
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
            &LOBSTER,
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
        // lasts 0.5 s. The lobster is 3 frames, so t=0→frame 0,
        // t=0.5→frame 1, t=1.0→frame 2, t=1.5→frame 0 again.
        assert!(std::ptr::eq(
            LOBSTER.frame(0.0, 2.0).as_ptr(),
            LOBSTER.frames[0].as_ptr()
        ));
        assert!(std::ptr::eq(
            LOBSTER.frame(0.25, 2.0).as_ptr(),
            LOBSTER.frames[0].as_ptr()
        ));
        assert!(std::ptr::eq(
            LOBSTER.frame(0.5, 2.0).as_ptr(),
            LOBSTER.frames[1].as_ptr()
        ));
        assert!(std::ptr::eq(
            LOBSTER.frame(1.0, 2.0).as_ptr(),
            LOBSTER.frames[2].as_ptr()
        ));
        assert!(std::ptr::eq(
            LOBSTER.frame(1.5, 2.0).as_ptr(),
            LOBSTER.frames[0].as_ptr()
        ));
        // Hz = 0 pins frame 0 regardless of time (fully static
        // sprites like the monitor use this).
        assert!(std::ptr::eq(
            LOBSTER.frame(10.0, 0.0).as_ptr(),
            LOBSTER.frames[0].as_ptr()
        ));
    }

    #[test]
    fn sprite_size_scales() {
        let s = sprite_size_px(&LOBSTER, 4.0);
        assert_eq!(s.width, LOBSTER.width() as f32 * 4.0);
        assert_eq!(s.height, LOBSTER.height() as f32 * 4.0);
    }
}
