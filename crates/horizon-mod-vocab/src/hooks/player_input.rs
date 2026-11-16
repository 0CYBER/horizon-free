//! The `player_input` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0289

pub trait Hooks {
    /// A guest sent the server's current `PlayerInput` flags, after they were applied to the hub. The guest's state is swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerInputEvent)"
    )]
    fn player_input();
}
