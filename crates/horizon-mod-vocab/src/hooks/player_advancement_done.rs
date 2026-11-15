//! The `player_advancement_done` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0275

pub trait Hooks {
    /// A server player completed an advancement, after progress was recorded.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "AdvancementEvent.AdvancementEarnEvent",
        cancellable = false
    )]
    fn player_advancement_done();
}
