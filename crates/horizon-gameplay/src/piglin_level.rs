//! What piglin code calls on `ServerLevel` beyond [`MobLevel`]: the content tables it
//! reads and the questions only the game can answer.
use crate::mob_level::MobLevel;
use horizon_client::{
    items::{ItemStack, Items},
    mobs::Mob,
};
use horizon_core::geometry::Vec3;

pub trait PiglinLevel: MobLevel {
    /// The game's own content tables (`Resources.items`): item tags and item data.
    fn res_items(&self) -> &Items;
    /// The content tables the world plays with (its data packs' over the game's own).
    fn world_items(&self) -> &Items;
    // The `Game` methods piglin code calls on itself. Each is the game's own method, so a mod
    // that listens to one still hears the moved code; the free functions of `piglins` are
    // what those methods run.
    fn item_in_tag(&self, tag: &str, item: &str) -> bool;
    fn piglin_loved(&self, item: &str) -> bool;
    /// `PiglinAi.isBarterCurrency`.
    fn piglin_currency(&self, stack: &ItemStack) -> bool;
    fn item_blocks_attacks(&self, item: &str) -> bool;
    fn piglin_wanted(&self, m: &Mob, item: &str) -> bool;
    fn piglin_slot(&self, item: &str) -> usize;
    fn piglin_can_replace(&self, m: &Mob, stack: &ItemStack, slot: usize) -> bool;
    fn piglin_can_add(&self, m: &Mob, stack: &ItemStack) -> bool;
    fn piglin_max_stack(&self, item: &str) -> u32;
    fn piglin_wants(&self, m: &Mob, stack: &ItemStack) -> bool;
    fn piglin_equip(&mut self, index: usize, stack: &ItemStack) -> bool;
    fn piglin_stash(&mut self, index: usize, stack: ItemStack);
    fn piglin_keep(&mut self, index: usize, stack: ItemStack);
    fn piglin_throw(&mut self, index: usize, target_pos: [f64; 3], stack: ItemStack);
    fn admire_seen_item(&mut self, index: usize, nearest: Option<f64>, loved: bool);
    fn start_admiring(&mut self, index: usize, stack: ItemStack);
    fn piglin_throw_items(&mut self, i: usize, items: Vec<ItemStack>);
    /// The bartering loot table rolled on its own `random_sequence`, not the
    /// level random (`PiglinAi.barter` / `LootContext.getRandom`).
    fn piglin_barter_items(&mut self) -> Vec<ItemStack>;
    /// Where `PiglinAi.throwItems` throws toward from `at`: one block above the feet of
    /// the nearest player within 16 blocks, the level's own player ahead of the guests,
    /// those alive and not spectators. `None` when no player qualifies.
    fn piglin_throw_target(&self, at: Vec3) -> Option<[f64; 3]>;
}
