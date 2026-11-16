use super::*;

/// The client's world for a guess at what a click does: reads go to the
/// world, writes stay here and are dropped.
pub(super) struct DryLevel<'w> {
    base: &'w dyn horizon_world::level::LevelAccess,
    pub(super) changed: HashMap<horizon_core::position::BlockPos, BlockStateId>,
    random: horizon_core::random::LegacyRandom,
    /// The sounds the use plays (a door's creak).
    pub(super) sounds: Vec<DrySound>,
}

impl<'w> DryLevel<'w> {
    pub(super) fn new(base: &'w dyn horizon_world::level::LevelAccess) -> Self {
        Self {
            base,
            changed: HashMap::new(),
            random: horizon_core::random::LegacyRandom::new(0),
            sounds: Vec::new(),
        }
    }
}

impl horizon_world::level::BlockEntityNeighbors for DryLevel<'_> {}

impl horizon_world::level::LevelAccess for DryLevel<'_> {
    fn cake_can_eat(&self, vanilla: bool) -> bool {
        self.base.cake_can_eat(vanilla)
    }
    fn emit(&mut self, event: horizon_world::level::LevelEvent) {
        if let horizon_world::level::LevelEvent::Sound {
            pos,
            sound,
            volume,
            pitch,
        } = event
        {
            self.sounds.push((pos, sound, volume, pitch));
        }
    }
    fn block_state(&self, pos: horizon_core::position::BlockPos) -> BlockStateId {
        self.changed
            .get(&pos)
            .copied()
            .unwrap_or_else(|| self.base.block_state(pos))
    }
    fn set_raw(
        &mut self,
        pos: horizon_core::position::BlockPos,
        state: BlockStateId,
    ) -> Option<BlockStateId> {
        let previous = self.block_state(pos);
        self.changed.insert(pos, state);
        Some(previous)
    }
    fn is_loaded(&self, pos: horizon_core::position::BlockPos) -> bool {
        self.base.is_loaded(pos)
    }
    /// `Level.isClientSide`: the preview keeps its live level's side.
    fn is_client_side(&self) -> bool {
        self.base.is_client_side()
    }
    /// A block this guess changed has no block entity left, so it reads as a
    /// missing one; every other position asks the level it stands in for.
    fn shulker_is_closed(&self, pos: horizon_core::position::BlockPos) -> Option<bool> {
        if self.changed.contains_key(&pos) {
            return None;
        }
        self.base.shulker_is_closed(pos)
    }
    fn min_y(&self) -> i32 {
        self.base.min_y()
    }
    fn max_y(&self) -> i32 {
        self.base.max_y()
    }
    fn block_light(&self, pos: horizon_core::position::BlockPos) -> i32 {
        self.base.block_light(pos)
    }
    fn sky_light(&self, pos: horizon_core::position::BlockPos) -> i32 {
        self.base.sky_light(pos)
    }
    fn sky_darken(&self) -> i32 {
        self.base.sky_darken()
    }
    fn is_unobstructed(&self, pos: horizon_core::position::BlockPos, state: BlockStateId) -> bool {
        self.base.is_unobstructed(pos, state)
    }
    fn random(&mut self) -> &mut horizon_core::random::LegacyRandom {
        &mut self.random
    }
    /// A mod's bone meal answer is read, never grown here: the preview asks
    /// the level it stands in for, as `isValidBonemealTarget` is read on both
    /// sides.
    fn is_valid_bonemeal_target(
        &self,
        pos: horizon_core::position::BlockPos,
        state: BlockStateId,
    ) -> Option<bool> {
        self.base.is_valid_bonemeal_target(pos, state)
    }
    fn game_time(&self) -> i64 {
        self.base.game_time()
    }
}
