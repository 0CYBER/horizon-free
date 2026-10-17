//! Encoding resident chunks for the wire: ClientboundLevelChunkWithLightPacket
//! (sections with their paletted containers, block entities and the sky and
//! block DataLayers) and the section block-change batches.
use crate::{
    chunk::{Chunk, Light, MIN_Y, SECTIONS, Section},
    containers::BlockEntity,
    fast_hash::FxHashMap,
};
use horizon_protocol::{
    buf::set_bit,
    chunk::{BIOMES, BLOCKS, BlockEntityInfo, ChunkData, LightData, Paletted, Strategy},
    packets::play::LevelChunkWithLight,
    registry::{Builtin, ceil_log2},
};
use horizon_world::nbt::Tag;
use std::{collections::HashMap, sync::Arc};

const NON_AIR: u8 = 1;
const FLUID: u8 = 2;
const ENTITY: u8 = 4;

/// The vanilla block entity type of a block (BlockEntityType.validBlocks).
pub fn block_entity_type(name: &str) -> Option<&'static str> {
    let n = name.strip_prefix("minecraft:")?;
    Some(match n {
        "chest" => "minecraft:chest",
        "trapped_chest" => "minecraft:trapped_chest",
        "ender_chest" => "minecraft:ender_chest",
        "barrel" => "minecraft:barrel",
        "furnace" => "minecraft:furnace",
        "blast_furnace" => "minecraft:blast_furnace",
        "smoker" => "minecraft:smoker",
        "hopper" => "minecraft:hopper",
        "dispenser" => "minecraft:dispenser",
        "dropper" => "minecraft:dropper",
        "brewing_stand" => "minecraft:brewing_stand",
        "beacon" => "minecraft:beacon",
        "bell" => "minecraft:bell",
        "campfire" | "soul_campfire" => "minecraft:campfire",
        "enchanting_table" => "minecraft:enchanting_table",
        "jukebox" => "minecraft:jukebox",
        "lectern" => "minecraft:lectern",
        "spawner" => "minecraft:mob_spawner",
        "comparator" => "minecraft:comparator",
        "daylight_detector" => "minecraft:daylight_detector",
        "conduit" => "minecraft:conduit",
        "decorated_pot" => "minecraft:decorated_pot",
        "moving_piston" => "minecraft:piston",
        "structure_block" => "minecraft:structure_block",
        "jigsaw" => "minecraft:jigsaw",
        "command_block" | "chain_command_block" | "repeating_command_block" => {
            "minecraft:command_block"
        }
        "end_portal" => "minecraft:end_portal",
        "end_gateway" => "minecraft:end_gateway",
        "sculk_sensor" => "minecraft:sculk_sensor",
        "calibrated_sculk_sensor" => "minecraft:calibrated_sculk_sensor",
        "sculk_catalyst" => "minecraft:sculk_catalyst",
        "sculk_shrieker" => "minecraft:sculk_shrieker",
        "crafter" => "minecraft:crafter",
        "chiseled_bookshelf" => "minecraft:chiseled_bookshelf",
        "trial_spawner" => "minecraft:trial_spawner",
        "vault" => "minecraft:vault",
        "beehive" | "bee_nest" => "minecraft:beehive",
        "suspicious_sand" | "suspicious_gravel" => "minecraft:brushable_block",
        "creaking_heart" => "minecraft:creaking_heart",
        "test_block" => "minecraft:test_block",
        "test_instance_block" => "minecraft:test_instance_block",
        n if n.ends_with("shulker_box") => "minecraft:shulker_box",
        n if n.ends_with("_wall_hanging_sign") || n.ends_with("_hanging_sign") => {
            "minecraft:hanging_sign"
        }
        n if n.ends_with("_sign") => "minecraft:sign",
        n if n.ends_with("_banner") => "minecraft:banner",
        n if n.ends_with("_head") || n.ends_with("_skull") => "minecraft:skull",
        n if n.ends_with("_shelf") => "minecraft:shelf",
        n if n.ends_with("copper_golem_statue") => "minecraft:copper_golem_statue",
        _ => return None,
    })
}

