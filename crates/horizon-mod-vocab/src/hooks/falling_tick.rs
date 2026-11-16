//! The `falling_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A falling block of `state` ticked and is still in the air
    /// (`IFallableExtension.fallingTick`): `entity` is the falling block, and `pos` the block
    /// it is in (`Entity.blockPosition`) after its move. Heard once for each tick that ends with
    /// it still falling, not in the tick it lands, drops as an item or is removed (that is
    /// `Fallable.onLand`'s moment). The part can write to the level and, through `entity`, to
    /// the falling block: its velocity, its position. The tick is recorded as it ends and
    /// delivered once the level's falling blocks, primed TNT and orbs have ticked in this pass,
    /// so a write lands on the stored falling block; a falling block gone by then is skipped.
    /// Diverges from NeoForge: server only, where NeoForge also calls it on the client. Only a
    /// state a part listens to is recorded; with no such part, one table lookup per falling
    /// block per tick. Diverges from upstream: new hook.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        neoforge = "IFallableExtension.fallingTick"
    )]
    fn falling_tick();
}
