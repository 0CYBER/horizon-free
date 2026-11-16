//! The `entity_drop_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0201

pub trait Hooks {
    /// A mob is about to drop `count` of `item` at `pos` (Bukkit's
    /// `EntityDropItemEvent`).
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's EntityDropItemEvent)"
    )]
    fn entity_drop_item();
}
