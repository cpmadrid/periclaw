//! Dynamic grid layout for an arbitrary list of rooms.
//!
//! Takes a `&[Room]` and packs them into a rows×cols grid picked from a
//! small lookup table, then exposes `room_rect()` / `sprite_slot()`
//! keyed by the room's string id. The old hardcoded 3×2 layout was a
//! closed enum (`RoomId::Observatory` etc.); this replaces it so the
//! operator can edit the room list in `UiState` without any code change.

use iced::{Point, Rectangle, Size};

use crate::domain::Room;

/// How many (cols, rows) to use for a given room count. Hand-picked so
/// every supported size fills the canvas without awkward gaps.
///
///   1 → 1×1  ·  2 → 2×1  ·  3 → 3×1
///   4 → 2×2  ·  5-6 → 3×2  ·  7-8 → 4×2
///
/// Counts outside 1..=8 get clamped — anything bigger falls back to
/// 4×2 and rooms past slot 8 are dropped at draw time.
fn grid_for(count: usize) -> (usize, usize) {
    match count {
        0 | 1 => (1, 1),
        2 => (2, 1),
        3 => (3, 1),
        4 => (2, 2),
        5 | 6 => (3, 2),
        _ => (4, 2),
    }
}

/// Layout of the room grid inside a given scene bounds.
#[derive(Debug, Clone)]
pub struct RoomLayout<'a> {
    pub bounds: Rectangle,
    pub gutter: f32,
    pub rooms: &'a [Room],
    cols: usize,
    rows: usize,
}

impl<'a> RoomLayout<'a> {
    pub fn new(bounds: Rectangle, rooms: &'a [Room]) -> Self {
        let (cols, rows) = grid_for(rooms.len());
        Self {
            bounds,
            gutter: 12.0,
            rooms,
            cols,
            rows,
        }
    }

    /// Iterate `(room, rect)` pairs in grid order. Callers that need
    /// both the room metadata and its pixel bounds (room-drawing loop
    /// + sprite placement) can just walk this.
    pub fn iter(&self) -> impl Iterator<Item = (&'a Room, Rectangle)> + '_ {
        self.rooms
            .iter()
            .take(self.cols * self.rows)
            .enumerate()
            .map(move |(idx, room)| (room, self.rect_at_index(idx)))
    }

    /// Rectangle occupied by the room with the given id, if present in
    /// the configured list.
    pub fn room_rect(&self, id: &str) -> Option<Rectangle> {
        self.rooms
            .iter()
            .take(self.cols * self.rows)
            .position(|r| r.id == id)
            .map(|idx| self.rect_at_index(idx))
    }

    fn rect_at_index(&self, idx: usize) -> Rectangle {
        let col = (idx % self.cols) as f32;
        let row = (idx / self.cols) as f32;
        let cols = self.cols as f32;
        let rows = self.rows as f32;

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
    pub fn sprite_slot(&self, rect: Rectangle, index: usize) -> Point {
        let slot_count = 4;
        let idx = (index % slot_count) as f32;
        let slot_col = idx % 2.0;
        let slot_row = (idx / 2.0).floor();

        let pad = 18.0;
        let usable_w = rect.width - pad * 2.0;
        let slot_w = usable_w / 2.0;
        let slot_h = (rect.height - pad * 2.0) / 2.0;

        let x = rect.x + pad + slot_col * slot_w + slot_w / 2.0;
        // Lower half so room labels at the top stay readable.
        let y = rect.y + rect.height * 0.55 + slot_row * (slot_h * 0.4);

        Point::new(x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::room::default_rooms;

    #[test]
    fn six_room_layout_is_three_by_two() {
        let rooms = default_rooms();
        let layout = RoomLayout::new(
            Rectangle::new(Point::ORIGIN, Size::new(900.0, 600.0)),
            &rooms,
        );
        let rects: Vec<_> = layout.iter().map(|(_, r)| r).collect();
        assert_eq!(rects.len(), 6);
        // First row y-equal to second room.
        assert_eq!(rects[0].y, rects[1].y);
        // Second row shifted down.
        assert!(rects[3].y > rects[0].y);
    }

    #[test]
    fn grid_table_matches_spec() {
        assert_eq!(grid_for(1), (1, 1));
        assert_eq!(grid_for(2), (2, 1));
        assert_eq!(grid_for(3), (3, 1));
        assert_eq!(grid_for(4), (2, 2));
        assert_eq!(grid_for(5), (3, 2));
        assert_eq!(grid_for(6), (3, 2));
        assert_eq!(grid_for(7), (4, 2));
        assert_eq!(grid_for(8), (4, 2));
    }

    #[test]
    fn room_rect_finds_by_id() {
        let rooms = default_rooms();
        let layout = RoomLayout::new(
            Rectangle::new(Point::ORIGIN, Size::new(900.0, 600.0)),
            &rooms,
        );
        assert!(layout.room_rect("command-hq").is_some());
        assert!(layout.room_rect("does-not-exist").is_none());
    }
}
