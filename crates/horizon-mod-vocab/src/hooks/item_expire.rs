//! The `item_expire` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0231

pub trait Hooks {
    /// A dropped item reaches its lifetime on the server (`ItemEntity.tick`,
    /// NeoForge's `ItemExpireEvent`). Every mod may set or add to `extra_life`,
    /// in ticks, which starts at zero; a positive total keeps the item that much
    /// longer, zero or less lets it expire. Extensions are capped at 32766
    /// ticks.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ItemExpireEvent",
        cancellable = false
    )]
    fn item_expire();
}