/// Per-state facts and registry ids for encoding chunks.
pub struct ChunkEncoder {
    flags: Vec<u8>,
    /// Block entity type id per state (-1 for none).
    entity_types: Vec<i16>,
    /// Network biome id per internal biome index.
    pub biome_ids: Vec<i32>,
    biome_bits: u8,
    /// The block registry id of each state's block, for BlockEvent.
    pub block_ids: Vec<i32>,
    /// The save's tables, for the heightmaps.
    pub tables: Option<Arc<crate::save::SaveTables>>,
    /// Sections already looked at, by address (see `Encoded`).
    cache: std::sync::Mutex<FxHashMap<usize, Encoded>>,
    /// Each column's heightmaps, with the sections they were made from.
    heights: std::sync::Mutex<HashMap<(i32, i32), Heights>>,
    /// The level's sections among a column's: all of them in the Overworld,
    /// y 0 to 255 in the Nether and the End (`DimensionType` min_y and height).
    sections: std::ops::Range<usize>,
    /// Whether the level has sky light (`DimensionType.hasSkyLight`).
    sky: bool,
}

/// A column's client heightmaps, valid while it holds the same sections.
struct Heights {
    sections: Vec<std::sync::Weak<Section>>,
    maps: Vec<(i32, Vec<i64>)>,
}

/// What a resident section gives the wire, kept while the section is
/// unchanged. Resident sections are immutable behind `Arc` (edits go through
/// `Arc::make_mut`, which moves an edited section to a new address while a
/// `Weak` points at the old one), and the `Weak` keeps the old address from
/// being reused: the same address is the same, unchanged section.
struct Encoded {
    section: std::sync::Weak<Section>,
    non_air: bool,
    /// LevelChunkSection.write, and its block entity blocks.
    wire: Option<(Arc<[u8]>, Entities)>,
    /// The section's light as the wire's two DataLayers, when not uniform.
    light: Option<Arc<Layers>>,
}

/// A section's blocks that have a block entity type, as (index, state) in
/// index order (y, then z, then x).
type Entities = Arc<[(u16, u16)]>;

/// One section's light as DataLayers (two blocks per byte, the even one in
/// the low nibble), and whether each holds any light.
struct Layers {
    sky: Vec<u8>,
    block: Vec<u8>,
    sky_any: bool,
    block_any: bool,
}
impl Layers {
    fn of(light: &Light) -> Self {
        let (mut sky, mut block) = (vec![0u8; 2048], vec![0u8; 2048]);
        let (sky_any, block_any) = match light {
            Light::Uniform(v) => {
                let (sk, bl) = (v >> 4, v & 15);
                sky.fill(sk | sk << 4);
                block.fill(bl | bl << 4);
                (sk != 0, bl != 0)
            }
            Light::Full(a) => {
                for ((pair, sky), block) in
                    a.as_chunks::<2>().0.iter().zip(&mut sky).zip(&mut block)
                {
                    let [even, odd] = *pair;
                    *sky = even >> 4 | (odd & 0xF0);
                    *block = (even & 15) | odd << 4;
                }
                let all = a.iter().fold(0u8, |acc, &v| acc | v);
                (all & 0xF0 != 0, all & 15 != 0)
            }
        };
        Self {
            sky,
            block,
            sky_any,
            block_any,
        }
    }
}

/// Sections kept before the ones no longer resident are dropped.
const CACHE_SWEEP: usize = 1 << 15;
impl ChunkEncoder {
    /// `biome_names`: the internal biome list in order; `biome_registry`:
    /// the synchronized biome entries in network order.
    pub fn new(
        states: impl IntoIterator<Item = (Arc<str>, bool)>,
        builtin: &Builtin,
        biome_names: &[String],
        biome_registry: &[String],
    ) -> Self {
        let types = builtin.get("minecraft:block_entity_type");
        let blocks = builtin.get("minecraft:block");
        let mut flags = Vec::new();
        let mut entity_types = Vec::new();
        let mut block_ids = Vec::new();
        for (name, has_fluid) in states {
            let air = matches!(
                &*name,
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            );
            let mut f = 0u8;
            if !air {
                f |= NON_AIR;
                if has_fluid {
                    f |= FLUID;
                }
            }
            let t = block_entity_type(&name)
                .and_then(|t| types.and_then(|ids| ids.id(t)))
                .map_or(-1, |v| v as i16);
            if t >= 0 {
                f |= ENTITY;
            }
            flags.push(f);
            entity_types.push(t);
            block_ids.push(blocks.and_then(|b| b.id(&name)).unwrap_or(0));
        }
        let biome_ids = biome_names
            .iter()
            .map(|n| {
                biome_registry
                    .iter()
                    .position(|b| b == n)
                    .map_or(0, |i| i as i32)
            })
            .collect();
        Self {
            flags,
            entity_types,
            biome_ids,
            biome_bits: ceil_log2(biome_registry.len()),
            block_ids,
            tables: None,
            cache: Default::default(),
            heights: Default::default(),
            sections: 0..SECTIONS,
            sky: true,
        }
    }

