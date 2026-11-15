use super::*;

/// The level's side of fire and lava ignition (`FireBlock`, `LavaFluid`), which `GameLevel`'s
/// `LevelAccess` forwards to: the mods' answers on an authoritative server, and the level's own
/// facts. A client's copy of the level asks no mod.
impl GameLevel<'_> {
    /// `allow_block_ignite`: the `block_ignite` hook's cancel.
    pub(in crate::game) fn block_ignite_allowed(&self, pos: BlockPos, cause: &'static str) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_block_ignite(self.dimension, self.native_player, pos, cause, &mut cancel);
        }
        !cancel
    }

    /// `allow_block_burn`: the `block_burn` hook's cancel.
    pub(in crate::game) fn block_burn_allowed(&self, pos: BlockPos) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_block_burn(self.dimension, pos, &mut cancel);
        }
        !cancel
    }

    /// `before_fire_odds`: the block's parts answer `fire_odds`.
    pub(in crate::game) fn answer_fire_odds(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        face: Direction,
        odds: &mut Option<(i32, i32)>,
    ) {
        if self.native_authoritative {
            self.native_fire_odds(self.dimension, state, pos, face, odds);
        }
    }

    /// `before_is_fire_source`: the block's parts answer `is_fire_source`.
    pub(in crate::game) fn answer_is_fire_source(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        source: &mut Option<bool>,
    ) {
        if self.native_authoritative {
            self.native_is_fire_source(self.dimension, state, pos, source);
        }
    }

    /// `before_ignited_by_lava`: the block's parts answer `ignited_by_lava`.
    pub(in crate::game) fn answer_ignited_by_lava(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        face: Direction,
        ignites: &mut Option<bool>,
    ) {
        if self.native_authoritative {
            self.native_ignited_by_lava(self.dimension, state, pos, face, ignites);
        }
    }

    /// `on_caught_fire`: the burned block's parts hear `on_caught_fire`.
    pub(in crate::game) fn caught_fire(
        &mut self,
        ticks: &mut Ticks,
        state: BlockStateId,
        pos: BlockPos,
        face: Direction,
    ) {
        if self.native_authoritative {
            self.native_on_caught_fire(ticks, self.dimension, state, pos, face);
        }
    }

    /// `before_smoke_source`: the block's parts answer `smoke_source`.
    pub(in crate::game) fn answer_smoke_source(
        &self,
        state: BlockStateId,
        smoke: &mut Option<bool>,
    ) {
        if self.native_authoritative {
            self.native_smoke_source(self.dimension, state, smoke);
        }
    }

    /// `ServerLevel.canSpreadFireAround`: a player (not a spectator) closer
    /// than the `fire_spread_radius_around_player` rule to the block's corner
    /// (`ChunkMap.anyPlayerCloseEnoughTo`); -1 spreads everywhere.
    pub(in crate::game) fn player_near_fire(&self, pos: BlockPos) -> bool {
        let radius = self.rules.fire_spread_radius_around_player;
        radius == -1
            || self.fire_players.iter().any(|p| {
                let d = [
                    p[0] - f64::from(pos.x),
                    p[1] - f64::from(pos.y),
                    p[2] - f64::from(pos.z),
                ];
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < f64::from(radius) * f64::from(radius)
            })
    }

    /// The `INCREASED_FIRE_BURNOUT` environment attribute of the biome at `pos`.
    pub(in crate::game) fn biome_burns_out_fire(&self, pos: BlockPos) -> bool {
        self.world
            .chunks
            .get(&(pos.x >> 4, pos.z >> 4))
            .is_some_and(|chunk| {
                self.biomes
                    .get(chunk.biome(pos.x, pos.y, pos.z))
                    .increased_fire_burnout
            })
    }
}
