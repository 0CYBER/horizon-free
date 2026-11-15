use super::*;
use mod_api::ModBlockEntity;
impl Game<'_> {
    pub(in crate::game) fn native_on_chunk_unloaded(
        &mut self,
        pos: [i32; 3],
        block_entity: BlockEntityRef,
        entity: &ModBlockEntity,
    ) {
        self.fire_on_chunk_unloaded(pos, block_entity, entity);
    }
}

impl Game<'_> {
    /// Chunk `chunk` of `dimension` left the world (NeoForge's
    /// `ChunkEvent.Unload`), with a level that closes for being empty too. Don't
    /// read its blocks. Its block entities are still there, but changes made in
    /// this hook aren't saved: as in vanilla, the chunk is saved before it unloads.
    /// Not told when the world closes, which `native_world_lifecycle` tells.
    /// Server-side.
    pub(in crate::game) fn native_chunk_unload(&mut self, dimension: &str, chunk: [i32; 2]) {
        self.fire_chunk_unload(dimension, chunk);
        if let Some(level) = self.level(dimension) {
            self.forget_composter_chunk(level.dimension(), chunk);
        }
    }
    /// Chunk `chunk` of this server level `dimension` left it, after its last save: the
    /// parts of its mod block entities are told, then the mods of the chunk.
    pub(in crate::game) fn native_chunk_and_block_entities_unload(
        &mut self,
        dimension: &str,
        chunk: [i32; 2],
    ) {
        if mod_api::__private::has_part_hook(mod_api::HookId::OnChunkUnloaded)
            && let Some(level) = self.level(dimension)
        {
            self.notify_chunk_block_entities_unloaded(level, (chunk[0], chunk[1]));
        }
        self.native_chunk_unload(dimension, chunk);
    }
    /// Tells the parts of the mod block entities in chunk `chunk` of `level` that it
    /// unloads, each once, while they are still in the world's block entities.
    pub(in crate::game) fn notify_chunk_block_entities_unloaded(
        &mut self,
        level: mod_api::LevelRef,
        chunk: (i32, i32),
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::OnChunkUnloaded) {
            return;
        }
        let block_entities = &self.world.block_entities;
        let unloading: Vec<([i32; 3], BlockEntityRef, ModBlockEntity)> = block_entities
            .mods_in(chunk)
            .filter_map(|&(x, y, z)| {
                let horizon_client::containers::BlockEntity::Mod(entity) =
                    block_entities.get(&(x, y, z))?
                else {
                    return None;
                };
                let generation = block_entities.generation(&(x, y, z))?;
                Some((
                    [x, y, z],
                    mod_api::__private::game::block_entity_ref(level, [x, y, z], generation),
                    entity.try_into_api().ok()?,
                ))
            })
            .collect();
        for (pos, block_entity, entity) in &unloading {
            self.native_on_chunk_unloaded(*pos, *block_entity, entity);
        }
    }
}
