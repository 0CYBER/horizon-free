//! The `piston_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0041

pub trait Hooks {
    /// On an authoritative server, the piston at `piston` is about to push
    /// the blocks at `push` one step `direction` (`[0, 1, 0]` is up; pulling
    /// when it retracts, `extending` false) and destroy those at `destroy`
    /// (NeoForge's `PistonEvent.Pre`, in `PistonBaseBlock.moveBlocks`, once its
    /// structure resolved). Set
    /// `canceled` and nothing moves, as when the structure can't; a sticky
    /// piston then retracts without its block, and clients are sent the
    /// blocks rather than the retraction to replay. Diverges from upstream: `piston_move` is renamed to
    /// `piston_pre`, and `piston_post` is new.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "PistonEvent.Pre",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_piston_pre"
    )]
    fn piston_pre(
        dimension: LevelDimension,
        piston: BlockPos,
        direction: [i32; 3],
        extending: bool,
        push: &[BlockPos],
        destroy: &[BlockPos],
        canceled: &mut bool,
    );
    /// On an authoritative server, the piston at `piston` pushed the blocks at
    /// `push` one step `direction` (`[0, 1, 0]` is up; pulling when it
    /// retracts, `extending` false) and destroyed those at `destroy`
    /// (NeoForge's `PistonEvent.Post`, at the end of `PistonBaseBlock.moveBlocks`,
    /// after the neighbour updates). It only hears; `piston_pre` is the veto.
    /// Server-side. Diverges from upstream: `piston_move` is split into
    /// `piston_pre` and `piston_post`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PistonEvent.Post",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_piston_post"
    )]
    fn piston_post();
}
