use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// NeoForge's `PlayerSetSpawnEvent`: a server player's respawn point is about to be
    /// set (`ServerPlayer.setRespawnPosition`), once for each call vanilla makes: every sleep
    /// in a bed (the same bed too, which refreshes its angle), an anchor that is a new
    /// point, `/spawnpoint`, and the copy to the new player when a player respawns and their
    /// respawn block was found (`pos` is none for a player with no point). A respawn block
    /// that is gone makes no call. `pos` is the block position and `dimension` its level;
    /// `forced` is `/spawnpoint`'s flag. Set `cancel` and the point stays as it was (the
    /// respawned player is left with none). `player_entity_id` is the server's entity id for
    /// the player. Server-side.
    pub(in crate::game) fn native_player_set_spawn(
        &mut self,
        call: mod_api::__private::PlayerSetSpawnCall<'_>,
    ) {
        self.fire_player_set_spawn(call);
    }
}
