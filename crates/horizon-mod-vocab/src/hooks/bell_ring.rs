//! The `bell_ring` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0169

pub trait Hooks {
    /// On an authoritative server, a bell at `pos` is about to ring. Set
    /// `cancel` to stop the ring and its raider reveal.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's BellRingEvent)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_bell_ring"
    )]
    fn bell_ring();
}
