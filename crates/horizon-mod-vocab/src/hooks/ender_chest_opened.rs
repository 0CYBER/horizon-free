//! The `ender_chest_opened` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0352

pub trait Hooks {
    /// A guest opened an ender chest: the menu shows the guest's own inventory. Heard after
    /// `container_opened` let the menu open, so a cancelled open is not heard. `player` is the
    /// guest's UUID and `pos` the chest's block position. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn ender_chest_opened();
}
