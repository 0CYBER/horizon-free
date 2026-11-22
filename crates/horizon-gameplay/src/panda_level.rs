//! What panda code calls on `ServerLevel` beyond [`MobLevel`]: the food tag, the player's
//! game mode and the parts of `Animal.setInLove` and the held stack that need the game.
use crate::mob_level::MobLevel;

pub trait PandaLevel: MobLevel {
    /// `Panda.canPickUpAndEat`'s tag (`#minecraft:panda_eats_from_ground`).
    fn panda_eats_from_ground(&self, item: &str) -> bool;
    /// Whether the player who clicked has infinite materials.
    fn is_creative(&self) -> bool;
    /// The panda's current index and whether love mode may start. None when
    /// the animal left the interaction.
    fn panda_love_decision(&mut self, index: usize) -> Option<(usize, bool)>;
    /// `Animal.setInLove` for the panda fed by the acting player: who gets the credit,
    /// the mods' enter-love hook, and entity event 18, the love hearts.
    fn panda_in_love(&mut self, index: usize);
    /// `ItemStack.consume(1)` on the held stack of the player who clicked.
    fn consume_held(&mut self);
    /// Called after the mobs were lifted out of the level for a pass and before the pass
    /// reads the level: the game's tests spawn a mob here.
    fn after_mobs_lifted(&mut self) {}
}
