//! The `player_stop_using_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0313

pub trait Hooks {
    /// A server player released the use-item key, or a mod's `LivingRef::release_using_item` did,
    /// before the item finished being used. The player's state is swapped in.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerStopUsingItem)"
    )]
    fn player_stop_using_item();
}
