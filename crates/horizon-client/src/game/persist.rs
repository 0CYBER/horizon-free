//! Saving and loading the game state that lives outside chunk blocks:
//! entities (`entities/*.mca`, per chunk like vanilla's `EntityStorage`) and
//! scheduled block/fluid ticks (chunk `block_ticks`/`fluid_ticks`).
use super::{FallingBlock, Game};
use horizon_client::{
    Result,
    chunk::Chunk,
    entity_nbt,
    save::{ChunkExtras, random_uuid},
};
use horizon_core::{
    geometry::Vec3,
    position::ChunkPos,
    random::RandomSource,
    ticks::ChunkTicks,
};
use horizon_world::{chunk::BlockState, nbt::Tag, registry::BlockStateId};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// What Horizon remembers about loaded entities to save them faithfully.
#[derive(Default)]
pub struct EntityMemory {
    /// UUIDs of mobs and item entities by Horizon entity ID.
    uuids: HashMap<u32, [i32; 4]>,
    /// The compound each mob or item entity was loaded with.
    raw: HashMap<u32, Tag>,
    /// Entities of types Horizon does not simulate, kept per chunk and
    /// written back unchanged.
    unknown: HashMap<(i32, i32), Vec<Tag>>,
}

fn chunk_of(p: Vec3) -> (i32, i32) {
    ((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4)
}

impl Game<'_> {
    /// Saved ticks and the save time for a chunk about to be written.
    pub(super) fn attach_save_data(&self, chunk: &mut Chunk) {
        let pos = ChunkPos::new(chunk.x, chunk.z);
        let time = self.world.game_time;
        let mut extras: ChunkExtras = chunk.saved.as_deref().cloned().unwrap_or_default();
        if let Some(t) = self.block_ticks.blocks.chunk(pos) {
            extras.block_ticks = t.pack(time);
        } else {
            extras.block_ticks.clear();
        }
        if let Some(t) = self.block_ticks.fluids.chunk(pos) {
            extras.fluid_ticks = t.pack(time);
        } else {
            extras.fluid_ticks.clear();
        }
        extras.last_update = time;
        chunk.saved = Some(Arc::new(extras));
    }
    /// Register a newly resident chunk with the tick scheduler, with the
    /// block and fluid ticks it was saved with.
    pub(super) fn register_chunk_ticks(&mut self, key: (i32, i32)) {
        let pos = ChunkPos::new(key.0, key.1);
        let saved = self.world.chunks.get(&key).and_then(|c| c.saved.clone());
        let (blocks, fluids) = saved.as_deref().map_or((Vec::new(), Vec::new()), |s| (s.block_ticks.clone(), s.fluid_ticks.clone()));
        let loaded = !blocks.is_empty() || !fluids.is_empty();
        if self.block_ticks.add_chunk(pos, ChunkTicks::from_saved(blocks), ChunkTicks::from_saved(fluids)).is_ok() && loaded {
            let time = self.world.game_time;
            self.block_ticks.blocks.unpack_chunk(pos, time);
            self.block_ticks.fluids.unpack_chunk(pos, time);
        }
        // The saved list is consumed: later saves take the scheduler's ticks.
        if loaded && let Some(c) = self.world.chunks.get_mut(&key) {
            let c = Arc::make_mut(c);
            if let Some(s) = c.saved.as_mut() {
                let s = Arc::make_mut(s);
                s.block_ticks.clear();
                s.fluid_ticks.clear();
            }
        }
    }
    /// Spawn the entities of chunks that finished loading.
    pub(super) fn spawn_loaded_entities(&mut self) {
        let loaded = std::mem::take(&mut self.world.loaded_entities);
        let player_uuid = self.level.player_uuid;
        for (key, tags) in loaded {
            for tag in tags {
                let kind = entity_nbt::kind(&tag).unwrap_or_default();
                let uuid = entity_nbt::uuid(&tag);
                match kind.as_str() {
                    "minecraft:item" => {
                        self.next_entity += 1;
                        let bob = self.world.random.next_float() * std::f32::consts::PI * 2.0;
                        if let Some(e) = entity_nbt::item_entity_from_nbt(&tag, self.next_entity, bob) {
                            self.remember(e.id, uuid, tag);
                            self.dropped.push(e);
                        }
                    }
                    "minecraft:experience_orb" => self.orbs.extend(entity_nbt::orbs_from_nbt(&tag)),
                    "minecraft:arrow" => {
                        if let Some(a) = entity_nbt::arrow_from_nbt(&tag, player_uuid) {
                            self.arrows.push(a);
                        }
                    }
                    "minecraft:falling_block" => {
                        if let Some((state, position, velocity, age)) = entity_nbt::falling_block_from_nbt(&tag)
                            && let Some(state) = self.state_from_nbt(&state)
                        {
                            self.falling.push(FallingBlock { state, position, previous: position, velocity, age });
                        }
                    }
                    _ if self.res.mob_types.contains_key(&kind) => {
                        self.next_entity += 1;
                        if let Some(m) = entity_nbt::mob_from_nbt(&tag, &self.res.mob_types, self.next_entity) {
                            self.remember(m.id, uuid, tag);
                            self.mobs.push(m);
                        }
                    }
                    _ => self.persist.unknown.entry(key).or_default().push(tag),
                }
            }
        }
    }
    fn remember(&mut self, id: u32, uuid: Option<[i32; 4]>, tag: Tag) {
        if let Some(u) = uuid {
            self.persist.uuids.insert(id, u);
        }
        self.persist.raw.insert(id, tag);
    }
    fn uuid_for(&mut self, id: u32) -> [i32; 4] {
        *self.persist.uuids.entry(id).or_insert_with(random_uuid)
    }
    /// `BlockState.CODEC`: a block name or `{id, properties}` (or legacy `{Name, Properties}`).
    fn state_from_nbt(&self, tag: &Tag) -> Option<BlockStateId> {
        let registry = self.res.assets.registry;
        let state = match tag.get("Name") {
            Some(Tag::String(name)) => BlockState {
                name: name.to_string_lossy(),
                properties: match tag.get("Properties") {
                    Some(Tag::Compound(p)) => p
                        .iter()
                        .filter_map(|(k, v)| match v {
                            Tag::String(v) => Some((k.to_string_lossy(), v.to_string_lossy())),
                            _ => None,
                        })
                        .collect(),
                    _ => Default::default(),
                },
            },
            _ => BlockState::from_nbt(tag).ok()?,
        };
        registry.id(&state).or_else(|| registry.default_state(&state.name))
    }
    fn state_to_nbt(&self, state: BlockStateId) -> Tag {
        let registry = self.res.assets.registry;
        match registry.state(state) {
            Some(s) if registry.default_state(&s.name) == Some(state) => Tag::String(s.name.as_str().into()),
            Some(s) => s.to_nbt(),
            None => Tag::String("minecraft:air".into()),
        }
    }
    /// The entity NBT of every simulated entity in `keys`, plus the unknown
    /// entities kept for them. With `remove`, the entities leave the world
    /// (their chunks unload).
    fn entity_chunks(&mut self, keys: &HashSet<(i32, i32)>, remove: bool) -> Vec<((i32, i32), Vec<Tag>)> {
        let mut out: HashMap<(i32, i32), Vec<Tag>> = keys.iter().map(|k| (*k, Vec::new())).collect();
        let player_uuid = self.level.player_uuid;
        let mobs: Vec<usize> = (0..self.mobs.len()).filter(|&i| keys.contains(&chunk_of(self.mobs[i].position)) && self.mobs[i].alive()).collect();
        for i in mobs {
            let id = self.mobs[i].id;
            let uuid = self.uuid_for(id);
            let tag = entity_nbt::mob_to_nbt(&self.mobs[i], uuid, self.persist.raw.get(&id));
            out.entry(chunk_of(self.mobs[i].position)).or_default().push(tag);
        }
        let items: Vec<usize> = (0..self.dropped.len()).filter(|&i| keys.contains(&chunk_of(self.dropped[i].position))).collect();
        for i in items {
            let id = self.dropped[i].id;
            let uuid = self.uuid_for(id);
            let tag = entity_nbt::item_entity_to_nbt(&self.dropped[i], uuid, self.persist.raw.get(&id));
            out.entry(chunk_of(self.dropped[i].position)).or_default().push(tag);
        }
        for o in &self.orbs {
            if keys.contains(&chunk_of(o.position)) {
                out.entry(chunk_of(o.position)).or_default().push(entity_nbt::orb_to_nbt(o, random_uuid()));
            }
        }
        for a in &self.arrows {
            if keys.contains(&chunk_of(a.position)) {
                let owner = if a.from_player { player_uuid } else { None };
                out.entry(chunk_of(a.position)).or_default().push(entity_nbt::arrow_to_nbt(a, random_uuid(), owner));
            }
        }
        for f in &self.falling {
            if keys.contains(&chunk_of(f.position)) {
                let tag = entity_nbt::falling_block_to_nbt(self.state_to_nbt(f.state), f.position, f.velocity, f.age, random_uuid());
                out.entry(chunk_of(f.position)).or_default().push(tag);
            }
        }
        for key in keys {
            let unknown = if remove { self.persist.unknown.remove(key) } else { self.persist.unknown.get(key).cloned() };
            if let Some(list) = unknown {
                out.entry(*key).or_default().extend(list);
            }
        }
        if remove {
            let gone = |p: Vec3| keys.contains(&chunk_of(p));
            let mut removed: Vec<u32> = Vec::new();
            self.mobs.retain(|m| {
                let keep = !gone(m.position);
                if !keep {
                    removed.push(m.id);
                }
                keep
            });
            self.dropped.retain(|e| {
                let keep = !gone(e.position);
                if !keep {
                    removed.push(e.id);
                }
                keep
            });
            self.orbs.retain(|o| !gone(o.position));
            self.arrows.retain(|a| !gone(a.position));
            self.falling.retain(|f| !gone(f.position));
            for id in removed {
                self.persist.uuids.remove(&id);
                self.persist.raw.remove(&id);
            }
        }
        out.into_iter().collect()
    }
    /// Write the entities of every resident chunk (an autosave or quit).
    pub(super) fn save_entities(&mut self) -> Result<()> {
        self.spawn_loaded_entities();
        let keys: HashSet<(i32, i32)> = self.world.chunks.keys().copied().collect();
        let chunks = self.entity_chunks(&keys, false);
        // Forget entities that no longer exist.
        let alive: HashSet<u32> = self.mobs.iter().map(|m| m.id).chain(self.dropped.iter().map(|e| e.id)).collect();
        self.persist.uuids.retain(|id, _| alive.contains(id));
        self.persist.raw.retain(|id, _| alive.contains(id));
        self.save.save_entities(chunks)
    }
    /// Save and remove the entities of chunks that unloaded.
    pub(super) fn unload_entities(&mut self, unloaded: &[Arc<Chunk>]) {
        if unloaded.is_empty() {
            return;
        }
        self.spawn_loaded_entities();
        let keys: HashSet<(i32, i32)> = unloaded.iter().map(|c| (c.x, c.z)).collect();
        let chunks = self.entity_chunks(&keys, true);
        if let Err(e) = self.save.save_entities(chunks) {
            eprintln!("Horizon: failed to save entities: {e}");
        }
    }
    /// Leaving a dimension: its entities are saved with it.
    pub(super) fn save_dimension_entities(&mut self) -> Result<()> {
        self.spawn_loaded_entities();
        let keys: HashSet<(i32, i32)> = self.world.chunks.keys().copied().collect();
        let chunks = self.entity_chunks(&keys, true);
        self.persist = EntityMemory::default();
        self.save.save_entities(chunks)
    }
}
