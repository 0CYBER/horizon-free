//! Foxes and dolphins: the moved glue runs on the game's `MobLevel`.
use super::Game;

impl Game<'_> {
    /// `Fox.ALLOWED_ITEMS` and the looting block of `Mob.aiStep` for foxes, once a tick.
    pub(super) fn tick_foxes(&mut self) {
        horizon_gameplay::foxes::tick_foxes(self);
    }
    /// `Dolphin.pickUpItem` and `Dolphin.MoveToItemGoal`'s item, once a tick.
    pub(super) fn tick_dolphins(&mut self) {
        horizon_gameplay::foxes::tick_dolphins(self);
    }
}
