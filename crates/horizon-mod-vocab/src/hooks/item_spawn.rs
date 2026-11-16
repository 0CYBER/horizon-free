//! The `item_spawn` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0232

pub trait Hooks {
    /// A dropped item entity was successfully added to the server level.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn item_spawn();
}