    /// The client heightmaps (Heightmap.prime over the column), from the
    /// cache while the column's sections are unchanged.
    fn heightmaps(&self, chunk: &Chunk, tables: &crate::save::SaveTables) -> Vec<(i32, Vec<i64>)> {
        let same = |h: &Heights| {
            h.sections.len() == chunk.sections.len()
                && h.sections
                    .iter()
                    .zip(&chunk.sections)
                    .all(|(w, s)| std::ptr::eq(w.as_ptr(), Arc::as_ptr(s)))
        };
        let mut heights = self.heights.lock().unwrap();
        if let Some(h) = heights.get(&(chunk.x, chunk.z))
            && same(h)
        {
            return h.maps.clone();
        }
        if heights.len() >= CACHE_SWEEP / 4 {
            heights.retain(|_, h| h.sections.iter().all(|w| w.strong_count() > 0));
            if heights.len() >= CACHE_SWEEP / 8 {
                heights.clear();
            }
        }
        let min_y = MIN_Y + 16 * self.sections.start as i32;
        let height = 16 * self.sections.len() as i32;
        let maps = crate::save::client_heightmaps_in(chunk, tables, min_y, height);
        heights.insert(
            (chunk.x, chunk.z),
            Heights {
                sections: chunk.sections.iter().map(Arc::downgrade).collect(),
                maps: maps.clone(),
            },
        );
        maps
    }

    /// The cached entry for a section, made with `non_air` when missing.
    fn cached<T>(&self, section: &Arc<Section>, read: impl FnOnce(&mut Encoded) -> T) -> T {
        read(self.entry(&mut self.cache.lock().unwrap(), section))
    }

