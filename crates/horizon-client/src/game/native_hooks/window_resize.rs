use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The physical or scaled GUI dimensions changed.
    pub(in crate::game) fn native_window_resize(
        &mut self,
        width: u32,
        height: u32,
        gui_width: u32,
        gui_height: u32,
    ) {
        self.fire_window_resize(width, height, gui_width, gui_height);
    }
}
