use super::{
    Game,
    api_values::{BlockEntitySnapshotError, TryIntoApi, finish_block_entity_writeback},
};

impl Game<'_> {
    pub(in crate::game) fn api_block_entity_tick(
        &mut self,
        pos: [i32; 3],
        entity: &mut horizon_client::containers::ModBlockEntity,
        slots: usize,
    ) -> bool {
        // Normalize the outgoing snapshot, not the original entity: Unchanged
        // must restore even a legacy unnormalized payload byte-for-byte.
        let mut fields = mod_api::item::containers::ModBlockEntityFields {
            nbt: entity.nbt.clone(),
            items: if slots == 0 {
                Vec::new()
            } else {
                let Ok(items) = entity.items.try_into_api() else {
                    if entity.items.len() < slots {
                        entity.items.resize(slots, None);
                    }
                    return true;
                };
                items
            },
            data: entity.data.clone(),
        };
        if fields.items.len() < slots {
            fields.items.resize(slots, None);
        }
        let normalized_items = (slots == 0 && !entity.items.is_empty())
            .then(|| horizon_client::save::encode_items(&entity.items));
        if let Some(items) = &normalized_items {
            fields.nbt.insert("Items".into(), items.clone());
        }
        // The old take path performed these DerefMut writes before snapshotting.
        // Retain its Changed revision exactly, but defer those writes until chosen.
        let normalization_writes =
            u64::from(entity.items.len() < slots) + if normalized_items.is_some() { 2 } else { 0 };
        let mut api = mod_api::ModBlockEntity::from_snapshot(
            entity.id().to_owned(),
            fields,
            mod_api::item::containers::ModRevision::from_snapshot(
                entity.revision().identity(),
                entity.revision().changes() + normalization_writes,
            ),
        );
        let revision = api.revision();
        let mut reply = mod_api::BlockEntityTickReply::Changed;
        self.native_block_entity_tick(&mut reply, pos, &mut api);
        let changed = reply == mod_api::BlockEntityTickReply::Changed;
        if changed {
            // Preserve the existing Changed normalization before checked apply.
            if entity.items.len() < slots {
                entity.items.resize(slots, None);
            }
            if let Some(items) = normalized_items {
                entity.items.clear();
                entity.nbt.insert("Items".into(), items);
            }
        }
        if let Some(scope) = &mut self.block_entity_tick_items
            && scope.items.is_some()
        {
            scope.unchanged = !changed;
            if changed {
                scope.snapshot = Some(Box::new((api, revision)));
            }
            return changed;
        }
        if !changed {
            return false;
        }
        let _ = finish_block_entity_writeback(
            entity,
            &api,
            revision,
            self.items(),
            "block_entity_tick",
            if self.block_entity_tick_key.as_ref().is_none_or(|key| {
                !key.unbalanced && key.failure_generation == self.wasm_dispatch_failures.get()
            }) {
                Ok(())
            } else {
                Err(BlockEntitySnapshotError::InvalidRevision)
            },
        );
        true
    }
}
