//! The `vehicle_damage` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0270

pub trait Hooks {
    /// A vehicle is about to take damage from a player's attack. `kind` is
    /// `boat` or the minecart type; mods may change `damage` or set `cancel`
    /// to leave its damage and wobble unchanged. The attack itself still
    /// completes. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's VehicleDamageEvent)",
        cancellable = true
    )]
    fn vehicle_damage();
}
