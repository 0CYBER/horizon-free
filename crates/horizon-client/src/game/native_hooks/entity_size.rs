use super::*;

impl Game<'_> {
    /// `EntityEvent.Size`: the size vanilla's `Entity.refreshDimensions` just computed for
    /// `entity` in `pose`, before its box and eyes use it (`modding/entity_sizes.rs`).
    pub(in crate::game) fn native_entity_size(
        &self,
        entity: EntityRef,
        pose: mod_api::Pose,
        old_size: [f32; 3],
        size: &mut [f32; 3],
    ) {
        self.fire_entity_size(entity, pose, old_size, size);
    }
}
