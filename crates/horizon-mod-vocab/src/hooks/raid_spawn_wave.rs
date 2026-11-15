//! The `raid_spawn_wave` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0328

pub trait Hooks {
    /// A raid wave has been spawned at `pos`; `wave` is its one-based wave
    /// number and `raiders` counts the raiders, not the mobs they ride.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn raid_spawn_wave();
}
