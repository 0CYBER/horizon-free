use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// How far from the camera the mod block entity of block `block` draws, in blocks
    /// (`BlockEntityRenderer.getViewDistance`): 64 unless a part answers. Asked once for
    /// each mod block, the first time this client draws block entities, and kept.
    pub(in crate::game) fn native_block_entity_view_distance(
        &self,
        block: &str,
        distance: &mut f64,
    ) {
        self.fire_block_entity_view_distance(block, distance);
    }
}
