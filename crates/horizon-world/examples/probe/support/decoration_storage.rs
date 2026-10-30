use horizon_world::{
    DATA_VERSION,
    heightmap::HeightmapState,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{tags::TagDefinitions, terrain::TerrainBlockStates},
};
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path};
#[derive(Clone, Copy)]
pub struct State {
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
pub struct Properties<'a>(pub &'a [State]);
impl TerrainBlockStates for Properties<'_> {
    type State = State;
    fn heightmap_state(&self, id: BlockStateId) -> State {
        self.0[id.0 as usize]
    }
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
pub fn load(dir: &Path) -> Result<(BlockRegistry, Vec<State>), Box<dyn std::error::Error>> {
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
    Ok((registry, states))
}
