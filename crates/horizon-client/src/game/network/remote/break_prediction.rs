use super::*;

impl Game<'_> {
    /// MultiPlayerGameMode.destroyBlock: the predicted break.
    pub(super) fn predict_break(&mut self, pos: [i32; 3], state: BlockStateId, sequence: i32) {
        self.tutorial_destroy(state, 1.0);
        // BlockStatePredictionHandler.retainKnownServerState.
        let here = self.player_position();
        if let Some(r) = self.remote.as_mut() {
            r.predictions
                .entry(pos)
                .and_modify(|p| p.sequence = sequence)
                .or_insert(Prediction {
                    sequence,
                    state: state.0 as i32,
                    player_pos: here,
                });
        }
        // Block.playerWillDestroy's destroy event (particles and the break
        // sound), which BaseFireBlock skips: the server's extinguish sound
        // is all a player hears.
        if !self.res.facts().get(state).has_class("BaseFireBlock") {
            self.play_break_sound(pos, state);
            self.destroy_particles(pos, state);
        }
        // The server's block update follows (waterlogged blocks leave water).
        let air = self.world.air;
        if self.world.set(pos[0], pos[1], pos[2], air).is_ok() {
            self.world.block_entities.remove(&(pos[0], pos[1], pos[2]));
            self.client_block_written(pos, state, air, true);
        }
        self.mining = None;
    }
}
