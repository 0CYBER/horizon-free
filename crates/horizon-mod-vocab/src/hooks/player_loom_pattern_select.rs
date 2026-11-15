//! The `player_loom_pattern_select` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0297

pub trait Hooks {
    /// A server player selected a loom pattern, after the loom menu accepted the button. `pattern` is
    /// the zero-based button index. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_loom_pattern_select();
}
