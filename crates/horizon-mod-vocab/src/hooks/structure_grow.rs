//! The `structure_grow` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A tree, mushroom or fungus grew from `source` (a sapling's random tick, a fungus, or
    /// a feature placed through the level): its feature placed the blocks `placed`, in
    /// order. The blocks are already in the world, and the hook only tells. Server-side.
    ///
    /// `structure_grow` tells a mod which blocks a grown feature changed: a tree from a sapling, a
    /// mushroom or a fungus. `source` is the position the feature grew from (the sapling or
    /// fungus). `placed` lists every position whose block changed, in placement order, and the
    /// hook is asked only when the feature placed something. It is listen-only: a feature cannot be
    /// cancelled here yet, because the list is known only after the blocks are written. Bone meal
    /// on a sapling, an azalea, a fungus or nylium, and grass bone meal vegetation, place their
    /// features through this path and fire it too.
    #[hook(
        side = "server",
        target = "horizon_client::game::native_hooks::structure_grow::GameLevel<'_>::native_structure_grow",
        contract = "listen",
        neoforge = "none (NeoForge has no structure growth event)",
        paper = "StructureGrowEvent (getBlocks gives the placed blocks; Paper's event is cancellable and this one is not)"
    )]
    fn structure_grow();
}
