use super::*;

impl GameLevel<'_> {
    /// After the burn roll and cancel gates, before replacing `state` at `pos`.
    /// Listener writes at `pos` are overwritten; TNT primes after replacement.
    #[inline(always)]
    pub(in crate::game) fn native_on_caught_fire(
        &mut self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        state: BlockStateId,
        pos: BlockPos,
        face: Direction,
    ) {
        self.fire_on_caught_fire(ticks, dimension, state, pos, face);
    }
}
