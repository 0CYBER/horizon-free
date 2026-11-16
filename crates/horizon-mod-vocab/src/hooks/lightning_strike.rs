//! The `lightning_strike` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0234

pub trait Hooks {
    /// A server lightning bolt is about to be created. `cause` is `weather`,
    /// `trident`, `command` or `mod` (a native mod's `strike_lightning`); `player` is the responsible player when known.
    /// Set `cancel` to prevent the bolt and its effects.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn lightning_strike();
}
