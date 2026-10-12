//! Encoding resident chunks for the wire: ClientboundLevelChunkWithLightPacket
//! (sections with their paletted containers, block entities and the sky and
//! block DataLayers) and the section block-change batches.
use crate::{
    chunk::{Chunk, Light, MIN_Y, SECTIONS, Section},
    containers::BlockEntity,
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
}
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
        }
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

    fn section(&self, section: &Section) -> (horizon_protocol::chunk::Section, bool) {
        let mut values = Vec::with_capacity(BLOCKS);
        let (mut non_empty, mut fluids, mut entities) = (0i16, 0i16, false);
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
        let mut unpacked = [0u16; BLOCKS];
        section.unpack(&mut unpacked);
        for &v in &unpacked {
            entities |= count(v, 1);
            values.push(i32::from(v));
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
                states: Paletted::from_values(&values, Strategy::Blocks, 16),
                biomes: Paletted::from_values(&biome_values, Strategy::Biomes, self.biome_bits),
            },
            entities,
        )
    }

    /// The chunk column with its light and block entities.
    /// The non-air sections of a column as bits (LevelChunkSection.hasOnlyAir).
    fn occupied(&self, chunk: &Chunk) -> u64 {
        let mut bits = 0u64;
        for (i, s) in chunk.sections.iter().enumerate() {
            let mut unpacked = [0u16; BLOCKS];
            s.unpack(&mut unpacked);
            let any = unpacked.iter().any(|&v| {
                self.flags.get(usize::from(v)).copied().unwrap_or(0) & NON_AIR != 0
            });
            if any {
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
        let mut packet = self.encode(chunk, &world.block_entities);
        packet.light = light_data(chunk, around);
        packet
    }

    pub fn encode(
        &self,
        chunk: &Chunk,
        block_entities: &HashMap<(i32, i32, i32), BlockEntity>,
    ) -> LevelChunkWithLight {
        let mut sections = Vec::with_capacity(SECTIONS);
        let mut entity_infos = Vec::new();
        for (i, s) in chunk.sections.iter().enumerate() {
            let (encoded, has_entities) = self.section(s);
            if has_entities {
                for y in 0..16 {
                    for z in 0..16 {
                        for x in 0..16 {
                            let state = s.get(crate::chunk::index(x, y, z));
                            if let Some(kind) = self.entity_type(state) {
                                let wy = MIN_Y + (i as i32) * 16 + y;
                                let pos = (chunk.x * 16 + x, wy, chunk.z * 16 + z);
                                entity_infos.push(BlockEntityInfo {
                                    packed_xz: ((x << 4) | z) as i8,
                                    y: wy as i16,
                                    kind,
                                    // BlockEntityInfo.create: an empty update tag is not sent.
                                    tag: block_entities.get(&pos).and_then(entity_tag).unwrap_or(Tag::End),
                                });
                            }
                        }
                    }
                }
            }
            sections.push(encoded);
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
                    .map(|t| crate::save::client_heightmaps(chunk, t))
                    .unwrap_or_default(),
                buffer: ChunkData::from_sections(&sections),
                block_entities: entity_infos,
            },
            light: light_data(chunk, u64::MAX),
        }
    }
}

/// The iteration order of a java.util.HashMap keyed by BlockPos (LevelChunk's
/// block entities) filled in the given order: by bucket at the capacity the
/// map grew to, insertion order within a bucket (resizes keep it).
fn java_hash_order<T>(items: &mut [T], pos: impl Fn(&T) -> (i32, i32, i32)) {
    let mut capacity = 16usize;
    while items.len() * 4 > capacity * 3 {
        capacity *= 2;
    }
    items.sort_by_key(|item| {
        let (x, y, z) = pos(item);
        // Vec3i.hashCode, then HashMap.hash.
        let h = y.wrapping_add(z.wrapping_mul(31)).wrapping_mul(31).wrapping_add(x);
        ((h ^ ((h as u32) >> 16) as i32) as u32 as usize) & (capacity - 1)
    });
}

/// The update tag of a block entity (getUpdateTag) for what the client shows.
pub fn entity_tag(entity: &BlockEntity) -> Option<Tag> {
    match entity {
        BlockEntity::Sign(sign) => Some(Tag::compound(sign.encode())),
        BlockEntity::Jukebox(_) | BlockEntity::Lectern(_) => None,
        _ => None,
    }
}

/// Light sections run from one below the world to one above it. A layer
/// exists where vanilla's light storage keeps one: within one section of a
/// non-air section of this column or a neighbouring one (`around`, bit i for
/// section i; LayerLightSectionStorage.updateSectionStatus). An existing
/// layer is sent, or marked empty when all dark; the others are left out.
fn light_data(chunk: &Chunk, around: u64) -> LightData {
    let mut data = LightData {
        sky_mask: Vec::new(),
        block_mask: Vec::new(),
        empty_sky_mask: Vec::new(),
        empty_block_mask: Vec::new(),
        sky: Vec::new(),
        block: Vec::new(),
    };
    // Light section b holds chunk section b - 1.
    let exists = |b: usize| {
        (b.saturating_sub(2)..=b.min(SECTIONS - 1)).any(|s| s + 2 >= b && around >> s & 1 != 0)
    };
    let full_sky = Light::Uniform(0xF0);
    for b in 0..SECTIONS + 2 {
        if !exists(b) {
            continue;
        }
        let light = match b {
            // Below the world nothing is lit; above it the sky is full.
            0 => &Light::Uniform(0),
            _ if b == SECTIONS + 1 => &full_sky,
            _ => &chunk.sections[b - 1].light,
        };
        let (mut sky, mut block) = (vec![0u8; 2048], vec![0u8; 2048]);
        let (mut sky_any, mut block_any) = (false, false);
        match light {
            Light::Uniform(v) => {
                let (sk, bl) = (v >> 4, v & 15);
                sky_any = sk != 0;
                block_any = bl != 0;
                let (sb, bb) = (sk | sk << 4, bl | bl << 4);
                sky.fill(sb);
                block.fill(bb);
            }
            Light::Full(a) => {
                for (j, &v) in a.iter().enumerate() {
                    let (sk, bl) = (v >> 4, v & 15);
                    sky_any |= sk != 0;
                    block_any |= bl != 0;
                    if j & 1 == 0 {
                        sky[j >> 1] |= sk;
                        block[j >> 1] |= bl;
                    } else {
                        sky[j >> 1] |= sk << 4;
                        block[j >> 1] |= bl << 4;
                    }
                }
            }
        }
        if sky_any {
            set_bit(&mut data.sky_mask, b);
            data.sky.push(sky);
        } else {
            set_bit(&mut data.empty_sky_mask, b);
        }
        if block_any {
            set_bit(&mut data.block_mask, b);
            data.block.push(block);
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
    fn light_masks_cover_the_world_plus_borders() {
        let chunk = Chunk::empty(0, 0);
        let data = light_data(&chunk);
        // Default sections are full sky light and no block light.
        assert_eq!(data.sky.len(), SECTIONS + 1);
        assert_eq!(data.block.len(), 0);
        assert_eq!(data.sky[0][0], 0xFF);
        assert!(horizon_protocol::buf::bit(&data.empty_block_mask, 5));
    }
}
