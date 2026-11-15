use super::*;

impl Game<'_> {
    /// `IBlockExtension.getBeaconColorMultiplier`: the colour `state` at `pos` gives the beam of
    /// the beacon at `beacon_pos`. `color` is the block's own answer, which its parts may change.
    pub(in crate::game) fn native_beacon_color_multiplier(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        beacon_pos: BlockPos,
        color: &mut Option<u32>,
    ) {
        self.fire_beacon_color_multiplier(state, pos, beacon_pos, color);
    }
}
