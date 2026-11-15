//! Applies block entity synchronization before notifying client listeners.
use super::*;
use mod_api::BlockEntityDataSource;
impl Game<'_> {
    /// `handleBlockEntityData`: the block entity at `pos`, when it is of the
    /// packet's type, loads the tag (`loadWithComponents`). An empty compound
    /// leaves stored fields unchanged; update packets still send a notice.
    pub(in crate::game) fn remote_block_entity_data(
        &mut self,
        pos: [i32; 3],
        kind: i32,
        source: BlockEntityDataSource,
        tag: &Tag,
    ) {
        let key = (pos[0], pos[1], pos[2]);
        let Some(name) = self
            .remote
            .as_ref()
            .and_then(|r| r.builtin.get("minecraft:block_entity_type")?.name(kind))
        else {
            return;
        };
        // A decorated pot is no block entity here: its tag is kept with the chunk.
        if name == "minecraft:decorated_pot" {
            if let Tag::Compound(fields) = tag
                && !fields.is_empty()
            {
                let mut full = fields.clone();
                full.insert("id".into(), Tag::String(name.into()));
                for (k, v) in ["x", "y", "z"].into_iter().zip(pos) {
                    full.insert(k.into(), Tag::Int(v));
                }
                crate::game::spill::put_saved_entity(&mut self.world, pos, Tag::Compound(full));
                // The section is drawn again with the pot's sherds.
                let state = self.world.block(pos[0], pos[1], pos[2]);
                let _ = self.world.set(pos[0], pos[1], pos[2], state);
            }
            return;
        }
        let Tag::Compound(fields) = tag else {
            return;
        };
        // Only the declared type on one of its valid blocks accepts an update tag.
        // Nothing is drawn from it yet, so its section stays as it is.
        let block = self.world.block(pos[0], pos[1], pos[2]);
        if let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&key) {
            if m.id() == name
                && horizon_client::block_entity_types::valid(
                    &self.res.facts().get(block).name,
                    name,
                ) == Some(true)
            {
                m.merge_update_tag(fields);
                if source == BlockEntityDataSource::UpdatePacket || !fields.is_empty() {
                    self.notify_block_entity_data_received(pos, source, fields);
                }
            }
            return;
        }
        let Some(entity) = self.world.block_entities.get(&key) else {
            return;
        };
        if entity.id() != name || fields.is_empty() && !matches!(entity, BlockEntity::Conduit(_)) {
            return;
        }
        let mut full = fields.clone();
        full.insert("id".into(), Tag::String(name.into()));
        for (k, v) in ["x", "y", "z"].into_iter().zip(pos) {
            full.insert(k.into(), Tag::Int(v));
        }
        if let Some((_, mut loaded)) =
            horizon_client::save::decode_block_entity(&Tag::Compound(full))
        {
            keep_client_state(self.world.block_entities.get(&key), &mut loaded);
            let command_block = matches!(loaded, BlockEntity::CommandBlock(_));
            self.world.block_entities.insert(key, loaded);
            self.update_spawner_display(key);
            // The section is drawn again with what the block entity shows.
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let _ = self.world.set(pos[0], pos[1], pos[2], state);
            if command_block {
                self.command_block_data_arrived(key);
            }
        }
    }
}

impl Game<'_> {
    pub(in crate::game) fn remote_block_entity_packet(
        &mut self,
        p: horizon_protocol::packets::play::BlockEntityData,
    ) {
        self.remote_block_entity_data(p.pos, p.kind, BlockEntityDataSource::UpdatePacket, &p.tag);
    }
}
