//! The `block_entity_render_bounding_box` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0081

pub trait Hooks {
    /// Where the mod block entity at `pos` draws, for culling
    /// (NeoForge's `getRenderBoundingBox`): `bounds` is min x, y, z then max
    /// x, y, z, in blocks from the block's lower north-west corner. It starts
    /// as the block with a block of margin around it; widen it when your
    /// `block_entity_submit` draws further (a tall shaft, a beam), or
    /// set an infinite value for one never culled. Asked once a frame for each
    /// within its view distance (`block_entity_view_distance`) whose chunk is drawn.
    #[hook(
        side = "client",
        contract = "collect",
        out = "bounds",
        scope = "block_entity",
        key = "block",
        neoforge = "IBlockEntityRendererExtension.getRenderBoundingBox"
    )]
    fn block_entity_render_bounding_box();
}
