use super::*;

impl Game<'_> {
    /// Whether `stack` may be placed into `slot` of the mod container block entity `entity`
    /// at `pos` (`Container.canPlaceItem`). `Some(true)` allows, `Some(false)` refuses, and
    /// `None` keeps the container's default `inputs` rule.
    pub(in crate::game) fn native_can_place_item(
        pos: [i32; 3],
        entity: &ModBlockEntity,
        slot: u32,
        stack: &ItemStack,
        allowed: &mut Option<bool>,
    ) {
        Self::fire_can_place_item(pos, entity, slot, stack, allowed);
    }
}
