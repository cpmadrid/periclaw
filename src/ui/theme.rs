//! Dark-mode palette mirrored from apps/web/src/app/globals.css.
//!
//! Web app expresses colors in OKLCH (OkLab with chroma+hue). Iced's
//! [`iced::Color`] is sRGB floats in `[0, 1]`, so we convert on the
//! way in via the `palette` crate. Conversions are cheap and run
//! once at startup through [`std::sync::LazyLock`].

// Entire palette is pre-defined to mirror globals.css; some entries
// are consumed by later milestones.
#![allow(dead_code)]

use std::sync::LazyLock;

use iced::Color;
use palette::{FromColor, Oklch, Srgb};

fn oklch(l: f32, c: f32, h: f32) -> Color {
    let src = Oklch::new(l, c, h);
    let srgb: Srgb = Srgb::from_color(src);
    Color::from_rgb(
        srgb.red.clamp(0.0, 1.0),
        srgb.green.clamp(0.0, 1.0),
        srgb.blue.clamp(0.0, 1.0),
    )
}

fn oklch_a(l: f32, c: f32, h: f32, a: f32) -> Color {
    let src = Oklch::new(l, c, h);
    let srgb: Srgb = Srgb::from_color(src);
    Color::from_rgba(
        srgb.red.clamp(0.0, 1.0),
        srgb.green.clamp(0.0, 1.0),
        srgb.blue.clamp(0.0, 1.0),
        a,
    )
}

// Layered dark surfaces (green-tinted, hue 155)
pub static SURFACE_0: LazyLock<Color> = LazyLock::new(|| oklch(0.08, 0.005, 155.0));
pub static SURFACE_1: LazyLock<Color> = LazyLock::new(|| oklch(0.11, 0.008, 155.0));
pub static SURFACE_2: LazyLock<Color> = LazyLock::new(|| oklch(0.14, 0.010, 155.0));
pub static SURFACE_3: LazyLock<Color> = LazyLock::new(|| oklch(0.17, 0.013, 155.0));

// Foreground / text
pub static FOREGROUND: LazyLock<Color> = LazyLock::new(|| oklch(0.75, 0.08, 155.0));
pub static MUTED: LazyLock<Color> = LazyLock::new(|| oklch(0.50, 0.06, 155.0));

// Signature terminal green
pub static TERMINAL_GREEN: LazyLock<Color> = LazyLock::new(|| oklch(0.72, 0.19, 155.0));
pub static TERMINAL_GREEN_DIM: LazyLock<Color> = LazyLock::new(|| oklch(0.45, 0.12, 155.0));
pub static TERMINAL_GREEN_GLOW: LazyLock<Color> =
    LazyLock::new(|| oklch_a(0.72, 0.19, 155.0, 0.25));

// Status indicators
pub static STATUS_UP: LazyLock<Color> = LazyLock::new(|| oklch(0.72, 0.19, 155.0));
pub static STATUS_DOWN: LazyLock<Color> = LazyLock::new(|| oklch(0.63, 0.24, 25.0));
pub static STATUS_DEGRADED: LazyLock<Color> = LazyLock::new(|| oklch(0.75, 0.18, 75.0));
pub static STATUS_UNKNOWN: LazyLock<Color> = LazyLock::new(|| oklch(0.35, 0.03, 155.0));

// Border / ring (translucent terminal green)
pub static BORDER: LazyLock<Color> = LazyLock::new(|| oklch_a(0.55, 0.15, 155.0, 0.30));

/// Build the Iced custom theme for the app.
pub fn periclaw_theme() -> iced::Theme {
    iced::Theme::custom(
        "PeriClaw".to_string(),
        iced::theme::Palette {
            background: *SURFACE_0,
            text: *FOREGROUND,
            primary: *TERMINAL_GREEN,
            success: *STATUS_UP,
            warning: *STATUS_DEGRADED,
            danger: *STATUS_DOWN,
        },
    )
}