    /// A section's entry in the locked cache, made with `non_air` when missing.
    fn entry<'a>(
        &self,
        cache: &'a mut FxHashMap<usize, Encoded>,
        section: &Arc<Section>,
    ) -> &'a mut Encoded {
        if cache.len() >= CACHE_SWEEP {
            cache.retain(|_, e| e.section.strong_count() > 0);
            if cache.len() >= CACHE_SWEEP / 2 {
                cache.clear();
            }
        }
        cache
            .entry(Arc::as_ptr(section) as usize)
            .or_insert_with(|| Encoded {
                section: Arc::downgrade(section),
                non_air: self.non_air(section),
                wire: None,
                light: None,
            })
    }

    /// LevelChunkSection.hasOnlyAir, negated.
    fn non_air(&self, section: &Section) -> bool {
        let non_air = |v: u32| self.flags.get(v as usize).copied().unwrap_or(0) & NON_AIR != 0;
        // The palette first: most sections are all air, or one block.
        if !section.blocks.maybe_has(non_air) {
            false
        } else if let Some(v) = section.blocks.uniform() {
            non_air(v)
        } else {
            // Stops at the first block, usually near the bottom.
            (0..BLOCKS).any(|i| non_air(section.blocks.get(i)))
        }
    }

    /// A section on the wire, from the cache or encoded now.
    fn section_wire(&self, section: &Arc<Section>) -> (Arc<[u8]>, Entities) {
        if let Some(wire) = self.cached(section, |e| e.wire.clone()) {
            return wire;
        }
        let (encoded, entities) = self.section(section);
        let mut w = horizon_protocol::buf::Writer::new();
        encoded.write(&mut w);
        let wire: (Arc<[u8]>, Entities) = (w.data.into(), entities.into());
        self.cached(section, |e| e.wire = Some(wire.clone()));
        wire
    }
    /// Encode for a level of `min_y` and `height` (multiples of 16 within
    /// the column), with or without sky light.
    pub fn set_dimension(&mut self, min_y: i32, height: i32, sky: bool) {
        let lo = usize::try_from((min_y - MIN_Y) >> 4)
            .unwrap_or(0)
            .min(SECTIONS);
        let n = usize::try_from(height >> 4).unwrap_or(SECTIONS);
        self.sections = lo..(lo + n).min(SECTIONS);
        self.sky = sky;
        // The heightmaps depend on the level's height.
        self.heights.get_mut().unwrap().clear();
    }
    pub fn entity_type(&self, state: u16) -> Option<i32> {
        self.entity_types
            .get(usize::from(state))
            .filter(|t| **t >= 0)
            .map(|t| i32::from(*t))
    }
    pub fn is_air(&self, state: u16) -> bool {
        self.flags
            .get(usize::from(state))
            .is_none_or(|f| f & NON_AIR == 0)
    }

    fn section(&self, section: &Section) -> (horizon_protocol::chunk::Section, Vec<(u16, u16)>) {
        let mut values = Vec::with_capacity(BLOCKS);
        let (mut non_empty, mut fluids, mut entities) = (0i16, 0i16, Vec::new());
        let mut count = |state: u16, n: i16| {
            let f = self.flags.get(usize::from(state)).copied().unwrap_or(0);
            if f & NON_AIR != 0 {
                non_empty += n;
                if f & FLUID != 0 {
                    fluids += n;
                }
            }
            f & ENTITY != 0
        };
        // One block everywhere (all air, mostly): what from_values would give.
        let uniform = section.blocks.uniform();
        if let Some(v) = uniform {
            if count(v as u16, BLOCKS as i16) {
                entities = (0..BLOCKS as u16).map(|i| (i, v as u16)).collect();
            }
        } else {
            let mut unpacked = [0u16; BLOCKS];
            section.unpack(&mut unpacked);
            for (i, &v) in unpacked.iter().enumerate() {
                if count(v, 1) {
                    entities.push((i as u16, v));
                }
                values.push(i32::from(v));
            }
        }
        let biome_values: Vec<i32> = (0..BIOMES)
            .map(|j| {
                let internal = section.biomes.get(j) as u16;
                self.biome_ids
                    .get(usize::from(internal))
                    .copied()
                    .unwrap_or(0)
            })
            .collect();
        (
            horizon_protocol::chunk::Section {
                non_empty,
                fluids,
                states: match uniform {
                    Some(v) => Paletted {
                        bits: 0,
                        palette: vec![v as i32],
                        data: Vec::new(),
                    },
                    None => Paletted::from_values(&values, Strategy::Blocks, 16),
                },
                biomes: Paletted::from_values(&biome_values, Strategy::Biomes, self.biome_bits),
            },
            entities,
        )
    }

    /// The chunk column with its light and block entities.
    /// The non-air sections of a column as bits (LevelChunkSection.hasOnlyAir).
    fn occupied(&self, chunk: &Chunk) -> u64 {
        let mut bits = 0u64;
        // One lock for the column: encode_in asks for nine columns' sections.
        let mut cache = self.cache.lock().unwrap();
        for (i, s) in chunk.sections[self.sections.clone()].iter().enumerate() {
            if self.entry(&mut cache, s).non_air {
                bits |= 1 << i;
            }
        }
        bits
    }

    /// The chunk column with its light and block entities. `world` gives the
    /// neighbouring columns, whose sections decide which light layers exist.
    pub fn encode_in(&self, chunk: &Chunk, world: &crate::world::World) -> LevelChunkWithLight {
        let mut around = 0u64;
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(c) = world.chunks.get(&(chunk.x + dx, chunk.z + dz)) {
                    around |= self.occupied(c);
                }
            }
        }
        let mut packet = self.encode_blocks(chunk, &world.block_entities);
        packet.light = self.light_data(chunk, around);
        packet
    }

    pub fn encode(
        &self,
        chunk: &Chunk,
        block_entities: &HashMap<(i32, i32, i32), BlockEntity>,
    ) -> LevelChunkWithLight {
        let mut packet = self.encode_blocks(chunk, block_entities);
        packet.light = self.light_data(chunk, u64::MAX);
        packet
    }

    /// The packet without its light.
    fn encode_blocks(
        &self,
        chunk: &Chunk,
        block_entities: &HashMap<(i32, i32, i32), BlockEntity>,
    ) -> LevelChunkWithLight {
        let mut buffer = Vec::new();
        let mut entity_infos = Vec::new();
        for (i, s) in chunk
            .sections
            .iter()
            .enumerate()
            .skip(self.sections.start)
            .take(self.sections.len())
        {
            let (encoded, entities) = self.section_wire(s);
            // The section's block entity blocks, kept with its bytes, in the
            // order a scan of every block would find them.
            for &(index, state) in entities.iter() {
                let Some(kind) = self.entity_type(state) else {
                    continue;
                };
                let (x, y, z) = (
                    i32::from(index & 15),
                    i32::from(index >> 8),
                    i32::from(index >> 4 & 15),
                );
                let wy = MIN_Y + (i as i32) * 16 + y;
                let pos = (chunk.x * 16 + x, wy, chunk.z * 16 + z);
                entity_infos.push(BlockEntityInfo {
                    packed_xz: ((x << 4) | z) as i8,
                    y: wy as i16,
                    kind,
                    // BlockEntityInfo.create: an empty update tag is not sent.
                    tag: block_entities
                        .get(&pos)
                        .and_then(entity_tag)
                        .unwrap_or(Tag::End),
                });
            }
            buffer.extend_from_slice(&encoded);
        }
        // A loaded chunk's block entities enter its maps in the save's order
        // (ChunkAccess.pendingBlockEntities, then LevelChunk.blockEntities):
        // keys that share a bucket keep that order. Others follow in scan order.
        if let Some(order) = chunk.saved.as_ref().map(|s| &s.block_entity_order)
            && !order.is_empty()
        {
            let at = |e: &BlockEntityInfo| {
                let pos = (
                    chunk.x * 16 + i32::from(e.packed_xz as u8 >> 4),
                    i32::from(e.y),
                    chunk.z * 16 + i32::from(e.packed_xz as u8 & 15),
                );
                order.iter().position(|p| *p == pos).unwrap_or(usize::MAX)
            };
            entity_infos.sort_by_key(at);
        }
        java_hash_order(&mut entity_infos, |e| {
            (
                chunk.x * 16 + i32::from(e.packed_xz as u8 >> 4),
                i32::from(e.y),
                chunk.z * 16 + i32::from(e.packed_xz as u8 & 15),
            )
        });
        LevelChunkWithLight {
            x: chunk.x,
            z: chunk.z,
            chunk: ChunkData {
                heightmaps: self
                    .tables
                    .as_ref()
                    .map(|t| self.heightmaps(chunk, t))
                    .unwrap_or_default(),
                buffer,
                block_entities: entity_infos,
            },
            light: LightData {
                sky_mask: Vec::new(),
                block_mask: Vec::new(),
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky: Vec::new(),
                block: Vec::new(),
            },
        }
    }

    /// ClientboundLightUpdatePacketData: the light layers of the sections
    /// that exist around the column (`around`: the non-air sections of it
    /// and its neighbours, as bits). A resident section's layers come from
    /// the cache while it is unchanged (uniform light is quicker to fill than
    /// to keep).
    fn light_data(&self, chunk: &Chunk, around: u64) -> LightData {
        let mut data = light_data_with(&chunk.sections[self.sections.clone()], around, |s| {
            if matches!(s.light, Light::Uniform(_)) {
                return Arc::new(Layers::of(&s.light));
            }
            self.cached(s, |e| e.light.clone()).unwrap_or_else(|| {
                let layers = Arc::new(Layers::of(&s.light));
                self.cached(s, |e| e.light = Some(layers.clone()));
                layers
            })
        });
        if !self.sky {
            // No sky light storage at all.
            data.sky_mask.clear();
            data.empty_sky_mask.clear();
            data.sky.clear();
        }
        data
    }
}

