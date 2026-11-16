//! The `furnace_burn` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0220

pub trait Hooks {
    /// A furnace at `pos` consumed fuel and began or resumed burning it.
    /// `fuel` is the item consumed from its fuel slot. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn furnace_burn();
}
