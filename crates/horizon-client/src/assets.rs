use crate::Result;
use horizon_world::{
    DATA_VERSION,
    heightmap::HeightmapState,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{tags::TagDefinitions, terrain::TerrainBlockStates},
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{self, BufReader},
    path::Path,
    sync::Arc,
};

pub const MATERIALS: [&str; 6] = ["bedrock", "dirt", "stone", "sand", "oak_planks", "bricks"];
#[derive(Clone, Copy)]
pub struct StateFacts {
    air: bool,
    fluid: bool,
    air_block: bool,
    motion: bool,
    no_leaves: bool,
}
impl HeightmapState for StateFacts {
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
#[derive(Clone)]
pub struct Properties(Arc<Vec<StateFacts>>);
impl TerrainBlockStates for Properties {
    type State = StateFacts;
    fn heightmap_state(&self, id: BlockStateId) -> StateFacts {
        self.0[id.0 as usize]
    }
}
pub struct Assets {
    pub registry: BlockRegistry,
    pub properties: Properties,
    pub material_states: [BlockStateId; 6],
    pub layers: Vec<i32>,
    pub sprite_size: u32,
    pub pixels: Vec<u8>,
    pub texture_layers: u32,
    pub model_quads: Vec<crate::models::ModelInstance>,
    pub animations: Vec<crate::sprites::SpriteAnimation>,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn tag(value: &Value) -> Result<Tag> {
    Ok(match value {
        Value::String(value) => Tag::String(value.as_str().into()),
        Value::Number(value) => Tag::Int(i32::try_from(
            value
                .as_i64()
                .ok_or_else(|| invalid("invalid registry number"))?,
        )?),
        Value::Array(values) => Tag::List(values.iter().map(tag).collect::<Result<_>>()?),
        Value::Object(values) => Tag::Compound(
            values
                .iter()
                .map(|(name, value)| Ok((name.as_str().into(), tag(value)?)))
                .collect::<Result<_>>()?,
        ),
        _ => return Err(invalid("unsupported registry value").into()),
    })
}
impl Assets {
    pub fn load(root: &Path) -> Result<Self> {
        let manifest: Value = serde_json::from_reader(BufReader::new(File::open(
            root.join("horizon-resources.json"),
        )?))?;
        if manifest["data_version"].as_i64() != Some(i64::from(DATA_VERSION)) {
            return Err(invalid("resources do not match Horizon's pinned data version").into());
        }
        let facts: Value =
            serde_json::from_reader(BufReader::new(File::open(root.join("facts.json"))?))?;
        let states = facts["states"]
            .as_array()
            .ok_or_else(|| invalid("missing registry state facts"))?;
        let registry = BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            (
                "states",
                Tag::List(
                    states
                        .iter()
                        .map(|state| tag(&state[4]))
                        .collect::<Result<_>>()?,
                ),
            ),
            ("defaults", tag(&facts["defaults"])?),
        ]))?;
        let bundle: Value =
            serde_json::from_reader(BufReader::new(File::open(root.join("block-tags.json"))?))?;
        let mut definitions = TagDefinitions::default();
        for (name, text) in bundle
            .as_object()
            .ok_or_else(|| invalid("invalid block tag bundle"))?
        {
            definitions.apply_json(
                name,
                "vanilla",
                text.as_str()
                    .ok_or_else(|| invalid("invalid block tag text"))?,
            )?;
        }
        let built =
            definitions.build(|name, _| facts["defaults"].get(name).map(|_| name.to_owned()))?;
        if !built.missing.is_empty() {
            return Err(invalid("unresolved block tags").into());
        }
        let motion: BTreeSet<_> = built
            .tags
            .get("minecraft:blocks_motion_in_heightmap")
            .ok_or_else(|| invalid("missing motion tag"))?
            .iter()
            .map(String::as_str)
            .collect();
        let no_leaves: BTreeSet<_> = built
            .tags
            .get("minecraft:blocks_motion_in_heightmap_no_leaves")
            .ok_or_else(|| invalid("missing no-leaves motion tag"))?
            .iter()
            .map(String::as_str)
            .collect();
        let properties = Properties(Arc::new(
            states
                .iter()
                .map(|state| {
                    Ok(StateFacts {
                        air: state[1]
                            .as_bool()
                            .ok_or_else(|| invalid("missing air fact"))?,
                        fluid: state[2]
                            .as_bool()
                            .ok_or_else(|| invalid("missing fluid fact"))?,
                        air_block: state[0].as_str() == Some("minecraft:air"),
                        motion: motion.contains(
                            state[0]
                                .as_str()
                                .ok_or_else(|| invalid("missing state name"))?,
                        ),
                        no_leaves: no_leaves.contains(
                            state[0]
                                .as_str()
                                .ok_or_else(|| invalid("missing state name"))?,
                        ),
                    })
                })
                .collect::<Result<_>>()?,
        ));
        let mut layers: Vec<_> = properties
            .0
            .iter()
            .map(|state| if state.air { -1 } else { -2 })
            .collect();
        let mut material_states = [BlockStateId(0); 6];
        let mut sprites = crate::sprites::Sprites::default();
        for (layer, name) in MATERIALS.iter().enumerate() {
            let state = registry
                .default_state(&format!("minecraft:{name}"))
                .ok_or_else(|| invalid("missing test-scene material"))?;
            material_states[layer] = state;
            let sprite = sprites.insert(root, &format!("minecraft:block/{name}"))?;
            layers[state.0 as usize] = sprite as i32;
        }
        let model_quads = crate::models::gallery(root, &mut sprites)?;
        Ok(Self {
            registry,
            properties,
            material_states,
            layers,
            sprite_size: sprites.size,
            texture_layers: u32::try_from(sprites.layers.len())?,
            pixels: sprites.pixels,
            model_quads,
            animations: sprites.animations,
        })
    }
}
