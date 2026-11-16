//! The `player_flyable_fall` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `PlayerFlyableFallEvent`: server player `player` (`name`) landed after
    /// falling `distance` blocks at damage multiplier `multiplier` while allowed to fly (a
    /// creative or spectator player walking), so `Player.causeFallDamage` returned early
    /// and nothing was hurt. Listen only. Once for each landing, after the fall's
    /// block reaction. The player's state is swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerFlyableFallEvent"
    )]
    fn player_flyable_fall();
}
