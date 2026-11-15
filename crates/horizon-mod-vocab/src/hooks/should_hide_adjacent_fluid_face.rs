//! The `should_hide_adjacent_fluid_face` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether a block state hides the face a fluid beside it draws toward it
    /// (`IBlockExtension.shouldHideAdjacentFluidFace`): set `event.hidden` and the fluid's face toward the block is
    /// not drawn. `event.direction` points from the fluid to the block (Horizon's reading, not checked), and
    /// `event.fluid` is the fluid as it is asked: a source at amount 8, or a flowing fluid at amount 7, not falling.
    /// `hidden` starts false. Vanilla's tests still apply first: a fluid beside the same fluid, or behind a face
    /// that occludes it, draws no face there whatever the answer. The answer depends on the state and the fluid
    /// alone and is resolved at join or explicit visibility refresh, never on a meshing worker: the client asks once for every state and fluid where
    /// it resolves `block_render_visibility` (joining a world, `ClientHost::refresh_render_visibility`), and the
    /// meshing workers read the table. The level and position are not arguments. With no part answering, nothing
    /// is asked and meshes are unchanged. Diverges from upstream: new hook.
    #[hook(
        side = "client",
        contract = "collect",
        out = "hidden",
        scope = "block",
        key = "state",
        neoforge = "IBlockExtension.shouldHideAdjacentFluidFace"
    )]
    fn should_hide_adjacent_fluid_face();
}
