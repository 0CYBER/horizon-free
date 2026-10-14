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
    /// Leaked once per process: shared by world generation threads.
    pub registry: &'static BlockRegistry,
    pub properties: Properties,
    pub material_states: [BlockStateId; 6],
    pub sprite_size: u32,
    pub sprite_minimum_size: u32,
    pub pixels: Vec<u8>,
    pub texture_layers: u32,
    pub model_catalog: crate::models::StateModels,
    pub model_placements: Vec<(horizon_core::position::BlockPos, BlockStateId)>,
    pub model_gallery: Vec<Value>,
    pub animations: Vec<crate::sprites::SpriteAnimation>,
    pub sprite_layers: std::collections::BTreeMap<String, u32>,
    pub translucent_layers: Vec<bool>,
    /// Block names in `#minecraft:blocks_fluid_flow` (fluid flow vectors).
    pub blocks_fluid_flow: BTreeSet<String>,
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
        let blocks_fluid_flow = built
            .tags
            .get("minecraft:blocks_fluid_flow")
            .ok_or_else(|| invalid("missing fluid flow tag"))?
            .iter()
            .cloned()
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
        let mut material_states = [BlockStateId(0); 6];
        for (layer, name) in MATERIALS.iter().enumerate() {
            material_states[layer] = registry
                .default_state(&format!("minecraft:{name}"))
                .ok_or_else(|| invalid("missing test-scene material"))?;
        }
        let registry: &'static BlockRegistry = Box::leak(Box::new(registry));
        Self::with_models(
            root,
            registry,
            properties,
            material_states,
            blocks_fluid_flow,
        )
    }
    /// The same blocks with the models and sprites of another resource root
    /// (resource packs changed): the registry and block facts come from the
    /// game data, which resource packs do not touch.
    pub fn reload(&self, root: &Path) -> Result<Self> {
        Self::with_models(
            root,
            self.registry,
            self.properties.clone(),
            self.material_states,
            self.blocks_fluid_flow.clone(),
        )
    }
    fn with_models(
        root: &Path,
        registry: &'static BlockRegistry,
        properties: Properties,
        material_states: [BlockStateId; 6],
        blocks_fluid_flow: BTreeSet<String>,
    ) -> Result<Self> {
        let mut sprites = crate::sprites::Sprites::default();
        for name in MATERIALS {
            sprites.insert(root, &format!("minecraft:block/{name}"))?;
        }
        let models = crate::models::prepare(root, registry, &mut sprites)?;
        // Fluids are drawn by the fluid renderer, not block models; breaking
        // progress uses the destroy-stage overlays.
        for name in [
            "water_still",
            "water_flow",
            "water_overlay",
            "lava_still",
            "lava_flow",
        ] {
            sprites.insert(root, &format!("minecraft:block/{name}"))?;
        }
        for stage in 0..10 {
            sprites.insert(root, &format!("minecraft:block/destroy_stage_{stage}"))?;
        }
        sprites.finish();
        Ok(Self {
            registry,
            properties,
            material_states,
            sprite_size: sprites.size,
            sprite_minimum_size: sprites.minimum_size,
            texture_layers: u32::try_from(sprites.layers.len())?,
            pixels: sprites.pixels,
            model_catalog: models.catalog,
            model_placements: models.placements,
            model_gallery: models.states,
            animations: sprites.animations,
            sprite_layers: sprites.layers,
            translucent_layers: sprites.translucent,
            blocks_fluid_flow,
        })
    }
}
