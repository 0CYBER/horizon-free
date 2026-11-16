//! The `block_ignite` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0178

pub trait Hooks {
    /// On an authoritative server, fire is about to be placed at `pos` by
    /// flint and steel, a fire charge, lava, lightning or fire spread. Set
    /// `player` is the acting server player when there is one. Set `cancel` to
    /// leave the position unchanged.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's BlockIgniteEvent)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_ignite"
    )]
    fn block_ignite();
}
