//! The `calculate_player_turn` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The mouse sensitivity and cinematic camera option a grabbed player's
    /// mouse turn reads this frame (`MouseHandler.turnPlayer`). Change either
    /// for this turn only; the options are unchanged. Client-side. This shapes how far the
    /// mouse turns the view; `ClientHost::set_local_rotation` sets the view directly, and the
    /// two meet in the same angles the tick's move packet sends.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "CalculatePlayerTurnEvent",
        cancellable = false
    )]
    fn calculate_player_turn();
}
