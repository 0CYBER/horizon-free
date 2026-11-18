//! The `living_equipment_listened` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0158

pub trait Hooks {
    /// No longer needed: the game reads every mob's and connected player's
    /// equipment each tick, and tells `living_equipment_change`, whenever an
    /// enabled mod subscribes to that hook. A mod that still sets `listened`
    /// (HEAD, `*listened = true`) is honoured as before. Without either the
    /// game does not read the equipment, and neither is told (armor stands
    /// still are). A mob that ticks, or a player who is in the level, once it
    /// is read counts from empty. Asked once a tick for the mobs, after their
    /// tick, and once for the players. Server-side.
    #[hook(
        side = "server",
        contract = "collect",
        out = "listened",
        cancellable = false,
        neoforge = "none (Horizon-only; NeoForge has no counterpart); an opt-in gate for a hook"
    )]
    fn living_equipment_listened();
}
