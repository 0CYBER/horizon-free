//! Test transport for pre-light ProtoChunk writes and stage heightmap transitions.
use crate::support::common::tag;
use horizon_core::position::ChunkPos;
use horizon_world::{
    DATA_VERSION,
    chunk_status::ChunkStatus,
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
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut chunk = TerrainChunk::new(
        ChunkPos::new(n(&f["chunk"][0]), n(&f["chunk"][1])),
        n(&f["min"]),
        n(&f["height"]),
        registry,
        Properties(states),
    )?;
    writeln!(
        out,
        "chunk {} {} {} {}",
        n(&f["chunk"][0]),
        n(&f["chunk"][1]),
        chunk.min_y(),
        chunk.height()
    )?;
    for op in f["ops"].as_array().unwrap() {
        match op[0].as_str().unwrap() {
            "stage" => {
                let stage = PreLightChunkStage::ALL
                    .into_iter()
                    .find(|s| s.name() == op[1].as_str().unwrap())
                    .unwrap();
                chunk.set_generation_stage(stage);
                writeln!(out, "stage {}", chunk.generation_stage().name())?;
            }
            "upgrade" => {
                let data = if op[1].is_null() {
                    None
                } else {
                    Some(BelowZeroRetrogen::new(
                        ChunkStatus::parse(op[1].as_str().unwrap())?,
                        op[2]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_i64().unwrap() as u64)
                            .collect(),
                    )?)
                };
                chunk.set_below_zero_retrogen(data);
                writeln!(out, "upgrade")?;
            }
            "upgrade_info" => {
                let (min_y, height) = chunk.height_for_generation();
                writeln!(
                    out,
                    "upgrade_info {} {} {min_y} {height} {}",
                    chunk.is_upgrading(),
                    chunk
                        .below_zero_retrogen()
                        .map_or("none", |data| data.target_status().name()),
                    chunk.highest_generated_status().name()
                )?;
            }
            "replace_bedrock" => {
                BelowZeroRetrogen::replace_old_bedrock(&mut chunk)?;
                writeln!(out, "replace_bedrock")?;
            }
            "mask" => {
                let data = BelowZeroRetrogen::new(
                    ChunkStatus::Full,
                    op[1]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_i64().unwrap() as u64)
                        .collect(),
                )?;
                data.apply_bedrock_mask(&mut chunk)?;
                writeln!(out, "mask")?;
            }
            "fill" => {
                let states = op[1].as_array().unwrap();
                let salt = n(&op[2]);
                let p = chunk.position();
                for y in 0..chunk.height() {
                    for z in 0..16 {
                        for x in 0..16 {
                            let state = BlockStateId(n(
                                &states[((x + 3 * y + 5 * z + salt) as usize) % states.len()]
                            ) as u32);
                            chunk.set_block_worldgen(
                                p.x.wrapping_shl(4).wrapping_add(x),
                                chunk.min_y().wrapping_add(y),
                                p.z.wrapping_shl(4).wrapping_add(z),
                                state,
                            )?;
                        }
                    }
                }
                writeln!(out, "fill")?;
            }
            "set" => {
                let previous = chunk.set_block_worldgen(
                    n(&op[1]),
                    n(&op[2]),
                    n(&op[3]),
                    BlockStateId(n(&op[4]) as u32),
                )?;
                writeln!(out, "set {}", previous.0)?;
            }
            "get" => writeln!(
                out,
                "get {}",
                chunk.block(n(&op[1]), n(&op[2]), n(&op[3])).0
            )?,
            "prime" => {
                let kinds: Vec<_> = HeightmapType::ALL
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, k)| (n(&op[1]) & (1 << i) != 0).then_some(k))
                    .collect();
                chunk.prime_heightmaps(&kinds)?;
                writeln!(out, "prime {}", n(&op[1]))?;
            }
            "query" => {
                for kind in HeightmapType::ALL {
                    if let Some(map) = chunk.heightmap(kind) {
                        writeln!(
                            out,
                            "height {} {}",
                            kind.name(),
                            map.first_available(n(&op[1]) & 15, n(&op[2]) & 15)?
                        )?;
                    } else {
                        writeln!(out, "missing {}", kind.name())?;
                    }
                }
            }
            "dump" => {
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
                for i in 0..chunk.height() / 16 {
                    writeln!(out, "section {i}")?;
                    for index in 0..4096 {
                        writeln!(out, "{}", chunk.section(i as usize).unwrap().get(index)?.0)?;
                    }
                }
            }
            _ => panic!("unknown operation"),
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
