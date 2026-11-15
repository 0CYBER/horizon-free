//! The `player_ready_arrow` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0303

pub trait Hooks {
    /// A server player released a drawn bow with `arrow` selected, by the use-item
    /// key or a mod's `LivingRef::release_using_item`. The player's state is swapped in.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerReadyArrow)"
    )]
    fn player_ready_arrow();
}
