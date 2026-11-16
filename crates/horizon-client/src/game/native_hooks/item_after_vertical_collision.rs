use super::*;
impl Game<'_> {
    /// Dropped-item contact after vertical collision response and before block effects.
    pub(in crate::game) fn native_item_after_vertical_collision(
        &mut self,
        call: crate::game::native_hooks::fire::ItemAfterVerticalCollisionCall<'_>,
    ) {
        self.fire_item_after_vertical_collision(call);
    }
}
