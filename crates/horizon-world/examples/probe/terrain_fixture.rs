//! Test transport for registry-backed noise-stage filling against vanilla doFill.
use crate::support::common::tag;
use horizon_core::{
    density::{
        Beardifier, JigsawJunction, RigidPiece, SamplerContext, StructureBounds, TerrainAdjustment,
        WorldField, WorldFields, graph::DensityCompiler,
    },
    noise::DensityVolume,
    position::ChunkPos,
    random::{LegacyRandom, PositionalRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::{
    DATA_VERSION,
    heightmap::{HeightmapState, HeightmapType},
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        aquifer::{
            AquiferConfig, DisabledAquifer, GlobalFluidPicker, NoiseBasedAquifer,
            RegistryAquiferBlockStates,
        },
        density::DensityRegistry,
        router::{NoiseRouter, RouterField},
        tags::TagDefinitions,
        terrain::{TerrainAquifer, TerrainBlockStates, TerrainChunk, fill_noise},
    },
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
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
fn structure(v: &Value) -> Beardifier {
    let bounds = |v: &Value| {
        StructureBounds::new(
            std::array::from_fn(|i| n(&v[i])),
            std::array::from_fn(|i| n(&v[i + 3])),
        )
        .unwrap()
    };
    let pieces = v["pieces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| RigidPiece {
            bounds: bounds(&a[0]),
            ground_level_delta: n(&a[2]),
            adjustment: match a[1].as_str().unwrap() {
                "NONE" => TerrainAdjustment::None,
                "BURY" => TerrainAdjustment::Bury,
                "BEARD_THIN" => TerrainAdjustment::BeardThin,
                "BEARD_BOX" => TerrainAdjustment::BeardBox,
                "ENCAPSULATE" => TerrainAdjustment::Encapsulate,
                _ => panic!("unknown adjustment"),
            },
        })
        .collect();
    let junctions = v["junctions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| JigsawJunction {
            source_x: n(&a[0]),
            source_ground_y: n(&a[1]),
            source_z: n(&a[2]),
        })
        .collect();
    Beardifier::new(pieces, junctions, Some(bounds(&v["affected"])))
}
fn fixture(
    f: Value,
    registry: &BlockRegistry,
    states: &[State],
    bundle: &Value,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let min = n(&f["min"]);
    let height = n(&f["height"]);
    let cx = n(&f["cx"]);
    let cz = n(&f["cz"]);
    let seed = f["seed"].as_i64().unwrap();
    let legacy = f["legacy"].as_bool().unwrap();
    let volume = DensityVolume::new(
        [16, n(&f["noise_height"]), 16],
        [cx.wrapping_shl(4), n(&f["noise_min"]), cz.wrapping_shl(4)],
        [1; 3],
    )?;
    let mut chunk = TerrainChunk::new(
        ChunkPos { x: cx, z: cz },
        min,
        height,
        registry,
        Properties(states),
    )?;
    for a in f["initial"].as_array().unwrap() {
        chunk.set_block_raw(n(&a[0]), n(&a[1]), n(&a[2]), BlockStateId(n(&a[3]) as u32))?;
    }
    let noises: BTreeMap<String, String> = serde_json::from_value(bundle["noises"].clone())?;
    let densities: BTreeMap<String, String> = serde_json::from_value(bundle["densities"].clone())?;
    let mut definitions = DensityRegistry::from_json(&noises, &densities)?;
    let router = NoiseRouter::from_json(&f["router"].to_string(), &mut definitions)?;
    let config = if f["aquifers"].is_null() {
        None
    } else {
        Some(AquiferConfig::from_json(
            &f["aquifers"].to_string(),
            &mut definitions,
        )?)
    };
    let mut compiler = DensityCompiler::new_seeded(definitions.into_graph(), seed, legacy);
    let mut context = SamplerContext::cached();
    if let Some(source) = f.get("structure") {
        context = context.with_world_fields(
            WorldFields::default().with_field(WorldField::Beardifier, Arc::new(structure(source))),
        );
    }
    let air = registry.default_state("minecraft:air").unwrap();
    let picker = GlobalFluidPicker::new(
        n(&f["sea"]),
        BlockStateId(n(&f["fluid"]) as u32),
        registry.default_state("minecraft:lava").unwrap(),
        air,
    );
    let mut aquifer: Box<dyn TerrainAquifer> = if let Some(config) = config {
        let samplers = config.compile(&mut compiler)?;
        let blocks = RegistryAquiferBlockStates::new(registry)?;
        if legacy {
            let random = LegacyRandom::new(seed)
                .fork_positional()
                .from_hash_of("minecraft:aquifer")
                .fork_positional();
            Box::new(NoiseBasedAquifer::new(
                samplers,
                picker,
                random,
                blocks,
                volume,
                &mut context,
            )?)
        } else {
            let random = XoroshiroRandom::new(seed)
                .fork_positional()
                .from_hash_of("minecraft:aquifer")
                .fork_positional();
            Box::new(NoiseBasedAquifer::new(
                samplers,
                picker,
                random,
                blocks,
                volume,
                &mut context,
            )?)
        }
    } else {
        Box::new(DisabledAquifer::new(picker, air))
    };
    let density = compiler.get_sampler(router.function(RouterField::FinalDensity))?;
    writeln!(out, "chunk {cx} {cz} {min} {height}")?;
    for repeat in 0..n(&f["repeats"]) {
        fill_noise(
            &mut chunk,
            volume,
            density.as_ref(),
            &mut context,
            aquifer.as_mut(),
            BlockStateId(n(&f["block"]) as u32),
        )?;
        writeln!(
            out,
            "fill {repeat} {}",
            aquifer.should_schedule_fluid_update()
        )?;
        for section in 0..(height / 16) as usize {
            writeln!(out, "section {section}")?;
            for index in 0..4096 {
                writeln!(out, "{}", chunk.section(section).unwrap().get(index)?.0)?;
            }
        }
        for kind in [HeightmapType::OceanFloorWg, HeightmapType::WorldSurfaceWg] {
            writeln!(out, "heightmap {}", kind.name())?;
            for word in chunk.heightmap(kind).unwrap().raw_data() {
                writeln!(out, "{word:x}")?;
            }
        }
        for (section, list) in chunk.postprocessing().iter().enumerate() {
            writeln!(out, "post {section} {}", list.len())?;
            for offset in list {
                writeln!(out, "{offset}")?;
            }
        }
    }
    Ok(())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let facts: Value = serde_json::from_str(&fs::read_to_string(dir.join("facts-server.json"))?)?;
    let bundle: Value = serde_json::from_str(&fs::read_to_string(dir.join("registries.json"))?)?;
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
    for (name, text) in bundle["tags"].as_object().unwrap() {
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
        fixture(
            serde_json::from_str(line)?,
            &registry,
            &states,
            &bundle,
            &mut out,
        )?;
    }
    out.flush()?;
    Ok(())
}
