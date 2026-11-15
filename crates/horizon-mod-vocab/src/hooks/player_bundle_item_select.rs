//! The `player_bundle_item_select` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0281

pub trait Hooks {
    /// A server player selected a bundle item in `slot` at `index`. The guest's
    /// state is not swapped in during packet handling.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerBundleItemSelect)"
    )]
    fn player_bundle_item_select();
}
