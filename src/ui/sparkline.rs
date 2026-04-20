//! Token-usage sparkline for the Sessions drill-in detail pane.
//!
//! Renders `cumulativeTokens` over time as a filled line chart. When
//! the session's context budget is known, a dotted reference line
//! marks the budget so the operator can see the ratio at a glance.
//!
//! The canvas cache lives in `App::sparkline_cache` and is cleared
//! when the data changes — the draw method is a pure function of
//! the points passed in, so caching until invalidation is safe and
//! avoids redrawing on every unrelated tick.

use iced::alignment::Vertical;
use iced::mouse::Cursor;
use iced::widget::canvas::{self, Geometry, LineDash, Path, Stroke, stroke};
use iced::{Color, Point, Rectangle, Renderer, Theme};

use crate::Message;
use crate::net::rpc::SessionUsagePoint;
use crate::ui::theme;

/// Draws the sparkline. Borrow-shaped so it doesn't clone the
/// points Vec — the caller owns the data, we just read it.
pub struct TokenSparkline<'a> {
    pub points: &'a [SessionUsagePoint],
    /// Context-window size for this session, when known. Drawn as
    /// a dotted reference so the line's height is interpretable
    /// against the budget, not a bare number.
    pub context_budget: Option<i64>,
    pub cache: &'a canvas::Cache,
}

impl<'a> canvas::Program<Message> for TokenSparkline<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            draw_sparkline(frame, self.points, self.context_budget);
        });
        vec![geometry]
    }
}

fn draw_sparkline(
    frame: &mut canvas::Frame,
    points: &[SessionUsagePoint],
    context_budget: Option<i64>,
) {
    let size = frame.size();
    if size.width <= 0.0 || size.height <= 0.0 {
        return;
    }

    // With <2 points there's nothing to draw a line between — show
    // placeholder text instead so the sparkline area still
    // communicates "no data yet" rather than silently blank.
    if points.len() < 2 {
        draw_placeholder(frame, "no usage recorded yet");
        return;
    }

    // Domain: x = timestamp, y = cumulative tokens. Include the
    // context budget in the y-domain so the reference line never
    // clips out of frame even when the session is empty.
    let (t_min, t_max) = points.iter().fold((i64::MAX, i64::MIN), |(lo, hi), p| {
        (lo.min(p.timestamp), hi.max(p.timestamp))
    });
    let y_max = points
        .iter()
        .map(|p| p.cumulative_tokens)
        .max()
        .unwrap_or(0)
        .max(context_budget.unwrap_or(0));
    if t_min >= t_max || y_max <= 0 {
        draw_placeholder(frame, "usage pending first message");
        return;
    }

    // Leave a small margin so the line + label don't clip the edges.
    let pad_x = 4.0;
    let pad_y_top = 10.0;
    let pad_y_bot = 4.0;
    let plot_w = (size.width - pad_x * 2.0).max(1.0);
    let plot_h = (size.height - pad_y_top - pad_y_bot).max(1.0);

    let t_span = (t_max - t_min) as f32;
    let y_scale = plot_h / y_max as f32;

    let x_of = |ts: i64| -> f32 { pad_x + ((ts - t_min) as f32 / t_span) * plot_w };
    let y_of = |tokens: i64| -> f32 { pad_y_top + plot_h - (tokens as f32 * y_scale) };

    // Context-budget reference line first so the usage curve
    // paints on top — keeps the data layer visually dominant.
    if let Some(budget) = context_budget.filter(|b| *b > 0) {
        let y = y_of(budget);
        let path = Path::new(|p| {
            p.move_to(Point::new(pad_x, y));
            p.line_to(Point::new(pad_x + plot_w, y));
        });
        frame.stroke(
            &path,
            Stroke {
                width: 1.0,
                style: stroke::Style::Solid(Color {
                    a: 0.6,
                    ..*theme::MUTED
                }),
                line_dash: LineDash {
                    segments: &[3.0, 4.0],
                    offset: 0,
                },
                ..Default::default()
            },
        );
    }

    // Filled area under the cumulative line.
    let fill_path = Path::new(|p| {
        p.move_to(Point::new(x_of(points[0].timestamp), pad_y_top + plot_h));
        for point in points {
            p.line_to(Point::new(
                x_of(point.timestamp),
                y_of(point.cumulative_tokens),
            ));
        }
        p.line_to(Point::new(
            x_of(points[points.len() - 1].timestamp),
            pad_y_top + plot_h,
        ));
        p.close();
    });
    frame.fill(
        &fill_path,
        Color {
            a: 0.18,
            ..*theme::TERMINAL_GREEN
        },
    );

    // Cumulative-tokens line on top of the fill.
    let line_path = Path::new(|p| {
        let first = &points[0];
        p.move_to(Point::new(
            x_of(first.timestamp),
            y_of(first.cumulative_tokens),
        ));
        for point in &points[1..] {
            p.line_to(Point::new(
                x_of(point.timestamp),
                y_of(point.cumulative_tokens),
            ));
        }
    });
    frame.stroke(
        &line_path,
        Stroke {
            width: 1.5,
            style: stroke::Style::Solid(*theme::TERMINAL_GREEN),
            ..Default::default()
        },
    );

    // Current-value label in the top-right — the sparkline's only
    // text so operators can read exact tokens without mousing.
    let current = points.last().map(|p| p.cumulative_tokens).unwrap_or(0);
    let label = match context_budget.filter(|b| *b > 0) {
        Some(budget) => format!(
            "{} / {} ({:.0}%)",
            fmt_tokens_short(current),
            fmt_tokens_short(budget),
            (current as f64 / budget as f64 * 100.0).clamp(0.0, 999.0),
        ),
        None => fmt_tokens_short(current),
    };
    frame.fill_text(canvas::Text {
        content: label,
        position: Point::new(size.width - pad_x, 1.0),
        color: *theme::FOREGROUND,
        size: iced::Pixels(10.0),
        font: iced::Font::MONOSPACE,
        align_x: iced::widget::text::Alignment::Right,
        align_y: Vertical::Top,
        ..Default::default()
    });
}

