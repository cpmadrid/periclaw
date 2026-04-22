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

use std::sync::LazyLock;

use iced::advanced::image as iced_image;
use iced::widget::canvas;
use iced::{Color, Point, Rectangle, Size, Vector};

use crate::domain::AgentStatus;
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

// =====================================================================
// Space Lobster sprite — image-based.
//
// The pixel-art char-grid approach worked for decor but didn't have
// the fidelity for the mascot. The lobster now loads from a bundled
// PNG sprite sheet that the operator can swap / re-theme per release.
//
// Frames live at `assets/lobster/{idle,walk}-{0..3}.png` — four each
// for IDLE and WALK. They're embedded with `include_bytes!` so a
// release binary remains a single file (no asset-path resolution at
// runtime). Decoded once via `LazyLock` — Iced caches the GPU
// texture by `Handle` identity, so cloning the Handle on every frame
// redraw is cheap.
//
// Rendering goes through `draw_lobster` (canvas image + nearest-
// neighbor filter) instead of `draw_sprite_pixels` (per-cell rect
// fill). The two paths coexist: decor and MONITOR still use the
// char-grid renderer; lobsters use image.
// =====================================================================

static LOBSTER_IDLE: LazyLock<[iced_image::Handle; 4]> = LazyLock::new(|| {
    [
        iced_image::Handle::from_bytes(
            include_bytes!("../../assets/lobster/idle-0.png").as_slice(),
        ),
        iced_image::Handle::from_bytes(
            include_bytes!("../../assets/lobster/idle-1.png").as_slice(),
        ),
        iced_image::Handle::from_bytes(
            include_bytes!("../../assets/lobster/idle-2.png").as_slice(),
        ),
        iced_image::Handle::from_bytes(
            include_bytes!("../../assets/lobster/idle-3.png").as_slice(),
        ),
    ]
});

static LOBSTER_WALK: LazyLock<[iced_image::Handle; 4]> = LazyLock::new(|| {
    [
        iced_image::Handle::from_bytes(
            include_bytes!("../../assets/lobster/walk-0.png").as_slice(),
        ),
        iced_image::Handle::from_bytes(
            include_bytes!("../../assets/lobster/walk-1.png").as_slice(),
        ),
        iced_image::Handle::from_bytes(
            include_bytes!("../../assets/lobster/walk-2.png").as_slice(),
        ),
        iced_image::Handle::from_bytes(
            include_bytes!("../../assets/lobster/walk-3.png").as_slice(),
        ),
    ]
});

/// Target render size on the canvas for a lobster sprite. Roughly
/// matches the old char-grid's on-screen footprint (≈45 px wide
/// at 3× scale) with a little extra vertical room for the fan tail.
pub const LOBSTER_SIZE: Size = Size {
    width: 54.0,
    height: 80.0,
};

/// Draw a lobster centered on `center`, picking the frame based on
/// status and clock phase. Running/Unknown agents cycle the WALK
/// frames; Ok agents cycle IDLE; Disabled/Errored pin to the first
/// idle frame (still visible, just not animating).
///
/// `flip_h` mirrors the sprite horizontally via a canvas transform
/// so a lobster can face either direction without per-direction
/// frames.
pub fn draw_lobster(
    frame: &mut canvas::Frame,
    center: Point,
    status: AgentStatus,
    seconds: f32,
    flip_h: bool,
) {
    let (frames, hz): (&[iced_image::Handle; 4], f32) = match status {
        AgentStatus::Running | AgentStatus::Unknown => (&LOBSTER_WALK, 4.0),
        AgentStatus::Ok => (&LOBSTER_IDLE, 2.0),
        AgentStatus::Error | AgentStatus::Disabled => (&LOBSTER_IDLE, 0.0),
    };
    let idx = if hz <= 0.0 {
        0
    } else {
        let phase = (seconds * hz).rem_euclid(frames.len() as f32) as usize;
        phase.min(frames.len() - 1)
    };
    let handle = frames[idx].clone();

    let bounds = Rectangle::new(
        Point::new(
            center.x - LOBSTER_SIZE.width / 2.0,
            center.y - LOBSTER_SIZE.height / 2.0,
        ),
        LOBSTER_SIZE,
    );
    let image = iced_image::Image::new(handle).filter_method(iced_image::FilterMethod::Nearest);

    if flip_h {
        // Horizontal mirror: translate to sprite center, scale X by
        // -1, translate back, then draw. `with_save` auto-restores
        // the transform on close so subsequent sprites aren't
        // double-flipped.
        frame.with_save(|f| {
            f.translate(Vector::new(center.x, 0.0));
            f.scale_nonuniform(Vector::new(-1.0, 1.0));
            f.translate(Vector::new(-center.x, 0.0));
            f.draw_image(bounds, image);
        });
    } else {
        frame.draw_image(bounds, image);
    }
}

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
    fn char_grid_sprites_are_rectangular() {
        // Every row across every frame must have the same width and
        // frame height must match. A ragged row would mis-align the
        // render; a short frame would pop the sprite shorter on
        // alternate ticks. Applies to the remaining char-grid
        // sprites (decor + MONITOR) — LOBSTER lives in image form
        // now and is verified by the runtime PNG-decode path.
        for sprite in [
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
    fn sprite_size_scales() {
        let s = sprite_size_px(&MONITOR, 4.0);
        assert_eq!(s.width, MONITOR.width() as f32 * 4.0);
        assert_eq!(s.height, MONITOR.height() as f32 * 4.0);
    }
}
