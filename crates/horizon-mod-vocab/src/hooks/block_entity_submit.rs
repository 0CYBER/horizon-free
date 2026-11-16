//! The `block_entity_submit` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0080

pub trait Hooks {
    /// Draw the mod block entity at `pos`, of block `block`
    /// (`mymod:machine`), on this client: once a frame while it is in view
    /// (vanilla's `BlockEntityRenderer.submit`: within its view distance, 64 blocks unless
    /// `block_entity_view_distance` says otherwise, its chunk
    /// faded in, and its bounds, `block_entity_render_bounding_box`,
    /// in the frustum), when its `id` is yours. `state` is the block's state
    /// (`blockEntity.getBlockState()`: read its `facing` with
    /// `native_block_state_property`), `entity` holds what the server sent
    /// (`block_entity_update_tag`). Add geometry with
    /// `native_draw_quad`, `native_draw_cube`, `native_draw_block` and
    /// `native_draw_item` under `draw`'s pose stack, which starts at the
    /// block's lower north-west corner, in blocks; `draw` also has the partial
    /// tick, the game time and the block's light.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "block",
        neoforge = "EntityRenderersEvent.RegisterRenderers"
    )]
    fn block_entity_submit();
}
