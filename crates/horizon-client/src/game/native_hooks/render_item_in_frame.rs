use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Before a visible item frame's item or map is drawn. Cancel hides the item, not the frame.
    pub(in crate::game) fn native_render_item_in_frame(
        &self,
        entity_id: u32,
        pos: [i32; 3],
        stack: &ItemStack,
        canceled: &mut bool,
    ) {
        self.fire_render_item_in_frame(entity_id, pos, stack, canceled);
    }
}
