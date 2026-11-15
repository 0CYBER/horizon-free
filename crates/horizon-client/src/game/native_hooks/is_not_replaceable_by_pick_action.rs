use super::*;

impl Game<'_> {
    /// `IItemExtension.isNotReplaceableByPickAction`: may a pick leave `stack`, in hotbar
    /// `slot` (0 to 8), where it is instead of swapping it out? `not_replaceable` arrives with
    /// vanilla's answer (the stack is enchanted); the parts of the stack's item may set it.
    /// `player` is the player picking. Server-side.
    #[inline(never)]
    pub(in crate::game) fn native_is_not_replaceable_by_pick_action(
        &self,
        player: u128,
        stack: &ItemStack,
        slot: u32,
        not_replaceable: &mut bool,
    ) {
        self.fire_is_not_replaceable_by_pick_action(player, stack, slot, not_replaceable);
    }
}
