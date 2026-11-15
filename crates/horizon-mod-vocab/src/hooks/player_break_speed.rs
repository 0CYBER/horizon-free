//! The `player_break_speed` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player's destroy speed for the block at `pos` may be changed or canceled.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "PlayerEvent.BreakSpeed",
        cancellable = true
    )]
    fn player_break_speed();
}
