//! The `entity_enter_love_mode` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0203

pub trait Hooks {
    /// A mob entered love mode after player interaction. `player` is the
    /// feeder when known.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_enter_love_mode();
}
