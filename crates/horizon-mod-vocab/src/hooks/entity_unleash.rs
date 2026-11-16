//! The `entity_unleash` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0210

pub trait Hooks {
    /// A leashed mob's leash was removed, before any lead drop is handled
    /// (Bukkit's `EntityUnleashEvent`).
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's EntityUnleashEvent)"
    )]
    fn entity_unleash();
}
