//! The `state_at_viewpoint` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The block state the camera at `viewpoint` sees of the block `state` at `pos`
    /// (`IBlockExtension.getStateAtViewpoint`), so a partial fluid block, a tank or a
    /// waterlogged mod block can show underwater fog only while the eye is really inside the
    /// liquid. `seen` starts at `state`: set it to the state whose fluid and powder snow the
    /// camera should read. Asked as the client finds the fluid at the camera
    /// (`Camera.getFluidInCamera`) for the block at the eye, the block above it and the blocks
    /// at the near plane's centre and corners: the fog, the field of view in a fluid, the
    /// water vision, the sky's dark disc and conduit power's underwater vision read it. The
    /// client host is read-only; answer from the event alone, as the game asks every frame. With no
    /// part on any block the subscription check skips the asks; otherwise each block read costs
    /// one table lookup. Diverges from upstream: new hook.
    #[hook(
        side = "client",
        contract = "collect",
        out = "seen",
        scope = "block",
        key = "state",
        neoforge = "IBlockExtension.getStateAtViewpoint, IBlockStateExtension.getStateAtViewpoint"
    )]
    fn state_at_viewpoint();
}
