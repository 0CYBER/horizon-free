//! The `bucket_empty` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0240

pub trait Hooks {
    /// A server player empties a bucket at `pos` (`fluid`: `minecraft:water`,
    /// `minecraft:lava`, `minecraft:powder_snow`; a fish or axolotl bucket
    /// empties `minecraft:water`): where `BucketItem.use` has found the block
    /// the fluid goes into and the player may build there, right before it is
    /// placed. Set `cancel` and nothing is poured and the bucket stays full.
    /// The player's state is swapped in, so `PlayerRef::tell` answers them. Only
    /// for guests of a server. `fill_bucket` hears this click too (and
    /// the filling one) without the player and the fluid. Not heard: a
    /// dispenser, a cauldron, and a fluid a block places itself.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (vanilla BucketItem.emptyContents; NeoForge has no matching event)",
        cancellable = true
    )]
    fn bucket_empty();
}
