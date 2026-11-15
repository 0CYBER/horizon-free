//! The `player_shear_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0308

pub trait Hooks {
    /// A player shearing a mob is about to do so. Set `cancel` to leave the mob and shears unchanged.
    /// The guest's state is swapped in.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerShearEntityEvent)",
        cancellable = true
    )]
    fn player_shear_entity();
}
