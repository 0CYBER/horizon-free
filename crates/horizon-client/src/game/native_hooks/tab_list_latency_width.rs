use super::*;

impl Game<'_> {
    /// Negotiate latency-column width before laying out the visible player list.
    pub(in crate::game) fn native_tab_list_latency_width(&self, width: &mut i32) {
        self.fire_tab_list_latency_width(width);
    }
}
