use super::*;
use mod_api::ModBlockEntity;

impl Game<'_> {
    /// Tick the existing individual block entity moment. Unchosen replies use full write-back.
    pub(in crate::game) fn native_block_entity_tick(
        &mut self,
        reply: &mut mod_api::BlockEntityTickReply,
        pos: [i32; 3],
        entity: &mut ModBlockEntity,
    ) {
        self.fire_block_entity_tick(reply, pos, entity);
    }

    /// The tick of the mod block entity at `pos`.
    pub(in crate::game) fn tick_mod_block_entity_loaded(&mut self, pos: Pos) {
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&pos) else {
            return;
        };
        let mut taken = std::mem::take(&mut **m);
        let placeholder_identity = m.revision().identity();
        let state = self.world.block(pos.0, pos.1, pos.2).0;
        let slots = self
            .world
            .block_entities
            .mod_container(state)
            .map_or(0, |c| c.slots);
        let mut items = taken.items.clone();
        if items.len() < slots {
            items.resize(slots, None);
        }
        let parent = (
            self.block_entity_tick_key
                .replace(crate::game::block_entity_tick_items::Key {
                    pos,
                    placeholder_identity,
                    failure_generation: self.wasm_dispatch_failures.get(),
                    transaction_depth: self.transactions.len(),
                    unbalanced: false,
                }),
            self.block_entity_tick_items.take(),
        );
        let changed = self.api_block_entity_tick([pos.0, pos.1, pos.2], &mut taken, slots);
        self.finish_tick_item_writeback(pos, taken, &items, parent, changed);
    }

    /// Restore only this tick's original placeholder, with no tick-only effects.
    pub(in crate::game) fn restore_unchanged_tick(&mut self, pos: Pos, taken: EngineBlockEntity) {
        if let Some(BlockEntity::Mod(entity)) = self.world.block_entities.get_mut(&pos)
            && self.block_entity_tick_key.is_some_and(|key| {
                key.pos == pos
                    && entity.id().is_empty()
                    && entity.revision().identity() == key.placeholder_identity
            })
        {
            **entity = taken;
        } else {
            self.drop_unreturned_mod_contents(pos, taken);
        }
    }
}
