//! The `is_signal_source` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0055

pub trait Hooks {
    /// `BlockState.isSignalSource`: set `source` for a block of yours that
    /// gives a signal, so redstone wire beside it connects to it. Authoritative
    /// servers only.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_is_signal_source",
        neoforge = "none (vanilla BlockBehaviour.isSignalSource; NeoForge adds no event or extension for it)"
    )]
    fn is_signal_source();
}
