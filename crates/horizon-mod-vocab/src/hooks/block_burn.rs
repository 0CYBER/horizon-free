//! The `block_burn` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0171

pub trait Hooks {
    /// On an authoritative server, fire is about to burn the block at `pos`
    /// away. Set `cancel` to keep that block.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's BlockBurnEvent)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_burn"
    )]
    fn block_burn();
}
