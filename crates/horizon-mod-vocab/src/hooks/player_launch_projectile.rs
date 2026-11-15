//! The `player_launch_projectile` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0295

pub trait Hooks {
    /// A server player launched a projectile item, after it was created.
    /// `item` is the projectile's held item.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerLaunchProjectile)"
    )]
    fn player_launch_projectile();
}
