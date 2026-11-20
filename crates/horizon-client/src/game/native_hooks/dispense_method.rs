use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `DispenserBlock.getDispenseMethod`, before the item's `dispense_item`:
    /// the block the dispenser faces chooses what the dispense does, for any
    /// item, including vanilla items, on the authoritative server (not
    /// droppers). `state` is the block at `pos` moved by `facing` (air
    /// included), and the part is asked only for that block. An answer wins:
    /// no item part is asked and vanilla's behaviour
    /// doesn't run. The answer is `dispense_item`'s: a remainder of None
    /// empties the slot, `Some(stack)` replaces it (the hook owns
    /// consumption: no extra item is removed afterwards), and `event` is the
    /// level event played with the smoke: 1000 the click, 1001 the fail sound,
    /// or 0 for nothing at all. Leave answer None and `dispense_item` decides,
    /// as it did before.
    pub(in crate::game) fn native_dispense_method(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        state: BlockStateId,
        stack: &ItemStack,
        answer: &mut Option<mod_api::Dispensed>,
    ) {
        self.fire_dispense_method(dimension, pos, facing, state, stack, answer);
    }

    /// A dispenser acts on `stack`, before the faced block's `dispense_method` and the item's
    /// `dispense_item`: one global answer for any block or item. A mod's answer wins and no
    /// part is asked. Server-side, not droppers.
    pub(in crate::game) fn native_dispenser_acts(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        stack: &ItemStack,
        answer: &mut Option<mod_api::Dispensed>,
    ) {
        self.fire_dispenser_acts(dimension, pos, facing, stack, answer);
    }
}
