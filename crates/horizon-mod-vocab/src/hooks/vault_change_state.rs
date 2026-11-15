//! The `vault_change_state` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0339

pub trait Hooks {
    /// A vault at `pos` changed to `state`. Its block state is updated.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn vault_change_state();
}
