//! 3×2 grid of rooms, drawn as labeled rectangles on the Canvas.
//!
//! Sprites are positioned inside room bounds by
//! [`scene::office::OfficeScene`]. Simple grid math for v1 —
//! nicer tilemaps land in M4.

use iced::{Point, Rectangle, Size};

use crate::domain::RoomId;

/// Layout of the 3×2 room grid inside a given scene bounds.
#[derive(Debug, Clone, Copy)]
pub struct RoomLayout {
    pub bounds: Rectangle,
    pub gutter: f32,
}

impl RoomLayout {
    pub fn new(bounds: Rectangle) -> Self {
        Self {
            bounds,
            gutter: 12.0,
        }
    }

    /// Rectangle (in the scene's local coordinates) occupied by a room.
    pub fn room_rect(&self, room: RoomId) -> Rectangle {
        let col = room.col() as f32;
        let row = room.row() as f32;

        let cols = 3.0;
        let rows = 2.0;

        let total_gutter_x = self.gutter * (cols + 1.0);
        let total_gutter_y = self.gutter * (rows + 1.0);

        let w = (self.bounds.width - total_gutter_x) / cols;
        let h = (self.bounds.height - total_gutter_y) / rows;

        let x = self.bounds.x + self.gutter + col * (w + self.gutter);
        let y = self.bounds.y + self.gutter + row * (h + self.gutter);

        Rectangle::new(Point::new(x, y), Size::new(w, h))
    }

    /// Pick a position inside a room for the Nth sprite in that room
    /// (so multiple sprites in the same room don't stack).
    pub fn sprite_slot(&self, room: RoomId, index: usize) -> Point {
        let rect = self.room_rect(room);
        // 4 slots per room arranged in a 2×2 mini-grid within the lower half
        let slot_count = 4;
        let idx = (index % slot_count) as f32;
        let slot_col = idx % 2.0;
        let slot_row = (idx / 2.0).floor();

        let pad = 18.0;
        let usable_w = rect.width - pad * 2.0;
        let usable_h = rect.height - pad * 2.0;
        let slot_w = usable_w / 2.0;
        let slot_h = usable_h / 2.0;

        // Place sprites in the lower half so room labels read clearly at the top.
        let x = rect.x + pad + slot_col * slot_w + slot_w / 2.0;
        let y = rect.y + rect.height * 0.55 + slot_row * (slot_h * 0.4);

        Point::new(x, y)
    }
}