/// The order LevelChunk.getBlockEntities() iterates in: a fastutil
/// Object2ObjectOpenHashMap keyed by BlockPos (linear probing from
/// HashCommon.mix(hashCode), grown at 3/4 by reinserting from the top of
/// the table, iterated from the top down), filled in the order of the
/// java.util.HashMap of pending block entities that the loaded chunk's list
/// (`items` in section order) went through.
fn java_hash_order<T>(items: &mut Vec<T>, pos: impl Fn(&T) -> (i32, i32, i32)) {
    // Vec3i.hashCode.
    let hash = |&(x, y, z): &(i32, i32, i32)| {
        y.wrapping_add(z.wrapping_mul(31))
            .wrapping_mul(31)
            .wrapping_add(x)
    };
    let keys: Vec<i32> = items.iter().map(|i| hash(&pos(i))).collect();
    // java.util.HashMap: buckets in index order, insertion order within one.
    let mut capacity = 16usize;
    while keys.len() * 4 > capacity * 3 {
        capacity *= 2;
    }
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by_key(|&i| {
        ((keys[i] ^ ((keys[i] as u32) >> 16) as i32) as u32 as usize) & (capacity - 1)
    });
    // Object2ObjectOpenHashMap.
    let mix = |h: i32| {
        let h = h.wrapping_mul(0x9E37_79B9_u32 as i32) as u32;
        (h ^ (h >> 16)) as usize
    };
    let put = |table: &mut Vec<Option<usize>>, i: usize| {
        let mask = table.len() - 1;
        let mut at = mix(keys[i]) & mask;
        while table[at].is_some() {
            at = (at + 1) & mask;
        }
        table[at] = Some(i);
    };
    // Default expected size 16: HashCommon.arraySize(16, 0.75) = 32 slots.
    let mut table: Vec<Option<usize>> = vec![None; 32];
    for (size, &i) in order.iter().enumerate() {
        put(&mut table, i);
        // insert: `if (size++ >= maxFill) rehash(arraySize(size + 1, f))`.
        let max_fill = (table.len() * 3).div_ceil(4).min(table.len() - 1);
        if size >= max_fill {
            let grown = ((size + 2) * 4).div_ceil(3).next_power_of_two().max(2);
            let old = std::mem::replace(&mut table, vec![None; grown]);
            for &j in old.iter().rev().flatten() {
                put(&mut table, j);
            }
        }
    }
    let iteration: Vec<usize> = table.iter().rev().flatten().copied().collect();
    let mut slots: Vec<Option<T>> = items.drain(..).map(Some).collect();
    items.extend(iteration.into_iter().filter_map(|i| slots[i].take()));
}

