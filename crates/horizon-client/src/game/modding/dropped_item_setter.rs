use super::Game;
use horizon_client::items::ItemStack;

impl Game<'_> {
    /// Sets the stack of item entity `id` in the server level `dimension`, as
    /// a hopper's `addItem` leaves what it couldn't take (`ItemEntity.setItem`);
    /// an empty stack (count 0) removes the entity (`setItem(EMPTY)`, then
    /// `discard`). Players see it on their next tracker update. False when the
    /// level in place has no such item entity, or on a client.
    pub(super) fn native_set_item_entity(
        &mut self,
        dimension: &str,
        id: u32,
        stack: ItemStack,
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
        {
            return false;
        }
        if self.block_entity_tick_key.is_some() {
            return self
                .stage_tick_item(id, (stack.count != 0).then_some(stack))
                .unwrap_or(false);
        }
        let Some(i) = self.dropped.iter().position(|e| e.id == id) else {
            return false;
        };
        if stack.count == 0 {
            self.remove_dropped(i);
        } else {
            self.dropped[i].stack = stack;
        }
        true
    }
}
