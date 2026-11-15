//! The `player_toggle_sneak` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0317

pub trait Hooks {
    /// A guest's client changed sneaking in its player input packet, after the hub applied the new state.
    /// The guest's state is swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerToggleSneakEvent)"
    )]
    fn player_toggle_sneak();
}
