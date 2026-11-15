use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `PlayerSpawnPhantomsEvent`: the phantom spawner is about to decide whether to try a
    /// spawn near a non-spectator server player, before its difficulty and rest rolls.
    /// `result` starts empty, meaning `Default` (skylight and sea level check); `Allow` skips
    /// that check and `Deny` stops the attempt. `phantoms` starts with a randomly drawn
    /// group size; set it to override the count. Without mods the original draw order is kept.
    pub(in crate::game) fn native_player_spawn_phantoms(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        result: &mut Option<PhantomSpawnResult>,
        phantoms: &mut Option<i32>,
    ) {
        self.fire_player_spawn_phantoms(player, name, pos, result, phantoms);
    }
}
