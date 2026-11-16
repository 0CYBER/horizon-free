//! The `loot_function` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0136

pub trait Hooks {
    /// A loot function of a mod's own kind: the entry named a `type` in a
    /// namespace outside `minecraft:`, and the roll kept it for this call.
    /// The kind and JSON file input accompany owned API context and candidate snapshots.
    /// Candidate counts remain signed; ordered NBT patches preserve removals and types.
    /// Conversion happens only for subscribed hooks, and refused writeback leaves the roll unchanged.
    /// `tags` reads membership and ordered members by key from this evaluation's
    /// authoritative merged tags. No enumeration or full Host is exposed.
    #[hook(
        side = "server",
        version = 2,
        contract = "collect",
        out = "stack",
        target = "mod_api::loot_callbacks::native_loot_function",
        neoforge = "RegisterEvent"
    )]
    fn loot_function();
}
