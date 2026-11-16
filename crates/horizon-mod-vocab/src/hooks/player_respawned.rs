//! The `player_respawned` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0118

pub trait Hooks {
    /// `PlayerRespawnPositionEvent`: where `player` respawns, once the server has
    /// found it: the bed or anchor they stand up at (facing it), or the world
    /// spawn's free column. `original_level` and `original_position` are that
    /// point; `missing_respawn_block` says the bed or anchor was gone, so the
    /// player came back at the world spawn. Change `level`, `position`, `yaw`
    /// and `pitch` to send them elsewhere: within the level a teleport, into
    /// another level a level change. `copy_original_spawn_position` (default:
    /// the block wasn't missing) says whether they keep their respawn point for
    /// next time. Horizon places the player first and finds the point once its
    /// chunks have loaded, so this fires then, before `player_respawned` only
    /// when no chunks had to load. Not for the End exit, which isn't a respawn
    /// here (as for `player_respawned`). Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerRespawnPositionEvent"
    )]
    fn player_respawn_position(
        player: PlayerRef,
        original_level: LevelDimension,
        original_position: [f64; 3],
        missing_respawn_block: bool,
        level: &mut LevelDimension,
        position: &mut [f64; 3],
        yaw: &mut f32,
        pitch: &mut f32,
        copy_original_spawn_position: &mut bool,
    );
    /// `player` (`name`) respawned (NeoForge's `PlayerRespawnEvent`): at the
    /// end of `PlayerList.respawn`, once they are placed and their health,
    /// food and experience reset, with their state swapped in. Once for each
    /// death that ends in a respawn, on a server; not for the End-exit one.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.PlayerRespawnEvent"
    )]
    fn player_respawned();
}