/// The update tag of a block entity (getUpdateTag) for what the client shows.
pub fn entity_tag(entity: &BlockEntity) -> Option<Tag> {
    match entity {
        BlockEntity::Sign(sign) => Some(Tag::compound(sign.encode())),
        BlockEntity::Spawner(s) => Some(s.update_tag()),
        BlockEntity::Jukebox(_) | BlockEntity::Lectern(_) => None,
        _ => None,
    }
}

/// Light sections run from one below the world to one above it. A layer
/// exists where vanilla's light storage keeps one: within one section of a
/// non-air section of this column or a neighbouring one (`around`, bit i for
/// section i; LayerLightSectionStorage.updateSectionStatus). An existing
/// layer is sent, or marked empty when all dark; the others are left out.
#[cfg(test)]
fn light_data(sections: &[Arc<Section>], around: u64) -> LightData {
    light_data_with(sections, around, |s| Arc::new(Layers::of(&s.light)))
}

/// The light layers, `section` giving a resident section's.
fn light_data_with(
    sections: &[Arc<Section>],
    around: u64,
    section: impl Fn(&Arc<Section>) -> Arc<Layers>,
) -> LightData {
    let n = sections.len();
    let mut data = LightData {
        sky_mask: Vec::new(),
        block_mask: Vec::new(),
        empty_sky_mask: Vec::new(),
        empty_block_mask: Vec::new(),
        sky: Vec::new(),
        block: Vec::new(),
    };
    // Light section b holds chunk section b - 1.
    let exists =
        |b: usize| (b.saturating_sub(2)..=b.min(n - 1)).any(|s| s + 2 >= b && around >> s & 1 != 0);
    for b in 0..n + 2 {
        if !exists(b) {
            continue;
        }
        let layers = match b {
            // Below the world nothing is lit; above it the sky is full.
            0 => Arc::new(Layers::of(&Light::Uniform(0))),
            _ if b == n + 1 => Arc::new(Layers::of(&Light::Uniform(0xF0))),
            _ => section(&sections[b - 1]),
        };
        if layers.sky_any {
            set_bit(&mut data.sky_mask, b);
            data.sky.push(layers.sky.clone());
        } else {
            set_bit(&mut data.empty_sky_mask, b);
        }
        if layers.block_any {
            set_bit(&mut data.block_mask, b);
            data.block.push(layers.block.clone());
        } else {
            set_bit(&mut data.empty_block_mask, b);
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_entity_blocks_are_recognised() {
        assert_eq!(
            block_entity_type("minecraft:oak_sign"),
            Some("minecraft:sign")
        );
        assert_eq!(
            block_entity_type("minecraft:oak_wall_hanging_sign"),
            Some("minecraft:hanging_sign")
        );
        assert_eq!(
            block_entity_type("minecraft:red_shulker_box"),
            Some("minecraft:shulker_box")
        );
        assert_eq!(block_entity_type("minecraft:stone"), None);
    }

    #[test]
    fn block_entities_come_from_the_cached_section_in_scan_order() {
        let builtin = Builtin::parse(
            r#"{"minecraft:block_entity_type": {"entries": {
                "minecraft:chest": {"protocol_id": 1},
                "minecraft:sign": {"protocol_id": 7}}}}"#,
        )
        .unwrap();
        let states = [
            "minecraft:air",
            "minecraft:stone",
            "minecraft:chest",
            "minecraft:oak_sign",
        ]
        .map(|n| (Arc::<str>::from(n), false));
        let encoder = ChunkEncoder::new(states, &builtin, &[], &[]);
        let mut chunk = Chunk::empty(2, -3);
        for (x, y, z, state) in [(15, 3, 0, 2), (0, 3, 15, 3), (4, 3, 4, 1), (7, 40, 2, 2)] {
            chunk.set(x, MIN_Y + y, z, state);
        }
        // Every block of a section looked at, as the scan did before the cache.
        let scan = |chunk: &Chunk| {
            let mut found = Vec::new();
            for (i, s) in chunk.sections.iter().enumerate() {
                for y in 0..16 {
                    for z in 0..16 {
                        for x in 0..16 {
                            let state = s.get(crate::chunk::index(x, y, z));
                            if let Some(kind) = encoder.entity_type(state) {
                                found.push((x, MIN_Y + i as i32 * 16 + y, z, kind));
                            }
                        }
                    }
                }
            }
            // Then the order the packet gives them, as encode_blocks does.
            java_hash_order(&mut found, |e| {
                (chunk.x * 16 + e.0, e.1, chunk.z * 16 + e.2)
            });
            found
        };
        let sent = |chunk: &Chunk| {
            let packet = encoder.encode(chunk, &HashMap::new());
            let infos = packet.chunk.block_entities;
            infos
                .iter()
                .map(|e| {
                    let xz = e.packed_xz as u8;
                    (
                        i32::from(xz >> 4),
                        i32::from(e.y),
                        i32::from(xz & 15),
                        e.kind,
                    )
                })
                .collect::<Vec<_>>()
        };
        let expected = scan(&chunk);
        assert_eq!(expected.len(), 3);
        assert_eq!(sent(&chunk), expected);
        // From the cache the second time, and after an edit.
        assert_eq!(sent(&chunk), expected);
        chunk.set(15, MIN_Y + 3, 0, 0);
        chunk.set(9, MIN_Y + 3, 9, 3);
        assert_eq!(sent(&chunk), scan(&chunk));
    }

    fn encoder() -> ChunkEncoder {
        let states = ["minecraft:air", "minecraft:stone", "minecraft:water"]
            .map(|n| (Arc::<str>::from(n), n.ends_with("water")));
        ChunkEncoder::new(states, &Builtin::default(), &[], &[])
    }

    /// The bytes on the wire for a section, encoded afresh.
    fn fresh(encoder: &ChunkEncoder, section: &Section) -> Vec<u8> {
        let mut w = horizon_protocol::buf::Writer::new();
        encoder.section(section).0.write(&mut w);
        w.data
    }

    #[test]
    fn cached_sections_follow_edits() {
        let encoder = encoder();
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(1, MIN_Y + 2, 3, 1);
        let before = encoder.encode(&chunk, &HashMap::new());
        assert_eq!(encoder.occupied(&chunk), 1);
        assert_eq!(
            &*encoder.section_wire(&chunk.sections[0]).0,
            fresh(&encoder, &chunk.sections[0])
        );
        // An edit moves the section: the cache sees a new one.
        chunk.set(4, MIN_Y + 20, 4, 2);
        chunk.set(1, MIN_Y + 2, 3, 0);
        assert_eq!(encoder.occupied(&chunk), 2);
        let after = encoder.encode(&chunk, &HashMap::new());
        assert_ne!(before.chunk.buffer, after.chunk.buffer);
        let mut expected = Vec::new();
        for s in &chunk.sections {
            expected.extend(fresh(&encoder, s));
        }
        assert_eq!(after.chunk.buffer, expected);
    }

    #[test]
    fn light_layers_pack_two_blocks_per_byte() {
        let mut chunk = Chunk::empty(0, 0);
        let mut light = Box::new([0u8; 4096]);
        for (i, v) in light.iter_mut().enumerate() {
            *v = (i * 37 % 251) as u8;
        }
        Arc::make_mut(&mut chunk.sections[3]).light = Light::Full(light.clone());
        let data = light_data(&chunk.sections, u64::MAX);
        // Layer 4 holds section 3. Below the world (layer 0) there is no sky
        // light, so the sky layers listed start at layer 1; section 3 has the
        // only block light.
        let (sky, block) = (&data.sky[3], &data.block[0]);
        for j in 0..4096 {
            let (s, b) = if j & 1 == 0 {
                (sky[j >> 1] & 15, block[j >> 1] & 15)
            } else {
                (sky[j >> 1] >> 4, block[j >> 1] >> 4)
            };
            assert_eq!((s, b), (light[j] >> 4, light[j] & 15), "block {j}");
        }
    }

    #[test]
    fn cached_light_follows_edits() {
        let encoder = encoder();
        let mut chunk = Chunk::empty(0, 0);
        let mut light = Box::new([0x5Au8; 4096]);
        Arc::make_mut(&mut chunk.sections[2]).light = Light::Full(light.clone());
        let first = encoder.encode(&chunk, &HashMap::new()).light;
        assert_eq!(first, light_data(&chunk.sections, u64::MAX));
        // Relighting moves the section, so the cached layers aren't reused.
        light[7] = 0xF3;
        Arc::make_mut(&mut chunk.sections[2]).light = Light::Full(light);
        let second = encoder.encode(&chunk, &HashMap::new()).light;
        assert_ne!(first, second);
        assert_eq!(second, light_data(&chunk.sections, u64::MAX));
    }

    #[test]
    fn light_masks_cover_the_world_plus_borders() {
        let chunk = Chunk::empty(0, 0);
        // A column with nothing around it has no light layers at all.
        assert!(light_data(&chunk.sections, 0).sky.is_empty());
        let data = light_data(&chunk.sections, u64::MAX);
        // Default sections are full sky light and no block light.
        assert_eq!(data.sky.len(), SECTIONS + 1);
        assert_eq!(data.block.len(), 0);
        assert_eq!(data.sky[0][0], 0xFF);
        assert!(horizon_protocol::buf::bit(&data.empty_block_mask, 5));
    }

    #[test]
    fn a_nether_column_sends_its_sixteen_sections_without_sky_light() {
        let chunk = Chunk::empty(0, 0);
        let mut encoder = ChunkEncoder::new(
            [("minecraft:air".into(), false)],
            &Builtin::default(),
            &[],
            &[],
        );
        let overworld = encoder.encode(&chunk, &HashMap::new());
        encoder.set_dimension(0, 256, false);
        let nether = encoder.encode(&chunk, &HashMap::new());
        // An all-air section is the same bytes wherever it is.
        let per_section = overworld.chunk.buffer.len() / SECTIONS;
        assert_eq!(nether.chunk.buffer.len(), per_section * 16);
        assert!(nether.light.sky.is_empty() && nether.light.sky_mask.is_empty());
        // Block light layers: one below and one above the sixteen sections.
        assert!(horizon_protocol::buf::bit(
            &nether.light.empty_block_mask,
            17
        ));
        assert!(!horizon_protocol::buf::bit(
            &nether.light.empty_block_mask,
            18
        ));
    }
}
