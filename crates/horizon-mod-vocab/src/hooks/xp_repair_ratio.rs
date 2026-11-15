//! The `xp_repair_ratio` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IItemExtension.getXpRepairRatio`: the durability points Mending
    /// restores to `stack` per point of experience, where
    /// `ExperienceOrb.repairPlayerItems` repairs the damaged Mending item it
    /// picked, once per item it repairs. `experience` is the points being
    /// spent (never 0) and `player` the player they go to. `ratio` arrives
    /// holding vanilla's: the stack's `minecraft:repair_with_xp` repair
    /// divided by `experience`, 2.0 for vanilla Mending. The repair becomes
    /// `floor(experience × ratio)`, capped at the stack's damage; a ratio left
    /// as it arrived keeps vanilla's repair exactly. A ratio that isn't finite
    /// is refused and vanilla's repair is used; a negative one counts as 0.
    /// A ratio that takes a positive repair to 0 leaves the experience
    /// unspent, for the player. Only the parts attached to `stack`'s item are
    /// asked: to change Mending for every item, change the enchantment's
    /// `minecraft:repair_with_xp` effect in a data pack. Server-side,
    /// including `PlayerRef::give_experience_points`.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "IItemExtension.getXpRepairRatio"
    )]
    fn xp_repair_ratio();
}
