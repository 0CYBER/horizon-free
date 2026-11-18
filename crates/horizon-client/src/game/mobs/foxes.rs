//! Foxes and dolphins: the moved glue runs on the game's `MobLevel`; the foxes'
//! own steps are reached through [`FoxLevel`].
use super::Game;
use horizon_gameplay::foxes::FoxLevel;

impl Game<'_> {
    /// `Fox.ALLOWED_ITEMS` and the looting block of `Mob.aiStep` for foxes, once a tick.
    pub(super) fn tick_foxes(&mut self) {
        horizon_gameplay::foxes::tick_foxes(self);
    }
    /// `Dolphin.pickUpItem` and `Dolphin.MoveToItemGoal`'s item, once a tick.
    pub(super) fn tick_dolphins(&mut self) {
        horizon_gameplay::foxes::tick_dolphins(self);
    }
    /// `Fox.isConsumableFood`.
    fn fox_food(&self, item: &str) -> bool {
        horizon_gameplay::foxes::fox_food(self.items(), item)
    }
    /// `Fox.pickUpItem`: the extra items drop where it stands, what it held is spat out
    /// ahead of it, and it holds one of the item.
    fn fox_pick_up(&mut self, fox_index: usize, item_index: usize) {
        horizon_gameplay::foxes::fox_pick_up(self, fox_index, item_index);
    }
}

impl FoxLevel for Game<'_> {
    fn fox_food(&self, item: &str) -> bool {
        Game::fox_food(self, item)
    }
    fn fox_pick_up(&mut self, fox_index: usize, item_index: usize) {
        Game::fox_pick_up(self, fox_index, item_index);
    }
}
