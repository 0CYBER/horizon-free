//! The `player_edit_book` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0285

pub trait Hooks {
    /// A server player's book edit or signing packet is about to be applied. `pages` is its page
    /// count; `signing` means the packet has a title. Set `cancel` to drop the edit.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerEditBookEvent)",
        cancellable = true
    )]
    fn player_edit_book();
}
