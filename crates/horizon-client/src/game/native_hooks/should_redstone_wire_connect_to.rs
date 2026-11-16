use super::*;

impl GameLevel<'_> {
    /// `BlockBehaviour.shouldRedstoneWireConnectTo`: set `connected` to say
    /// whether redstone wire connects to the block `state` at `pos`.
    /// `direction` points from the wire toward the neighbouring block, or is
    /// None when the wire asks about the block above or below. `connected` holds vanilla's answer: change
    /// it when the block is yours. Asked whenever wire settles its shape:
    /// answer from `state` and `pos` without touching the world. When the
    /// answer changes, and once the wire is placed, update its neighbours
    /// (`level.update_neighbors`). Authoritative servers only.
    #[inline(always)]
    pub(in crate::game) fn native_should_redstone_wire_connect_to(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        direction: Option<Direction>,
        connected: &mut bool,
    ) {
        self.fire_should_redstone_wire_connect_to(dimension, pos, state, direction, connected);
    }
}
