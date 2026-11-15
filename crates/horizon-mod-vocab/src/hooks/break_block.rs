//! The `break_block` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0038

pub trait Hooks {
    /// A player breaks the block `state` at `pos`, once the game allowed it
    /// (the held item can break blocks, the debug stick didn't take the
    /// click): where `ServerPlayerGameMode.destroyBlock` starts. Set
    /// `canceled` to keep the block: nothing is destroyed or dropped, and a
    /// guest sees it back. Set `exp_to_drop` to `Some(amount)` to replace
    /// the experience sample later, after item drops are prepared;
    /// `None` keeps sampling. A subscribed block part's `exp_drop` answer is
    /// supplied as this event's initial amount. `block_drops` sees and may change that
    /// amount afterwards; cancellation or disabled block drops suppresses it.
    /// Server-side, for every player of the world (the
    /// helpers read the acting one). Diverges from upstream: renamed from
    /// `destroy_block`, to NeoForge's `BreakBlockEvent`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "BreakBlockEvent"
    )]
    fn break_block();
}
