use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A server entity is about to be placed by `/spreadplayers` (NeoForge's
    /// `EntityTeleportEvent.SpreadPlayersCommand`, vanilla's
    /// `SpreadPlayersCommand.setPlayerPositions`): once per entity, after the
    /// spread worked out its spot. `entity` is its network id, `player` its UUID (None for a
    /// non-player), `from` where
    /// it stands and `target` the spot. A mod may move `target` (a value that
    /// is not a number is ignored). Canceling leaves the entity where it is;
    /// the spread's other entities, its random draws and its feedback are
    /// unchanged.
    pub(in crate::game) fn native_spread_players_teleport(
        &mut self,
        dimension: &str,
        entity: u32,
        player: Option<u128>,
        from: [f64; 3],
        target: &mut [f64; 3],
        cancel: &mut bool,
    ) {
        self.fire_spread_players_teleport(dimension, entity, player, from, target, cancel);
    }
}
