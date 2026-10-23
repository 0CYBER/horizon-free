use crate::support::common::tag;
use horizon_world::worldgen::{
    anchor::WorldGenerationContext, bulk_section::TerrainBulkAccess, feature_region::TerrainRegion,
};
// Feature chains against a resident multi-chunk region.
use horizon_core::position::ChunkPos;
use horizon_world::{
    DATA_VERSION,
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
fn fixture(
    f: &Value,
    registry: &BlockRegistry,
    states: &[State],
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let minimum = n(&f["min"]);
    let height = n(&f["height"]);
    let mut chunks = Vec::new();
    for p in f["chunks"].as_array().unwrap() {
        let mut chunk = TerrainChunk::new(
            ChunkPos::new(n(&p[0]), n(&p[1])),
            minimum,
            height,
            registry,
            Properties(states),
        )?;
        chunk.set_generation_stage(PreLightChunkStage::Terrain);
        if f["prime"].as_bool().unwrap() {
            chunk.prime_heightmaps(&HeightmapType::ALL)?;
        }
        chunks.push(chunk);
    }
    let mut region = TerrainRegion::new(chunks, WorldGenerationContext::new(minimum, height, 0))?;
    let mut bulk = TerrainBulkAccess::new(&mut region)?;
    for op in f["ops"].as_array().unwrap() {
        if op[0].as_str() == Some("reset") {
            drop(bulk);
            bulk = TerrainBulkAccess::new(&mut region)?;
            writeln!(out, "reset")?;
            writeln!(out, "cache 0")?;
            continue;
        }
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            match op[0].as_str().unwrap() {
                "height" => {
                    let kind = HeightmapType::ALL
                        .into_iter()
                        .find(|k| k.name() == op[1].as_str().unwrap())
                        .unwrap();
                    writeln!(out, "height {}", bulk.height(kind, n(&op[2]), n(&op[3]))?)?;
                }
                kind => {
                    let p = [n(&op[1]), n(&op[2]), n(&op[3])];
                    match kind {
                        "get" => writeln!(out, "get {}", bulk.block(p)?.0)?,
                        "world" => writeln!(out, "world {}", bulk.world_block(p)?.0)?,
                        "has" => writeln!(out, "has {}", bulk.has_section(p)?)?,
                        "set" => match bulk.set_block(p, BlockStateId(n(&op[4]) as u32))? {
                            Some(old) => writeln!(out, "set {}", old.0)?,
                            None => writeln!(out, "set none")?,
                        },
                        _ => panic!("unknown operation"),
                    }
                }
            }
            Ok(())
        })();
        if result.is_err() {
            writeln!(out, "error")?;
        }
        writeln!(out, "cache {}", bulk.acquired_section_count())?;
    }
    drop(bulk);
    for chunk in region.chunks() {
        writeln!(out, "chunk {} {}", chunk.position().x, chunk.position().z)?;
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
        for i in 0..height / 16 {
            writeln!(out, "section {i}")?;
            for index in 0..4096 {
                writeln!(out, "{}", chunk.section(i as usize).unwrap().get(index)?.0)?;
            }
        }
    }
    Ok(())
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
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        fixture(&serde_json::from_str(line)?, &registry, &states, &mut out)?;
    }
    out.flush()?;
    Ok(())
}
