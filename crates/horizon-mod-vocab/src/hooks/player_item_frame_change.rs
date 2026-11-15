//! The `player_item_frame_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0255

pub trait Hooks {
    /// A server player changed an item frame after its contents or rotation changed. `action` is
    /// `put`, `rotate`, `remove_item` or `break`; `item` is empty when no item remains, and `rotation`
    /// is the resulting step. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_item_frame_change();
}
