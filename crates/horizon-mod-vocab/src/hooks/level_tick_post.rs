//! The `level_tick_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0011

pub trait Hooks {
    /// After a successful level tick pass on either logical side; never after a failed tick.
    /// `host.server_has_time()` says whether optional work still fits in the tick
    /// (`LevelTickEvent.hasTime`); always true on a client.
    #[hook(side = "both", contract = "listen", neoforge = "LevelTickEvent.Post")]
    fn level_tick_post();
}
