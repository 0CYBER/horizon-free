//! Replacement state for vanilla's Level.removeBlock.
pub trait Hooks {
    /// Choose the state left by removing your block. Return a state in `answer`
    /// to replace vanilla's fluid block (air when dry), or leave it unanswered.
    /// Only the removed block's Part is asked, on the authoritative server.
    /// The replacement must be a registered block state; an invalid answer is
    /// refused with a diagnostic and vanilla's replacement is written instead.
    /// The normal setBlock effects and flags 3 (+64 when moved by a piston)
    /// apply to the written state. The level is read-only during this call.
    #[hook(
        side = "server",
        contract = "replace",
        out = "answer",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::remove_block::GameLevel<'_>::native_remove_block",
        neoforge = "none (vanilla Level.removeBlock; no NeoForge event covers this call)"
    )]
    fn remove_block(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        moved_by_piston: bool,
        answer: &mut Option<BlockState>,
    );
}
