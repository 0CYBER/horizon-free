use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The local player left-clicks empty space, with any main-hand stack.
    /// Client-side only; the server does not receive this event.
    pub(in crate::game) fn native_left_click_empty(&mut self) {
        self.fire_left_click_empty();
    }
}
