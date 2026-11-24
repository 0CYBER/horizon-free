use super::Game;
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    pub(in crate::game) fn native_block_entity_scene_tick(
        kind: mod_api::BlockEntityTypeKey,
        pos: [i32; 3],
        state: BlockStateId,
        tick: u64,
        fields: &mut mod_api::item::containers::ModBlockEntityFields,
    ) {
        Self::fire_block_entity_scene_tick(kind, pos, state, tick, fields);
    }
}
