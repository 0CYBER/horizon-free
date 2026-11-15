use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Room for a mod's own panel right of the open container screen's window, in GUI pixels:
    /// `right` starts at 0; raise it to the width your panel takes beside the window, its gap
    /// included, and leave it be otherwise. Later mods see earlier mods' answers: keep the larger
    /// (`event.right = event.right.max(mine)`). When the window, the recipe book open beside it and
    /// that room all fit across the screen, the window and the book move left by half the room
    /// (rounded down), so the three stay centred as vanilla's recipe book moves the window right to
    /// make room for itself (`RecipeBookComponent.updateScreenPosition`); when they don't fit,
    /// nothing moves. This API is right-only: mod panels reserve the right while the existing
    /// recipe book occupies the left; left reservations are not part of this hook.
    /// The query's `ScreenView::rect` is pre-reservation. Later draw/input views expose the
    /// shifted window; check that rectangle's right edge plus the room against the viewport.
    /// Keep this query pure and cheap. Its answer is memoized for the frame and screen; recursive
    /// queries during dispatch see zero. Asked once a
    /// frame while the player's inventory, a container or a mod's menu screen is open, not for the
    /// creative inventory, a merchant's or a beacon's screen, nor while the recipe book covers the
    /// window of a narrow screen; an answer that changes geometry builds the layout again in the
    /// same frame (slots, book and effect panels move with the window). Not finite or below 0
    /// counts as 0. Beyond NeoForge: a mod there reads the container screen's `leftPos` but has no
    /// event to move it, so it draws in the space that's left.
    pub(in crate::game) fn native_screen_room(&self, screen: &ScreenView<'_>, right: &mut f32) {
        self.fire_screen_room(screen, right);
    }
}