fn draw_placeholder(frame: &mut canvas::Frame, msg: &str) {
    let size = frame.size();
    frame.fill_text(canvas::Text {
        content: msg.to_string(),
        position: Point::new(size.width / 2.0, size.height / 2.0),
        color: *theme::MUTED,
        size: iced::Pixels(10.0),
        font: iced::Font::MONOSPACE,
        align_x: iced::widget::text::Alignment::Center,
        align_y: Vertical::Center,
        ..Default::default()
    });
}

/// Compact token count for in-chart labels — different from the
/// list view's `fmt_tokens` in that it uses single-letter suffixes
/// and never emits a trailing zero after the decimal.
pub fn fmt_tokens_short(n: i64) -> String {
    let v = n.unsigned_abs();
    let sign = if n < 0 { "-" } else { "" };
    if v >= 1_000_000 {
        format!("{sign}{:.1}M", v as f64 / 1_000_000.0)
    } else if v >= 1_000 {
        format!("{sign}{:.1}k", v as f64 / 1_000.0)
    } else {
        format!("{sign}{v}")
    }
}

/// Minimum Iced canvas size we'll render in — below this the
/// sparkline is too small to be legible, so callers can suppress
/// it entirely rather than shoving it into a sliver.
pub const MIN_HEIGHT: f32 = 48.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_tokens_short_scales() {
        assert_eq!(fmt_tokens_short(0), "0");
        assert_eq!(fmt_tokens_short(500), "500");
        assert_eq!(fmt_tokens_short(12_400), "12.4k");
        assert_eq!(fmt_tokens_short(2_300_000), "2.3M");
        assert_eq!(fmt_tokens_short(-1_500), "-1.5k");
    }
}
