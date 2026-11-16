use super::*;

/// Server-level helpers for native mods.
#[allow(dead_code)]
impl GameLevel<'_> {
    /// This level's own random (vanilla's `Level.random`): the stream
    /// `native_level_random(dimension)` hands a server hook, for a world hook
    /// that draws as a vanilla block does (`isBonemealSuccess`), so the mod's
    /// draws shift vanilla's as a Java mod's would. `None` where this level
    /// isn't the server's copy.
    pub(in crate::game) fn native_level_random(
        &mut self,
    ) -> Option<&mut horizon_core::random::LegacyRandom> {
        self.native_authoritative.then_some(&mut self.world.random)
    }
}

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The random of the server level `level` names: vanilla's `Level.random`
    /// (`level.getRandom()`), the one its explosions, block drops and other
    /// level draws use, so a mod's draws move vanilla's later ones as a Java
    /// mod's do. A parked level (the Nether while the server ticks the
    /// Overworld) answers with its own stream. This is a copy of it as it
    /// stands: draw with `LevelRef::with_random`, which stores the stream
    /// back, so the draws move the level's own. `None` on a client of a
    /// server, for a stale handle and for a level not open: the Nether and
    /// the End open when someone first goes there and close after 300 ticks
    /// with nobody in them (`close_idle_levels`).
    pub(in crate::game) fn level_random(&self, level: LevelRef) -> Option<LegacyRandom> {
        self.native_level_world(level)?;
        if level.dimension() == self.dim.key() {
            Some((*self.world.random).clone())
        } else {
            self.levels
                .parked_world_of(level.dimension_id())
                .map(|world| (*world.random).clone())
        }
    }
    /// Replace the random of the server level `level` names with `random`,
    /// a stream `native_level_random` gave and the mod drew from. False where
    /// that gives `None`.
    pub(in crate::game) fn level_set_random(
        &mut self,
        level: LevelRef,
        random: LegacyRandom,
    ) -> bool {
        if self.native_level_world(level).is_none() {
            return false;
        }
        let target = if level.dimension() == self.dim.key() {
            Some(&mut self.world.random)
        } else {
            self.levels
                .parked_world_mut_of(level.dimension_id())
                .map(|world| &mut world.random)
        };
        target.map(|target| **target = random).is_some()
    }
}
