use super::*;

impl Game<'_> {
    /// an item fallback after a native mob's ordinary interaction passed.
    pub(in crate::game) fn native_item_interact_mob(
        &mut self,
        hand: Hand,
        entity: i32,
        kind: &str,
        stack: &ItemStack,
        result: &mut Option<mod_api::InteractionResult>,
    ) {
        self.fire_item_interact_mob(hand, entity, kind, stack, result);
    }
}
