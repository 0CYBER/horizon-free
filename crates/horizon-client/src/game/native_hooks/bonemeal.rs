use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `BonemealEvent`: bone meal from `stack` used on the block `state` at
    /// `pos` (`BoneMealItem.growCrop`), by `player` or, when it is `None`, by
    /// a dispenser or a villager. `valid_bonemeal_target` is the block's
    /// `isValidBonemealTarget`. Cancel skips vanilla's growth, the block's
    /// own bone meal Parts and the meal's use: `growCrop` then answers
    /// `successful` (`setSuccessful`), which is `false` unless a mod sets it.
    /// Fired on both sides; changes to `stack` stay.
    pub(in crate::game) fn native_bonemeal(&mut self, call: super::fire::BonemealCall<'_>) {
        self.fire_bonemeal(call);
    }
}
