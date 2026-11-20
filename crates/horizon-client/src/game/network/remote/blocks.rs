//! Block entities and block updates from the server, and the predictions they end.

use super::*;
// The point generator reads a signature's types through named imports only, not the glob.
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    /// Keep the renderable block entities (chests, furnaces) in step with blocks.
    /// `previous`: the block's state before, when it changed. A column sent again has
    /// none, and keeps its block entities' synced attachments: the server does not send
    /// them again (`remote_chunk` drops those of block entities the column lost).
    pub(super) fn sync_block_entity_body(
        &mut self,
        pos: (i32, i32, i32),
        previous: Option<BlockStateId>,
        state: BlockStateId,
    ) {
        let name = self.res.facts().get(state).name.clone();
        // LevelChunk.setBlockState: another block's block entity goes, even
        // one of the same type (a mod's blocks may share one).
        let other_block = previous.is_some_and(|p| self.res.facts().get(p).name != name);
        self.ensure_stateful_entity_table();
        // State-aware factories decide creation only. A valid same-block
        // instance survives even when the new state would create none.
        if !other_block
            && self.world.block_entities.stateful_entity(state.0)
            && matches!(
                self.world.block_entities.get(&pos),
                Some(BlockEntity::Mod(_))
            )
        {
            return;
        }
        // A mod block's is the mod's own, as on the server.
        match self.new_block_entity_for_state(state) {
            Some(entity) => {
                let same = self.world.block_entities.get(&pos).is_some_and(|e| {
                    e.id() == entity.id() && !(other_block && matches!(e, BlockEntity::Mod(_)))
                });
                if !same {
                    if previous.is_some() {
                        self.forget_block_entity_attachments(pos);
                    }
                    self.world.block_entities.replace(pos, entity);
                }
            }
            None => {
                if previous.is_some() {
                    self.forget_block_entity_attachments(pos);
                }
                self.world.block_entities.remove(&pos);
            }
        }
    }
    /// The server's synced attachments of a block entity that goes with it.
    pub(super) fn forget_block_entity_attachments_body(&mut self, (x, y, z): (i32, i32, i32)) {
        if let Some(r) = self.remote.as_mut() {
            r.attachments.remove(&Holder::BlockEntity([x, y, z]));
        }
    }
    /// ClientLevel.setServerVerifiedBlockState: a block the client predicted keeps its
    /// predicted state until the server acknowledges the action; the server's state is
    /// remembered for then.
    pub(super) fn server_block_body(&mut self, pos: [i32; 3], state: i32) {
        if let Some(p) = self
            .remote
            .as_mut()
            .and_then(|r| r.predictions.get_mut(&pos))
        {
            p.state = state;
            return;
        }
        self.set_block(pos, state, false);
    }
    /// BlockStatePredictionHandler.endPredictionsUpTo (ClientboundBlockChangedAckPacket): the
    /// actions up to `sequence` are settled, so the blocks they predicted take the state the
    /// server gave them (a break the server refused comes back).
    pub(super) fn end_predictions_body(&mut self, sequence: i32) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let last_teleport = r.last_teleport_sequence;
        let settled: Vec<_> = r
            .predictions
            .iter()
            .filter(|(_, p)| p.sequence <= sequence)
            .map(|(&pos, &p)| (pos, p))
            .collect();
        for (pos, prediction) in settled {
            if let Some(r) = self.remote.as_mut() {
                r.predictions.remove(&pos);
            }
            if self.world.block(pos[0], pos[1], pos[2]).0 as i32 == prediction.state {
                continue;
            }
            self.set_block(pos, prediction.state, false);
            // ClientLevel.syncBlockState: the block that comes back may now hold
            // the player, and then it goes to the position the prediction
            // remembered.
            let meets = self.block_meets_player(pos, prediction.state);
            if let Some(at) = snap_target(prediction.player_pos, last_teleport, sequence, meets) {
                self.snap_player_to(at);
            }
        }
    }
    /// The player's position, for `retainKnownServerState`'s `player.position()`.
    pub(super) fn player_position_body(&self) -> [f64; 3] {
        let at = self
            .player
            .as_ref()
            .map(|p| p.position)
            .unwrap_or(Vec3::ZERO);
        [at.x, at.y, at.z]
    }
    /// `Entity.isColliding(pos, state)` for the local player: the state's own
    /// collision shape, at the player's pose, against the player's box.
    pub(super) fn block_meets_player_body(&self, pos: [i32; 3], state: i32) -> bool {
        let Some(player) = self.player.as_ref() else {
            return false;
        };
        let entity = player.bounds();
        let context = horizon_world::collision_context::CollisionContext::for_entity(
            horizon_world::collision_context::CollisionEntity {
                identity: horizon_world::entity_obstruction::EntityIdentity(0),
                kind: horizon_world::collision_context::EntityContextKind::Player,
                y: player.position.y,
                descending: player.shift_key_down,
                main_hand: None,
            },
            false,
        );
        let mut scene = horizon_client::collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        match self.collision.collision_shape_with_context(
            &mut scene,
            BlockStateId(state.max(0) as u32),
            at,
            &context,
        ) {
            Ok(shape) => shape_meets(&shape, pos, entity),
            Err(_) => false,
        }
    }
    /// `Entity.absSnapTo`: the remembered point, clamped to the level's edge.
    /// The velocity is left alone, and the player has no separate previous
    /// position to reset (its box is drawn from `position`).
    pub(super) fn snap_player_to_body(&mut self, at: [f64; 3]) {
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let x = at[0].clamp(-3.0e7, 3.0e7);
        let z = at[2].clamp(-3.0e7, 3.0e7);
        player.position = Vec3::new(x, at[1], z);
    }
    pub(super) fn set_block_body(&mut self, pos: [i32; 3], state: i32, predicted: bool) {
        // A state we don't have (a server with mods we lack) reads as air, as in its chunks.
        let state = state.max(0) as u32;
        let known = horizon_client::chunk::known_block_state(state);
        let state = BlockStateId(if known { state } else { 0 });
        let previous = self.world.block(pos[0], pos[1], pos[2]);
        if self.world.loaded(pos[0], pos[2])
            && self.world.set(pos[0], pos[1], pos[2], state).is_ok()
        {
            self.sync_block_entity((pos[0], pos[1], pos[2]), Some(previous), state);
            self.client_block_written(pos, previous, state, predicted);
        }
        // A block we just placed: play its place sound (the server skips the placer).
        if let Some((target, _)) = self.remote.as_ref().and_then(|r| r.placing)
            && target == pos
            && !self.res.facts().is_air(state)
        {
            let s = self.own_sound_type(pos, state, mod_api::BlockSoundCause::Place);
            let at = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ];
            self.play_client_local_sound(
                LevelSound::new(&s.place, at, (s.volume + 1.0) / 2.0, s.pitch * 0.8)
                    .in_source(mod_api::SoundSource::Blocks),
            );
            if let Some(r) = self.remote.as_mut() {
                r.placing = None;
            }
        }
    }
}
