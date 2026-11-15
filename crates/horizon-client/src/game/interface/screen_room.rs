//! Room beside an open container screen's window for a mod's own panel (`screen_room`): the
//! window, and the recipe book open beside it, move left by half the room when all three fit
//! across the screen, as the book moves the window right to make room for itself
//! (`RecipeBookComponent.updateScreenPosition`).
use super::*;

/// How far `room` right of a window `image` wide moves the window, its left edge at `left`, and
/// the book open beside it, its left edge at `book`, on a screen `width` wide: half the room,
/// rounded down, when the book, the window and the room all fit across the screen; else nothing.
pub(super) fn room_shift(width: f32, left: f32, image: f32, book: Option<f32>, room: f32) -> f32 {
    if !room.is_finite() || room <= 0.0 {
        return 0.0;
    }
    let half = (room / 2.0).floor();
    let first = book.map_or(left, |book| book.min(left));
    if first - half >= 0.0 && left - half + image + room <= width {
        half
    } else {
        0.0
    }
}

impl Game<'_> {
    /// The room the mods ask right of the open container screen's window (`screen_room`), asked
    /// once a frame: 0 when no mod listens, for the creative inventory, a merchant's or a
    /// beacon's screen, and while the recipe book covers the window of a narrow screen.
    pub(super) fn mod_room(&self, batch: &HudBatch) -> f32 {
        let kind = self.station.as_ref().map(|s| s.kind);
        if self.screen != Screen::Inventory
            || matches!(
                kind,
                Some(stations::Kind::Merchant | stations::Kind::Beacon)
            )
            || self.book_covers_menu(batch.width)
            || !mod_api::__private::listened(mod_api::HookId::ScreenRoom)
        {
            return 0.0;
        }
        if let Some(room) = self
            .screen_state
            .build
            .room_answer(self.frames, self.screen)
        {
            return room;
        }
        let image = self.native_screen_width().unwrap_or(176.0);
        let height = self.screen_height();
        let (x, y) = batch.centered_origin(image, height);
        let rect = [x + self.recipe_book_shift(batch.width), y, image, height];
        let view = self.screen_view_at(self.screen, rect);
        // No room while the mods are asked, so a host call of theirs that lays the screen out
        // doesn't ask them again.
        let build = &self.screen_state.build;
        build.keep_room_answer(self.frames, self.screen, 0.0);
        let mut right = 0.0;
        self.native_screen_room(&view, &mut right);
        let room = if right.is_finite() {
            right.max(0.0)
        } else {
            0.0
        };
        build.keep_room_answer(self.frames, self.screen, room);
        room
    }

    /// How far the mods' room moves the open container screen's window, and the recipe book
    /// open beside it, left (`room_shift`).
    pub(super) fn screen_room_shift(&self, batch: &HudBatch) -> f32 {
        let room = self.mod_room(batch);
        if room <= 0.0 {
            return 0.0;
        }
        let image = self.native_screen_width().unwrap_or(176.0);
        let (x, _) = batch.centered_origin(image, self.screen_height());
        let left = x + self.recipe_book_shift(batch.width);
        room_shift(batch.width, left, image, self.open_book_left(batch), room)
    }
}

#[cfg(test)]
mod tests {
    use super::room_shift;

    #[test]
    fn the_room_moves_the_window_by_half_when_it_all_fits() {
        // 1440p at the automatic GUI scale: 427 wide, the window at 125. A panel of 147 and its
        // gap of 2 fit right of it once the window moves 74 left.
        assert_eq!(room_shift(427.0, 125.0, 176.0, None, 149.0), 74.0);
        // Just wide enough, and a pixel short.
        assert_eq!(room_shift(325.0, 74.0, 176.0, None, 149.0), 74.0);
        assert_eq!(room_shift(320.0, 72.0, 176.0, None, 149.0), 0.0);
        // No room asked, or none that counts.
        assert_eq!(room_shift(427.0, 125.0, 176.0, None, 0.0), 0.0);
        assert_eq!(room_shift(427.0, 125.0, 176.0, None, -20.0), 0.0);
        assert_eq!(room_shift(427.0, 125.0, 176.0, None, f32::NAN), 0.0);
        assert_eq!(room_shift(427.0, 125.0, 176.0, None, f32::INFINITY), 0.0);
    }

    #[test]
    fn the_book_beside_the_window_moves_with_it_or_keeps_it_in_place() {
        // The book open beside the window (the window 77 right of centre, the book left of it):
        // at 427 the three don't fit, so nothing moves; at 640 they do.
        let at_427 = (125.0 + 77.0, ((427.0 - 147.0) / 2.0f32).floor() - 86.0);
        assert_eq!(
            room_shift(427.0, at_427.0, 176.0, Some(at_427.1), 149.0),
            0.0
        );
        let at_640 = (232.0 + 77.0, ((640.0 - 147.0) / 2.0f32).floor() - 86.0);
        assert_eq!(
            room_shift(640.0, at_640.0, 176.0, Some(at_640.1), 149.0),
            74.0
        );
        // The book's left edge is what runs out first on the left.
        assert_eq!(room_shift(640.0, 300.0, 176.0, Some(60.0), 149.0), 0.0);
    }
}
