//! The `block_lock_check` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0179

pub trait Hooks {
    /// A guest's block inventory lock check at `pos`. `locked` is the
    /// vanilla result and may be changed. The guest's state is not swapped in.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "locked",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_lock_check();
}
