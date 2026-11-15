//! The `respawn_position` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Where a player whose respawn point is the block `state` at `pos` stands up when they
    /// respawn there (`IBlockExtension.getRespawnPosition`), or that they can't respawn there.
    /// `entity_type` is the respawning entity's type, always `minecraft:player`, and
    /// `orientation` the yaw saved with the respawn point. `position` starts at vanilla's answer:
    /// a bed's or a charged respawn anchor's stand-up spot facing the block, a forced point's
    /// free column, or empty where vanilla finds no spot. Set `Some((position, yaw, pitch))` to
    /// stand the player there, or `None` to send them to the world spawn with
    /// `missing_respawn_block` set. An answer with a non-finite value is ignored. An anchor still
    /// loses a charge only when the answer is a spot. Asked on the server as the respawn point
    /// is resolved, before `player_respawn_position`, which receives the answer as its original
    /// position and may still redirect it, and as a player leaves the End, which resolves the
    /// point the same way. The level is read-only. NeoForge asks only beds; this is asked
    /// for any block, so a mod block that is neither a bed nor an anchor can be a respawn point.
    /// With no part attached to the state, the game checks the hook subscription and the block
    /// key before any event conversion, so the cost is one table lookup per respawn. Diverges
    /// from upstream: new hook.
    #[hook(
        side = "server",
        contract = "collect",
        out = "position",
        scope = "block",
        key = "state",
        host = "read",
        neoforge = "IBlockExtension.getRespawnPosition, IBlockStateExtension.getRespawnPosition"
    )]
    fn respawn_position();
}
