//! The `player_spawn_phantoms` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `PlayerSpawnPhantomsEvent`: the phantom spawner is about to decide whether to try a
    /// spawn near a non-spectator server player, before its difficulty and rest rolls.
    /// `result` starts empty, meaning `Default` (skylight and sea level check); `Allow` skips
    /// that check and `Deny` stops the attempt. `phantoms` starts with a randomly drawn
    /// group size; set it to override the count. Without mods the original draw order is kept.
    #[hook(
        side = "server",
        contract = "replace",
        out = "result",
        neoforge = "PlayerSpawnPhantomsEvent",
        cancellable = false
    )]
    fn player_spawn_phantoms();
}
