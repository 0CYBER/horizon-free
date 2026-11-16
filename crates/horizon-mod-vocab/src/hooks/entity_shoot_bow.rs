//! The `entity_shoot_bow` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0206

pub trait Hooks {
    /// A player bow or crossbow launched an arrow with `force`, after the
    /// projectile entered the level.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_shoot_bow();
}
