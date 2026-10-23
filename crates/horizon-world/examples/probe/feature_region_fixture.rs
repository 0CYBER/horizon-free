use crate::support::common::tag;
use horizon_core::random::{LegacyRandom, RandomSource, WorldgenRandom, XoroshiroRandom};
use horizon_world::worldgen::{
    anchor::WorldGenerationContext,
    block_feature::FeatureBlockBindings,
    feature::{FeatureBlocks, FeatureWorld},
    feature_graph::FeatureDefinition,
    feature_region::{RegionError, TerrainFeatureAccess, TerrainRegion},
    placement::{FeaturePlacer, PlacementContext, PlacementModifier},
    terrain::TerrainError,
    write_zone::{GenerationWriteZone, UpgradeWriteHeight},
};
// Feature chains against a resident multi-chunk region.
use horizon_core::position::ChunkPos;
use horizon_world::{
    DATA_VERSION,
    chunk_status::ChunkStatus,
    fluid::FluidKind,
    heightmap::{HeightmapState, HeightmapType},
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
    retrogen::BelowZeroRetrogen,
    worldgen::{
        tags::TagDefinitions,
        terrain::{PreLightChunkStage, TerrainBlockStates, TerrainChunk},
    },
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Clone, Copy)]
struct State {
    air: bool,
    air_block: bool,
    fluid: bool,
    motion: bool,
    no_leaves: bool,
}
impl HeightmapState for State {
    fn is_air(self) -> bool {
        self.air
    }
    fn is_air_block(self) -> bool {
        self.air_block
    }
    fn has_fluid(self) -> bool {
        self.fluid
    }
    fn blocks_motion_in_heightmap(self) -> bool {
        self.motion
    }
    fn blocks_motion_in_heightmap_no_leaves(self) -> bool {
        self.no_leaves
    }
}
struct Properties<'a>(&'a [State]);
impl TerrainBlockStates for Properties<'_> {
    type State = State;
    fn heightmap_state(&self, id: BlockStateId) -> State {
        self.0[id.0 as usize]
    }
}
fn n(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}
fn fixture(
    f: &Value,
    registry: &BlockRegistry,
    states: &[State],
    bindings: &FeatureBlockBindings<'_>,
    tags: &[String],
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let seed = f["seed"].as_i64().unwrap();
    match (
        f["legacy"].as_bool().unwrap(),
        f["worldgen"].as_bool().unwrap(),
    ) {
        (true, false) => run(
            f,
            registry,
            states,
            bindings,
            tags,
            out,
            LegacyRandom::new(seed),
        ),
        (true, true) => run(
            f,
            registry,
            states,
            bindings,
            tags,
            out,
            WorldgenRandom::new(LegacyRandom::new(seed)),
        ),
        (false, false) => run(
            f,
            registry,
            states,
            bindings,
            tags,
            out,
            XoroshiroRandom::new(seed),
        ),
        (false, true) => run(
            f,
            registry,
            states,
            bindings,
            tags,
            out,
            WorldgenRandom::new(XoroshiroRandom::new(seed)),
        ),
    }
}
// Reuse one feature access object while successful callbacks change the center.
fn upgrade_lifecycle(
    registry: &BlockRegistry,
    states: &[State],
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    use horizon_world::worldgen::biome_chunk::{BiomeChunk, BiomeRegion};
    let biomes = BiomeRegion::new(
        (0..2)
            .map(|x| BiomeChunk::new(ChunkPos::new(x, 0), -64, 128, ()).unwrap())
            .collect(),
        0,
    )?;
    let stone = registry.default_state("minecraft:stone").unwrap();
    for stage in PreLightChunkStage::ALL {
        for target in ChunkStatus::ALL.into_iter().skip(1) {
            writeln!(out, "lifecycle {} {}", stage.status().name(), target.name())?;
            let mut chunks = Vec::new();
            for x in 0..2 {
                let mut chunk =
                    TerrainChunk::new(ChunkPos::new(x, 0), -64, 128, registry, Properties(states))?;
                chunk.set_generation_stage(stage);
                chunk.set_below_zero_retrogen(Some(BelowZeroRetrogen::new(target, vec![])?));
                chunks.push(chunk);
            }
            let mut region = TerrainRegion::new(chunks, WorldGenerationContext::new(-64, 128, 0))?;
            for pass in 0..2 {
                for x in 0..2 {
                    writeln!(
                        out,
                        "lifecycle_status {pass} {x} {} {}",
                        region.chunk(x, 0)?.highest_generated_status().name(),
                        region.noise_biome(&biomes, [x * 4, i32::MAX, 0]).is_ok()
                    )?;
                }
                if pass != 0 {
                    break;
                }
                let write = |region: &mut TerrainRegion<'_, Properties<'_>>,
                             _: [i32; 3],
                             _: BlockStateId,
                             flags: i32|
                 -> Result<bool, RegionError> {
                    let chunk = region.chunk_mut(0, 0)?;
                    match flags {
                        1 => chunk.set_generation_stage(PreLightChunkStage::Features),
                        2 => chunk.set_below_zero_retrogen(Some(
                            BelowZeroRetrogen::new(target, vec![]).unwrap(),
                        )),
                        3 => chunk.set_below_zero_retrogen(None),
                        4 => chunk.set_generation_stage(stage),
                        _ => {}
                    }
                    Ok(true)
                };
                let mut world = TerrainFeatureAccess::new(&mut region, write)
                    .with_write_zone(GenerationWriteZone::new(ChunkPos::new(0, 0), 1))
                    .with_upgrade_center(ChunkPos::new(0, 0))?;
                for (x, y, flags) in [
                    (16, 0, 0),
                    (16, -1, 1),
                    (16, 0, 0),
                    (0, -64, 3),
                    (16, 0, 2),
                    (16, 0, 0),
                    (0, -1, 4),
                    (16, 0, 0),
                    (0, -65, 0),
                    (0, -1, 3),
                    (16, i32::MAX, 0),
                ] {
                    writeln!(
                        out,
                        "lifecycle_write {x} {y} {flags} {}",
                        world.set_block([x, y, 0], stone, flags)?
                    )?;
                }
            }
        }
    }
    Ok(())
}
fn run(
    f: &Value,
    registry: &BlockRegistry,
    states: &[State],
    bindings: &FeatureBlockBindings<'_>,
    tags: &[String],
    out: &mut impl Write,
    mut random: impl RandomSource,
) -> Result<(), Box<dyn std::error::Error>> {
    let stage = PreLightChunkStage::ALL
        .into_iter()
        .find(|s| s.name() == f["stage"].as_str().unwrap())
        .unwrap();
    let mut chunks = Vec::new();
    for cx in -1..=1 {
        for cz in -1..=1 {
            let mut chunk =
                TerrainChunk::new(ChunkPos::new(cx, cz), -64, 64, registry, Properties(states))?;
            chunk.set_generation_stage(stage);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set_block_worldgen(
                        (cx << 4) + x,
                        -56,
                        (cz << 4) + z,
                        registry.default_state("minecraft:stone").unwrap(),
                    )?;
                    if (x + z) % 3 != 0 {
                        chunk.set_block_worldgen(
                            (cx << 4) + x,
                            -48,
                            (cz << 4) + z,
                            registry.default_state("minecraft:dirt").unwrap(),
                        )?;
                    }
                }
            }
            chunks.push(chunk);
        }
    }
    let mut region = TerrainRegion::new(chunks, WorldGenerationContext::new(-64, 64, 0))?
        .with_tick_clock(
            f["game_time"].as_i64().unwrap(),
            f["tick_order"].as_i64().unwrap(),
        )
        .with_block_bindings(bindings)?;
    if f["upgrading"].as_bool().unwrap() {
        region
            .chunk_mut(0, 0)?
            .set_below_zero_retrogen(Some(BelowZeroRetrogen::new(ChunkStatus::Features, vec![])?));
    }
    let blocks = FeatureBlocks::from_registry(registry)?;
    let features = f["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| FeatureDefinition::from_json_with_blocks(&v.to_string(), bindings))
        .collect::<Result<Vec<_>, _>>()?;
    let modifiers = f["placement"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| PlacementModifier::from_json(&v.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut placer = FeaturePlacer::new();
    let fail = n(&f["fail"]);
    let reject = n(&f["reject"]);
    for repeat in 0..2 {
        for feature in &features {
            let mut calls = 0;
            let result = {
                let write = |region: &mut TerrainRegion<'_, Properties<'_>>,
                             p: [i32; 3],
                             state: BlockStateId,
                             flags: i32|
                 -> Result<bool, RegionError> {
                    writeln!(out, "write {} {} {} {} {flags}", p[0], p[1], p[2], state.0)
                        .map_err(TerrainError::from)?;
                    let call = calls;
                    calls += 1;
                    if call == fail {
                        return Err(TerrainError::Storage(io::Error::other(
                            "injected write failure",
                        ))
                        .into());
                    }
                    let result = reject == 0 || call % reject != 0;
                    if result {
                        region.set_block_worldgen(p, state)?;
                    }
                    writeln!(out, "written {result}").map_err(TerrainError::from)?;
                    Ok(result)
                };
                let mut world = TerrainFeatureAccess::new(&mut region, write)
                    .with_upgrade_center(ChunkPos::new(0, 0))?;
                if let Some(zone) = f["write_zone"].as_array() {
                    world = world.with_write_zone(GenerationWriteZone::new(
                        ChunkPos::new(n(&zone[0]), n(&zone[1])),
                        n(&zone[2]),
                    ));
                }
                placer.place_with_context(
                    &modifiers,
                    &mut world,
                    &mut random,
                    [15, -40, 15],
                    |world, random, p| feature.place(world, random, p, blocks),
                )
            };
            match result {
                Ok(result) => writeln!(out, "place {result}")?,
                Err(_) => writeln!(out, "place error")?,
            }
            writeln!(out, "next {}", random.next_long())?;
            for [x, z] in [[0, 0], [15, 15], [16, 16], [-1, -1]] {
                for kind in HeightmapType::ALL {
                    writeln!(
                        out,
                        "height {} {x} {z} {}",
                        kind.name(),
                        region.height(kind, x, z)?
                    )?;
                }
            }
        }
        writeln!(out, "repeat {repeat}")?;
    }
    for y in [i32::MIN, -65, -64, -63, -1, 0, 1, 319, 320, i32::MAX] {
        for x in [0, 16] {
            let result = {
                let write = |region: &mut TerrainRegion<'_, Properties<'_>>,
                             p: [i32; 3],
                             state: BlockStateId,
                             flags: i32|
                 -> Result<bool, RegionError> {
                    writeln!(out, "write {} {} {} {} {flags}", p[0], p[1], p[2], state.0)
                        .map_err(TerrainError::from)?;
                    let result = reject == 0;
                    if result {
                        region.set_block_worldgen(p, state)?;
                    }
                    writeln!(out, "written {result}").map_err(TerrainError::from)?;
                    Ok(result)
                };
                let mut world = TerrainFeatureAccess::new(&mut region, write)
                    .with_upgrade_center(ChunkPos::new(0, 0))?;
                if let Some(zone) = f["write_zone"].as_array() {
                    world = world.with_write_zone(GenerationWriteZone::new(
                        ChunkPos::new(n(&zone[0]), n(&zone[1])),
                        n(&zone[2]),
                    ));
                }
                world.set_block(
                    [x, y, 0],
                    registry.default_state("minecraft:stone").unwrap(),
                    2,
                )?
            };
            writeln!(out, "heightwrite {x} {y} {result}")?;
        }
    }
    for command in f["tick_commands"].as_array().unwrap() {
        let kind = command[0].as_str().unwrap();
        let id = n(&command[1]) as u32;
        let p = std::array::from_fn(|i| n(&command[2][i]));
        let delay = n(&command[3]);
        writeln!(out, "tickrequest {kind} {id}")?;
        let result = if kind == "block" {
            region.schedule_block_tick(p, BlockStateId(id), delay)
        } else {
            region.schedule_fluid_tick(p, FluidKind::ALL[id as usize], delay)
        };
        writeln!(
            out,
            "tickresult {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        writeln!(out, "tickorder {}", region.tick_clock().1)?;
    }
    for id in f["probe_states"].as_array().unwrap() {
        let id = n(id) as u32;
        let p = [[0, -64, 0], [-1, -49, -1], [16, -1, 16], [31, -32, 31]][id as usize % 4];
        writeln!(out, "probe {id}")?;
        for state in [
            BlockStateId(id),
            registry.default_state("minecraft:air").unwrap(),
        ] {
            region.set_block_worldgen(p, state)?;
            let mut world = TerrainFeatureAccess::new(
                &mut region,
                |_: &mut TerrainRegion<'_, Properties<'_>>,
                 _: [i32; 3],
                 _: BlockStateId,
                 _: i32| Ok::<_, RegionError>(false),
            );
            query_state(&mut world, p, tags, out)?;
        }
    }
    for p in [
        [0, -65, 0],
        [31, 0, 31],
        [-1, i32::MIN, -1],
        [16, i32::MAX, 16],
        [-17, -40, 0],
        [32, -40, 0],
        [i32::MAX, i32::MIN, 0],
    ] {
        query_state(&mut region, p, tags, out)?;
    }
    for chunk in region.chunks() {
        let p = chunk.position();
        writeln!(out, "chunk {} {}", p.x, p.z)?;
        writeln!(out, "blockticks {}", chunk.block_ticks().count())?;
        for tick in chunk.block_ticks().pack(region.tick_clock().0) {
            writeln!(
                out,
                "bt {} {} {} {} {} {}",
                tick.kind.0, tick.pos.x, tick.pos.y, tick.pos.z, tick.delay, tick.priority as i8
            )?;
        }
        writeln!(out, "fluidticks {}", chunk.fluid_ticks().count())?;
        for tick in chunk.fluid_ticks().pack(region.tick_clock().0) {
            writeln!(
                out,
                "ft {} {} {} {} {} {}",
                FluidKind::ALL[tick.kind.0 as usize].name(),
                tick.pos.x,
                tick.pos.y,
                tick.pos.z,
                tick.delay,
                tick.priority as i8
            )?;
        }
        for kind in HeightmapType::ALL {
            if let Some(map) = chunk.heightmap(kind) {
                writeln!(out, "map {}", kind.name())?;
                for word in map.raw_data() {
                    writeln!(out, "{word:x}")?;
                }
            } else {
                writeln!(out, "missing {}", kind.name())?;
            }
        }
        for i in 0..4 {
            writeln!(out, "section {i}")?;
            for index in 0..4096 {
                writeln!(out, "{}", chunk.section(i).unwrap().get(index)?.0)?;
            }
        }
    }
    writeln!(out, "tail {}", random.next_long())?;
    Ok(())
}
fn query_state(
    world: &mut impl PlacementContext,
    p: [i32; 3],
    tags: &[String],
    out: &mut impl Write,
) -> io::Result<()> {
    writeln!(out, "query {} {} {}", p[0], p[1], p[2])?;
    match world.fluid_state(p) {
        Ok(fluid) => writeln!(
            out,
            "fluid {} {} {} {} {}",
            fluid.kind().name(),
            fluid.amount(),
            fluid.falling(),
            fluid.is_full(),
            fluid.is_source()
        )?,
        Err(_) => writeln!(out, "fluid error")?,
    }
    match world.water_at(p) {
        Ok(water) => writeln!(out, "water {water}")?,
        Err(_) => writeln!(out, "water error")?,
    }
    let mut bits = String::new();
    for tag in tags {
        bits.push(match world.block_matches_tag(p, tag) {
            Ok(true) => '1',
            Ok(false) => '0',
            Err(_) => 'E',
        });
    }
    writeln!(out, "tags {bits}")
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let facts: Value = serde_json::from_str(&fs::read_to_string(dir.join("facts-server.json"))?)?;
    let bundle: Value = serde_json::from_str(&fs::read_to_string(dir.join("tags.json"))?)?;
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(
                facts["states"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| tag(&s[4]))
                    .collect(),
            ),
        ),
        ("defaults", tag(&facts["defaults"])),
    ]))?;
    let mut definitions = TagDefinitions::default();
    for (name, text) in bundle.as_object().unwrap() {
        definitions.apply_json(name, "vanilla", text.as_str().unwrap())?;
    }
    let built = definitions.build(|id, _| facts["defaults"].get(id).map(|_| id.to_owned()))?;
    assert!(built.missing.is_empty());
    let motion: BTreeSet<_> = built.tags["minecraft:blocks_motion_in_heightmap"]
        .iter()
        .map(String::as_str)
        .collect();
    let no_leaves: BTreeSet<_> = built.tags["minecraft:blocks_motion_in_heightmap_no_leaves"]
        .iter()
        .map(String::as_str)
        .collect();
    let states: Vec<_> = facts["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            let name = a[0].as_str().unwrap();
            State {
                air: a[1].as_bool().unwrap(),
                air_block: name == "minecraft:air",
                fluid: a[2].as_bool().unwrap(),
                motion: motion.contains(name),
                no_leaves: no_leaves.contains(name),
            }
        })
        .collect();
    let fluid_resources: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("fluid-tags.json"))?)?;
    let mut fluid_defs = TagDefinitions::default();
    for (name, text) in fluid_resources {
        fluid_defs.apply_json(&name, "vanilla", &text)?;
    }
    let fluid_tags =
        fluid_defs.build(|name, _| FluidKind::from_name(name).map(|_| name.to_owned()))?;
    assert!(fluid_tags.missing.is_empty());
    let bindings = FeatureBlockBindings::with_tags(&registry, BTreeSet::new(), &built.tags)?
        .with_fluid_tags(&fluid_tags.tags)?;
    let tags: Vec<_> = built
        .tags
        .keys()
        .cloned()
        .chain(std::iter::once("horizon:absent".into()))
        .collect();
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    let zones: Value = serde_json::from_str(&fs::read_to_string(dir.join("zones.json"))?)?;
    for row in zones.as_array().unwrap() {
        let zone = GenerationWriteZone::new(ChunkPos::new(n(&row[0]), n(&row[1])), n(&row[2]));
        let (x, z) = (n(&row[3]), n(&row[4]));
        writeln!(
            out,
            "zone {} {}",
            zone.contains_chunk(ChunkPos::new(x, z)),
            zone.contains_block(horizon_core::position::BlockPos::new(x, i32::MIN, z))
        )?;
    }
    let heights: Value = serde_json::from_str(&fs::read_to_string(dir.join("heights.json"))?)?;
    for row in heights.as_array().unwrap() {
        let y = n(&row[2]);
        writeln!(
            out,
            "heightgate {} true {}",
            UpgradeWriteHeight::new(n(&row[0]), n(&row[1])).contains(y),
            UpgradeWriteHeight::BELOW_ZERO.contains(y)
        )?;
    }
    upgrade_lifecycle(&registry, &states, &mut out)?;
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        fixture(
            &serde_json::from_str(line)?,
            &registry,
            &states,
            &bindings,
            &tags,
            &mut out,
        )?;
    }
    out.flush()?;
    Ok(())
}
