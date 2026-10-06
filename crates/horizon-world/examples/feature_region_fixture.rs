use horizon_core::random::{LegacyRandom, RandomSource, WorldgenRandom, XoroshiroRandom};
use horizon_world::worldgen::{
    anchor::WorldGenerationContext,
    block_feature::FeatureBlockBindings,
    feature::FeatureBlocks,
    feature_graph::FeatureDefinition,
    feature_region::{RegionError, TerrainFeatureAccess, TerrainRegion},
    placement::{FeaturePlacer, PlacementContext, PlacementModifier},
    terrain::TerrainError,
};
// Feature chains against a resident multi-chunk region.
use horizon_core::position::ChunkPos;
use horizon_world::{
    DATA_VERSION,
    fluid::FluidKind,
    heightmap::{HeightmapState, HeightmapType},
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
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
fn tag(v: &Value) -> Tag {
    match v {
        Value::String(s) => Tag::String(s.as_str().into()),
        Value::Number(i) => Tag::Int(i.as_i64().unwrap() as i32),
        Value::Object(m) => {
            Tag::Compound(m.iter().map(|(k, v)| (k.as_str().into(), tag(v))).collect())
        }
        Value::Array(a) => Tag::List(a.iter().map(tag).collect()),
        _ => panic!("unsupported registry fixture value"),
    }
}
fn fixture(
    f: &Value,
    registry: &BlockRegistry,
    states: &[State],
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let seed = f["seed"].as_i64().unwrap();
    match (
        f["legacy"].as_bool().unwrap(),
        f["worldgen"].as_bool().unwrap(),
    ) {
        (true, false) => run(f, registry, states, out, LegacyRandom::new(seed)),
        (true, true) => run(
            f,
            registry,
            states,
            out,
            WorldgenRandom::new(LegacyRandom::new(seed)),
        ),
        (false, false) => run(f, registry, states, out, XoroshiroRandom::new(seed)),
        (false, true) => run(
            f,
            registry,
            states,
            out,
            WorldgenRandom::new(XoroshiroRandom::new(seed)),
        ),
    }
}
fn run(
    f: &Value,
    registry: &BlockRegistry,
    states: &[State],
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
        );
    let blocks = FeatureBlocks::from_registry(registry)?;
    // These configured fixture states use compact names and an enum property;
    // no integer-property parsing is requested by this region corpus.
    let bindings = FeatureBlockBindings::new(registry, BTreeSet::new())?;
    let features = f["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| FeatureDefinition::from_json_with_blocks(&v.to_string(), &bindings))
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
                let mut world = TerrainFeatureAccess::new(&mut region, write);
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
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).expect("private fixture directory");
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
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        fixture(&serde_json::from_str(line)?, &registry, &states, &mut out)?;
    }
    out.flush()?;
    Ok(())
}
