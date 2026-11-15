//! The `vehicle_create` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0269

pub trait Hooks {
    /// A boat or minecart was created at `pos`, after it joined the level.
    /// `kind` is the vehicle type. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's VehicleCreateEvent)"
    )]
    fn vehicle_create();
}
