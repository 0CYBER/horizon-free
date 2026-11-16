/// Original upstream notes; the existing fork event above owns this moment.
pub trait UpstreamNotes {
    #[notes = r###"a dead player's respawn point: `native_player_respawn_position(player, name, dimension, pos, yaw, pitch)` is heard once per respawn before the player is placed (NeoForge's `PlayerRespawnPositionEvent`, Bukkit's `PlayerRespawnEvent.setRespawnLocation`), with their bed, anchor, forced point or the world spawn. A changed dimension or position places the player exactly there, with no bed or anchor used; yaw and pitch set their look. An unknown dimension or a non-finite value leaves that part unchanged. Bundles that use it need `"hooks": @API:player_respawn_position@`."###]
    fn player_respawn_position();
}
