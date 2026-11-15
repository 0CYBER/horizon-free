//! The `should_display_fluid_overlay` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether a fluid's side beside a block state uses the fluid's overlay sprite (water's `water_overlay`)
    /// instead of its flowing sprite (`IBlockExtension.shouldDisplayFluidOverlay`): set `event.overlay`.
    /// `overlay` starts with vanilla's test, true beside a `HalfTransparentBlock` or `LeavesBlock` (for a content
    /// block, its template's class), so a part can turn the overlay on or off. `event.fluid` is the fluid as it is
    /// asked: a source at amount 8, or a flowing fluid at amount 7, not falling. A fluid with no overlay sprite,
    /// such as lava, draws its flowing sprite whatever the answer. The answer depends on the state and the fluid
    /// alone and is resolved at join or explicit visibility refresh, never on a meshing worker: the client asks once for every state and fluid where
    /// it resolves `block_render_visibility` (joining a world, `ClientHost::refresh_render_visibility`), and the
    /// meshing workers read the table. The level and position are dropped: the meshing workers run no mod code.
    /// With no part answering, nothing is asked and meshes are unchanged. Diverges from upstream: new hook.
    #[hook(
        side = "client",
        contract = "collect",
        out = "overlay",
        scope = "block",
        key = "state",
        neoforge = "IBlockExtension.shouldDisplayFluidOverlay"
    )]
    fn should_display_fluid_overlay();
}
