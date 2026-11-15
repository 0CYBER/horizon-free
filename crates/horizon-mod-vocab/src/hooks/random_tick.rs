//! The `random_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0014

pub trait Hooks {
    /// An authoritative server's `ServerLevel.tickChunk` is about to
    /// random-tick the block `state` at `pos` (`BlockState.randomTick`):
    /// only blocks that tick randomly, a mod's as its vanilla template block
    /// does. Bone meal's random ticks (`StemBlock.performBonemeal`) don't
    /// fire it. Change the block with `native_set_block_property` and set
    /// `handled` to skip the block's own random tick (a mod crop growing its
    /// own way); a block changed without `handled` takes its own random tick
    /// as it is now, if it still ticks randomly. Lava's fluid random tick
    /// runs either way.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "handled",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_random_tick",
        neoforge = "none (vanilla BlockBehaviour.randomTick; NeoForge adds no event or extension for it)"
    )]
    fn random_tick();
}
