use super::*;

impl Game<'_> {
    /// `IBlockExtension.getRespawnPosition`: where an entity of type `entity_type` whose
    /// respawn point is `state` at `pos`, saved facing `orientation`, stands up. `position` is
    /// vanilla's answer (position, yaw, pitch), which the block's parts may change.
    pub(in crate::game) fn native_respawn_position(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        entity_type: &str,
        orientation: f32,
        position: &mut Option<([f64; 3], f32, f32)>,
    ) {
        self.fire_respawn_position(state, pos, entity_type, orientation, position);
    }
}
