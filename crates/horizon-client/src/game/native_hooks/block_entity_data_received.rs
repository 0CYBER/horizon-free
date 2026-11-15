//! Client synchronization notices for mod block entities.
use super::{BlockEntityRef, Game};
use mod_api::BlockEntityDataSource;
impl Game<'_> {
    /// Notice after a matching mod block entity has merged its synchronized tag.
    pub(in crate::game) fn native_block_entity_data_received(
        &self,
        entity: BlockEntityRef,
        source: BlockEntityDataSource,
        tag: &horizon_core::nbt::Compound,
    ) {
        self.fire_block_entity_data_received(entity, source, tag);
    }
}

impl Game<'_> {
    pub(in crate::game) fn notify_block_entity_data_received(
        &self,
        pos: [i32; 3],
        source: BlockEntityDataSource,
        tag: &horizon_core::nbt::Compound,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityDataReceived) {
            return;
        }
        if let Some(entity) = self
            .native_client_level()
            .and_then(|level| self.block_entity(level, pos))
        {
            self.native_block_entity_data_received(entity, source, tag);
        }
    }
}
