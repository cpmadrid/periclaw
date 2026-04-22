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
use iced::{Color, Point, Rectangle, Size};

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

/// Decode a PNG at runtime and return a Handle built from raw RGBA.
/// Going through `image` + `from_rgba` (instead of `from_bytes`) lets
/// us mirror pixel data in RAM before handing it to iced — that's how
/// the flipped-handle arrays are built without relying on the canvas
/// transform stack. Panics on malformed input, which is fine: the
/// PNGs are baked in via `include_bytes!`, so a decode failure means
/// a broken checkout, not a runtime error path.
fn decode_rgba(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let img = image::load_from_memory(bytes)
        .expect("embedded lobster PNG decodes")
        .to_rgba8();
    let (w, h) = img.dimensions();
    (w, h, img.into_raw())
}

/// Mirror RGBA pixels left-right in place. Why bake a second handle
/// instead of flipping at draw time: iced 0.14's canvas image path
/// does not honor a `scale_nonuniform(-1, 1)` on the transform stack
/// reliably — the negative X scale flips the quad's winding and the
/// backend ends up rendering the texture inverted on Y (the upside-
/// down lobsters). Pre-flipping in RAM produces an identical-size
/// RGBA buffer with columns reversed; drawing it with an unmodified
/// transform stack renders as a clean horizontal mirror, no matter
/// what the backend does with negative scales.
fn mirror_horizontal(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let stride = w * 4;
    let mut out = vec![0u8; rgba.len()];
    for y in 0..h {
        let row_start = y * stride;
        for x in 0..w {
            let src = row_start + (w - 1 - x) * 4;
            let dst = row_start + x * 4;
            out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    out
}

/// Construct the unflipped + flipped handle pair for one PNG.
fn lobster_handle_pair(bytes: &[u8]) -> (iced_image::Handle, iced_image::Handle) {
    let (w, h, rgba) = decode_rgba(bytes);
    let flipped_rgba = mirror_horizontal(w, h, &rgba);
    (
        iced_image::Handle::from_rgba(w, h, rgba),
        iced_image::Handle::from_rgba(w, h, flipped_rgba),
    )
}

/// Paired (unflipped, flipped) handle arrays for one pose set. We
/// store them as a tuple inside a single LazyLock so each PNG is
/// decoded exactly once at first use and the two handle arrays stay
/// in lockstep (frame i unflipped and frame i flipped come from the
/// same raw RGBA buffer).
type IdleHandles = ([iced_image::Handle; 7], [iced_image::Handle; 7]);
type WalkHandles = ([iced_image::Handle; 8], [iced_image::Handle; 8]);

static LOBSTER_IDLE_HANDLES: LazyLock<IdleHandles> = LazyLock::new(|| {
    let p0 = lobster_handle_pair(include_bytes!("../../assets/lobster/idle-0.png"));
    let p1 = lobster_handle_pair(include_bytes!("../../assets/lobster/idle-1.png"));
    let p2 = lobster_handle_pair(include_bytes!("../../assets/lobster/idle-2.png"));
    let p3 = lobster_handle_pair(include_bytes!("../../assets/lobster/idle-3.png"));
    let p4 = lobster_handle_pair(include_bytes!("../../assets/lobster/idle-4.png"));
    let p5 = lobster_handle_pair(include_bytes!("../../assets/lobster/idle-5.png"));
    let p6 = lobster_handle_pair(include_bytes!("../../assets/lobster/idle-6.png"));
    (
        [p0.0, p1.0, p2.0, p3.0, p4.0, p5.0, p6.0],
        [p0.1, p1.1, p2.1, p3.1, p4.1, p5.1, p6.1],
    )
});

static LOBSTER_WALK_HANDLES: LazyLock<WalkHandles> = LazyLock::new(|| {
    let p0 = lobster_handle_pair(include_bytes!("../../assets/lobster/walk-0.png"));
    let p1 = lobster_handle_pair(include_bytes!("../../assets/lobster/walk-1.png"));
    let p2 = lobster_handle_pair(include_bytes!("../../assets/lobster/walk-2.png"));
    let p3 = lobster_handle_pair(include_bytes!("../../assets/lobster/walk-3.png"));
    let p4 = lobster_handle_pair(include_bytes!("../../assets/lobster/walk-4.png"));
    let p5 = lobster_handle_pair(include_bytes!("../../assets/lobster/walk-5.png"));
    let p6 = lobster_handle_pair(include_bytes!("../../assets/lobster/walk-6.png"));
    let p7 = lobster_handle_pair(include_bytes!("../../assets/lobster/walk-7.png"));
    (
        [p0.0, p1.0, p2.0, p3.0, p4.0, p5.0, p6.0, p7.0],
        [p0.1, p1.1, p2.1, p3.1, p4.1, p5.1, p6.1, p7.1],
    )
});

/// Target render size. Deliberately set to **exactly half** the
/// source resolution (140/2, 170/2) so the downsample is a clean
/// 2× nearest-neighbor reduction — every target pixel samples a
/// fixed 2×2 block of source texels, identically on every frame.
///
/// Why the exact integer ratio matters: at an arbitrary ratio like
/// 66/140 ≈ 0.471, `FilterMethod::Nearest` picks a different source
/// texel whenever the target rect's origin shifts by a fraction
/// of a pixel (wander offset, bob, etc.). Even with `snap(true)`
/// on the Image, the edges of the sprite shimmer because the
/// "which-texel-wins" decision is sensitive to sub-pixel rect
/// placement. At an integer scale factor the decision is fixed —
/// adjacent target pixels always sample adjacent 2×2 source blocks
/// with no interpolation ambiguity, no matter where the rect
/// lands. Flicker goes away.
///
/// Must remain `LOBSTER_SOURCE / 2.0` if the source ever changes
/// size; enforced by the `lobster_size_matches_source_scale` test.
pub const LOBSTER_SIZE: Size = Size {
    width: 70.0,
    height: 85.0,
};

/// Native size of each PNG frame. All 15 frames were padded to this
/// canvas during extraction. Used when converting per-frame anchor
/// offsets (measured in source pixels) to draw-space pixels.
pub const LOBSTER_SOURCE: Size = Size {
    width: 140.0,
    height: 170.0,
};

/// Per-frame body anchors inside the 140×170 source canvas. Centroid
/// alone wasn't enough to prevent flicker — a walking lobster with
/// an extended claw has its centroid pulled off the body midline, so
/// centroid-centered rendering made the body silhouette appear to
/// hop between poses. These anchors are tuned so each frame's
/// visible body midline lands at the same canvas point regardless
/// of where the claws / legs happen to be.
///
/// Seed values come from measuring the body band's x-midpoint in
/// each PNG; can be nudged by ±1 using the `PERICLAW_SPRITE_DEBUG=1`
/// overlay without a recompile cycle needed per tweak (the overlay
/// makes the mismatch visible; edit here, rebuild, re-check).
const LOBSTER_IDLE_ANCHORS: [(f32, f32); 7] = [
    (70.0, 85.0),
    (70.0, 85.0),
    (71.0, 85.0),
    (70.0, 85.0),
    (71.0, 85.0),
    (70.0, 85.0),
    (70.0, 85.0),
];
const LOBSTER_WALK_ANCHORS: [(f32, f32); 8] = [
    (73.0, 85.0),
    (74.0, 85.0),
    (74.0, 85.0),
    (75.0, 85.0),
    (75.0, 85.0),
    (76.0, 85.0),
    (73.0, 85.0),
    (70.0, 85.0),
];

/// Set `PERICLAW_SPRITE_DEBUG=1` in the env to enable a debug overlay:
/// per-sprite crosshair at `center`, wireframe of the draw bounds,
/// current frame index + phase underneath, and a static reference
/// lobster in the top-left of the scene running its walk cycle
/// without any of the wander / bob / flip / halo confounds.
///
/// Kept module-level so `office.rs` can peek at it to decide whether
/// to render the reference sprite. Read once per launch — no hot
/// path cost.
pub static DEBUG_SPRITES: LazyLock<bool> =
    LazyLock::new(|| std::env::var("PERICLAW_SPRITE_DEBUG").is_ok());

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
    // Pick the (unflipped, flipped) handle arrays + anchor table for
    // this status. The two arrays share one decode pass via
    // `LOBSTER_*_HANDLES` so frame i is guaranteed pixel-mirrored
    // against frame i unflipped.
    type PoseSelection<'a> = (
        &'a [iced_image::Handle],
        &'a [iced_image::Handle],
        &'a [(f32, f32)],
        f32,
    );
    let (unflipped, flipped, anchors, hz): PoseSelection = match status {
        AgentStatus::Running | AgentStatus::Unknown => (
            &LOBSTER_WALK_HANDLES.0,
            &LOBSTER_WALK_HANDLES.1,
            &LOBSTER_WALK_ANCHORS,
            6.0,
        ),
        AgentStatus::Ok => (
            &LOBSTER_IDLE_HANDLES.0,
            &LOBSTER_IDLE_HANDLES.1,
            &LOBSTER_IDLE_ANCHORS,
            3.0,
        ),
        AgentStatus::Error | AgentStatus::Disabled => (
            &LOBSTER_IDLE_HANDLES.0,
            &LOBSTER_IDLE_HANDLES.1,
            &LOBSTER_IDLE_ANCHORS,
            0.0,
        ),
    };
    let frames = if flip_h { flipped } else { unflipped };
    let (idx, phase) = if hz <= 0.0 || frames.is_empty() {
        (0, 0.0_f32)
    } else {
        let raw_phase = (seconds * hz).rem_euclid(frames.len() as f32);
        let i = (raw_phase as usize).min(frames.len() - 1);
        (i, raw_phase)
    };
    let handle = frames[idx].clone();

    // Per-frame anchor shift. Each PNG was padded to a 140×170
    // canvas with the sprite centered on its blob centroid, but
    // centroid ≠ body midline when claws/legs are extended — a
    // walking lobster's centroid drifts up to 6 source px across
    // the cycle. The anchor table records where the visible body
    // midline lives in each *unflipped* source frame; for a flipped
    // handle, the x anchor mirrors across the canvas X midpoint so
    // the body lands on `center` in either facing direction.
    let (raw_ax, ay) = anchors[idx.min(anchors.len() - 1)];
    let ax = if flip_h {
        LOBSTER_SOURCE.width - raw_ax
    } else {
        raw_ax
    };
    let canvas_cx = LOBSTER_SOURCE.width / 2.0;
    let canvas_cy = LOBSTER_SOURCE.height / 2.0;
    let scale_x = LOBSTER_SIZE.width / LOBSTER_SOURCE.width;
    let scale_y = LOBSTER_SIZE.height / LOBSTER_SOURCE.height;
    let dx = (ax - canvas_cx) * scale_x;
    let dy = (ay - canvas_cy) * scale_y;

    // Pin the bounds origin to integer pixels before handing them
    // to iced. Sub-pixel positions from wander / bob offsets would
    // feed into the image renderer, and even at `FilterMethod::
    // Nearest` the GPU picks subtly different source texels each
    // frame — visible shimmer along the sprite's edges.
    // `snap(true)` on the Image is belt-and-suspenders with this.
    let bounds = Rectangle::new(
        Point::new(
            (center.x - LOBSTER_SIZE.width / 2.0 - dx).floor(),
            (center.y - LOBSTER_SIZE.height / 2.0 - dy).floor(),
        ),
        LOBSTER_SIZE,
    );
    let image = iced_image::Image::new(handle)
        .filter_method(iced_image::FilterMethod::Nearest)
        .snap(true);

    // No transform. The mirror is pre-baked into the flipped handle
    // at LazyLock init — rendering with an unmodified transform
    // stack avoids iced 0.14's canvas-backend quirk where a
    // negative X scale inverts the image on Y (produced the
    // upside-down lobsters seen when flip_h was applied via
    // `scale_nonuniform`).
    frame.draw_image(bounds, image);

    // Debug overlay. Env-gated so it doesn't pay the price in
    // normal runs. Makes flicker sources visible:
    //   - Crosshair at `center` — should stay rock-still even
    //     while the sprite cycles. If the body's visible midline
    //     drifts off the crosshair, the anchor table needs a
    //     nudge for the frame index shown below.
    //   - Bounds wireframe — reveals whether bounds origin
    //     snapped to integers cleanly.
    //   - `idx` and `phase` text — so the tuning loop is "watch
    //     which frame looks off → edit that index in the anchor
    //     table → rebuild".
    if *DEBUG_SPRITES {
        let crosshair_color = *theme::TERMINAL_GREEN;
        let x_line = canvas::Path::line(
            Point::new(center.x - 12.0, center.y),
            Point::new(center.x + 12.0, center.y),
        );
        let y_line = canvas::Path::line(
            Point::new(center.x, center.y - 12.0),
            Point::new(center.x, center.y + 12.0),
        );
        frame.stroke(
            &x_line,
            canvas::Stroke::default()
                .with_color(crosshair_color)
                .with_width(1.0),
        );
        frame.stroke(
            &y_line,
            canvas::Stroke::default()
                .with_color(crosshair_color)
                .with_width(1.0),
        );
        let wireframe = canvas::Path::rectangle(bounds.position(), bounds.size());
        frame.stroke(
            &wireframe,
            canvas::Stroke::default()
                .with_color(crosshair_color)
                .with_width(1.0),
        );
        frame.fill_text(canvas::Text {
            content: format!("i={idx} p={phase:.2}"),
            position: Point::new(bounds.x, bounds.y + bounds.height + 2.0),
            color: *theme::MUTED,
            size: 10.0.into(),
            font: iced::Font::MONOSPACE,
            ..Default::default()
        });
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
pub fn decor_for_room(room_id: &str) -> Option<&'static Sprite> {
    match room_id {
        "observatory" => Some(&TELESCOPE),
        "command-hq" => Some(&CONSOLE),
        "security" => Some(&CABINET),
        "research-lab" => Some(&BEAKERS),
        "memory-vault" => Some(&ARCHIVE),
        "studio" => Some(&MICROPHONE),
        _ => None,
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

    #[test]
    fn lobster_size_matches_source_scale() {
        // Flicker fix depends on the target being an integer
        // reciprocal of the source. Anything else gives fractional
        // downsampling and the edges shimmer again.
        assert_eq!(LOBSTER_SIZE.width, LOBSTER_SOURCE.width / 2.0);
        assert_eq!(LOBSTER_SIZE.height, LOBSTER_SOURCE.height / 2.0);
    }

    #[test]
    fn lobster_anchor_tables_match_frame_counts() {
        // Index out-of-bounds in `draw_lobster` would panic at runtime
        // and silently bury the flicker fix; catch a mismatched edit
        // (e.g., someone adds a WALK frame without extending the
        // anchor table) at compile-time-ish via tests instead.
        assert_eq!(
            LOBSTER_IDLE_HANDLES.0.len(),
            LOBSTER_IDLE_ANCHORS.len(),
            "idle frame count and anchor count must match",
        );
        assert_eq!(
            LOBSTER_IDLE_HANDLES.1.len(),
            LOBSTER_IDLE_ANCHORS.len(),
            "idle flipped frame count and anchor count must match",
        );
        assert_eq!(
            LOBSTER_WALK_HANDLES.0.len(),
            LOBSTER_WALK_ANCHORS.len(),
            "walk frame count and anchor count must match",
        );
        assert_eq!(
            LOBSTER_WALK_HANDLES.1.len(),
            LOBSTER_WALK_ANCHORS.len(),
            "walk flipped frame count and anchor count must match",
        );
    }

    #[test]
    fn mirror_horizontal_reverses_rows() {
        // 2×2 RGBA, row 0 = [R, G], row 1 = [B, A] (conceptually).
        // After mirror, row 0 = [G, R], row 1 = [A, B].
        #[rustfmt::skip]
        let rgba = vec![
            1, 0, 0, 255,   2, 0, 0, 255,
            3, 0, 0, 255,   4, 0, 0, 255,
        ];
        let out = mirror_horizontal(2, 2, &rgba);
        #[rustfmt::skip]
        let expected = vec![
            2, 0, 0, 255,   1, 0, 0, 255,
            4, 0, 0, 255,   3, 0, 0, 255,
        ];
        assert_eq!(out, expected);
    }
}
