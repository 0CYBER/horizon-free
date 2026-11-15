//! The `can_harvest_block` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether the player `player`'s tool harvests the block `state` at `pos`
    /// (`IBlockExtension.canHarvestBlock`, what `Player.hasCorrectToolForDrops` asks of the block).
    /// `can_harvest` starts with vanilla's answer: false for a block that needs a tool the player's
    /// held item is not, true otherwise. Set it for your own block; false breaks the block and drops
    /// nothing, and the dig is slowed as vanilla slows a wrong tool. The part answers first and the
    /// global `player_harvest_check` hears the result, so a mod reacting to every block still has the
    /// last word. Asked wherever the harvest rule is: the dig's progress (both sides, so a client
    /// predicts it), the break and a door's break.
    #[hook(
        side = "both",
        contract = "collect",
        out = "can_harvest",
        scope = "block",
        key = "state",
        neoforge = "IBlockExtension.canHarvestBlock"
    )]
    fn can_harvest_block();
}
