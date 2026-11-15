//! The `light_emission` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The light the block `state` emits at `pos` in `dimension`
    /// (`IBlockExtension.getLightEmission`), for a block that declares
    /// `dynamic_light: true`. The declaration is per block, not per state as
    /// `hasDynamicLightEmission` is, because it lives on the block's row: every state
    /// of a declared block is asked, and parts on other blocks are never asked, at any
    /// moment (NeoForge asks on a block change and a queued check whether or not the
    /// block declares). `emission` starts empty: set `Some(level)` to replace the
    /// state's own light at that position, or leave it empty to keep it. An answer
    /// above 15 is clamped to 15 here, so an out-of-range value never reaches the
    /// light queue; NeoForge clamps only the value its light manager stores. Asked
    /// only on the thread that owns the level, both sides, with that level read-only,
    /// as a chunk holding the block comes in, as an edit sets it and at
    /// `LevelRef::check_light` (`LevelLightEngine.checkBlock`); the light engine keeps
    /// the answer until the cell's state changes or the chunk unloads, its workers
    /// read only that answer, and it never asks on its own: an answer that reads more
    /// than the state (the block below, the depth) calls `check_light` when that
    /// changes. A client also asks its own parts again, at most once per cell, for the
    /// sections a server's light update changed: a Horizon addition, as the contract
    /// asks on no light packet, for an answer that reads data the client receives
    /// later. Not asked for the block's own rendered light, ambient occlusion or the
    /// spawn rule, which take the state's light. The game may ask more often than the
    /// contract does, so keep the answer a pure function of the level, `pos` and
    /// `state`. Diverges from upstream: new hook.
    #[hook(
        side = "both",
        contract = "replace",
        out = "emission",
        scope = "block",
        key = "state",
        host = "read",
        neoforge = "IBlockExtension.getLightEmission, IBlockStateExtension.getLightEmission"
    )]
    fn light_emission();
}
