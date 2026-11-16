use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A vanilla game event is about to reach its listeners (NeoForge's
    /// `VanillaGameEvent`, vanilla's `Level.gameEvent`): every event, in
    /// `dimension`. `event` is its id (`minecraft:step`), `pos` where it
    /// happened and `cause` the network id of the entity that caused it
    /// (None for none). `affected_state` is the block state the event touched
    /// (`GameEvent.Context.affectedState`, with `cause` its `sourceEntity`): the
    /// block a player broke or placed, the block a mob stepped on; None where
    /// vanilla passes none (`Context.of(entity)`).
    /// Canceling keeps the sculk sensors, wardens and allays from hearing it.
    /// Server-side.
    pub(in crate::game) fn native_vanilla_game_event(
        &mut self,
        dimension: &str,
        event: &str,
        pos: [f64; 3],
        cause: Option<u32>,
        affected_state: Option<BlockStateId>,
        cancel: &mut bool,
    ) {
        self.fire_vanilla_game_event(dimension, event, pos, cause, affected_state, cancel);
    }
}
