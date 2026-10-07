//! Vanilla tree, fallen-tree and ore features over a layered synthetic world.
use horizon_core::{
    geometry::{Direction, VoxelShape},
    random::{LegacyRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::{
    block_placement::BlockPlacementBindings,
    block_properties::BlockPropertiesRegistry,
    block_support::{BlockSupportRegistry, SupportType},
    collision::CachedCollisionRegistry,
    dynamic_support::DynamicSupportWorld,
    fluid::{FluidKind, FluidState, FluidStateTable},
    heightmap::HeightmapType,
    nbt::{self, Limits, Tag},
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        block_feature::FeatureBlockBindings,
        feature::{FeatureBlocks, FeatureWorld},
        feature_catalog::FeatureCatalog,
        placement::{PlacementContext, PlacementQueryError},
        state_provider::{StateProviderBindings, StateProviderCatalog},
        tags::TagDefinitions,
    },
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    io::{self, BufRead, Write},
    path::Path,
    sync::Arc,
};

const MIN_Y: i32 = -64;
const HEIGHT: i32 = 384;

struct Terrain {
    layers: Vec<[i32; 2]>,
    hole: [i32; 2],
    obstacle: [i32; 3],
    overrides: HashMap<[i32; 3], u32>,
    air: u32,
}
impl Terrain {
    fn new(f: &Value, air: u32) -> Self {
        let n = |key: &str| f[key].as_i64().unwrap() as i32;
        Self {
            layers: f["layers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| [l[0].as_i64().unwrap() as i32, l[1].as_i64().unwrap() as i32])
                .collect(),
            hole: [n("hole_mod"), n("hole_state")],
            obstacle: [n("obstacle_mod"), n("obstacle_state"), n("obstacle_min")],
            overrides: f["states"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| {
                    (
                        std::array::from_fn(|i| v[i].as_i64().unwrap() as i32),
                        v[3].as_u64().unwrap() as u32,
                    )
                })
                .collect(),
            air,
        }
    }
    fn hash(x: i32, y: i32, z: i32) -> i32 {
        let mut h = x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663) ^ z.wrapping_mul(83492791);
        h ^= ((h as u32) >> 13) as i32;
        h = h.wrapping_mul(0x5bd1e995);
        h ^= ((h as u32) >> 15) as i32;
        h & 0x7fff_ffff
    }
    fn at(&self, [x, y, z]: [i32; 3]) -> BlockStateId {
        if let Some(state) = self.overrides.get(&[x, y, z]) {
            return BlockStateId(*state);
        }
        let h = Self::hash(x, y, z);
        if let Some(layer) = self.layers.iter().find(|l| y <= l[0]) {
            if self.hole[0] > 0 && h % self.hole[0] == 0 {
                return BlockStateId(self.hole[1] as u32);
            }
            return BlockStateId(layer[1] as u32);
        }
        if self.obstacle[0] > 0 && y >= self.obstacle[2] && h % self.obstacle[0] == 0 {
            return BlockStateId(self.obstacle[1] as u32);
        }
        BlockStateId(self.air)
    }
}

struct World<'a, 'r, W> {
    out: &'a mut W,
    terrain: Terrain,
    overlay: BTreeMap<[i32; 3], BlockStateId>,
    void_air: BlockStateId,
    height: i32,
    fluids: &'a FluidStateTable,
    tags: &'a BTreeMap<String, BTreeSet<BlockStateId>>,
    support: &'a BlockSupportRegistry<'r>,
}
impl<W: Write> World<'_, '_, W> {
    fn get(&self, p: [i32; 3]) -> BlockStateId {
        if p[1] < MIN_Y || p[1] >= MIN_Y + HEIGHT {
            return self.void_air;
        }
        self.overlay
            .get(&p)
            .copied()
            .unwrap_or_else(|| self.terrain.at(p))
    }
}
fn query(error: impl std::fmt::Display) -> PlacementQueryError {
    PlacementQueryError::Failed(error.to_string())
}
impl<W: Write> FeatureWorld for World<'_, '_, W> {
    type Error = io::Error;
    fn block(&mut self, p: [i32; 3]) -> io::Result<BlockStateId> {
        Ok(self.get(p))
    }
    fn set_block(&mut self, p: [i32; 3], state: BlockStateId, flags: i32) -> io::Result<bool> {
        writeln!(self.out, "set {} {} {} {} {flags}", p[0], p[1], p[2], state.0)?;
        if p[1] < MIN_Y || p[1] >= MIN_Y + HEIGHT {
            return Ok(false);
        }
        self.overlay.insert(p, state);
        Ok(true)
    }
}
impl<W: Write> DynamicSupportWorld for World<'_, '_, W> {
    type Error = io::Error;
    fn shulker_is_closed(&mut self, _: [i32; 3]) -> io::Result<Option<bool>> {
        Ok(None)
    }
    fn moving_piston_shape(&mut self, _: [i32; 3]) -> io::Result<Option<VoxelShape>> {
        Ok(None)
    }
}
impl<W: Write> PlacementContext for World<'_, '_, W> {
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(MIN_Y, HEIGHT, 63)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(MIN_Y)
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(MIN_Y + HEIGHT - 1)
    }
    fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        Ok(self.height)
    }
    fn block_state(&mut self, p: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        Ok(self.get(p))
    }
    fn fluid_state(&mut self, p: [i32; 3]) -> Result<FluidState, PlacementQueryError> {
        self.fluids
            .get(self.get(p))
            .ok_or_else(|| query("unknown fluid"))
    }
    fn block_matches_tag(&mut self, p: [i32; 3], tag: &str) -> Result<bool, PlacementQueryError> {
        let state = self.get(p);
        Ok(self.tags.get(tag).is_some_and(|s| s.contains(&state)))
    }
    fn state_face_sturdy(
        &mut self,
        state: BlockStateId,
        p: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let support = self.support;
        support
            .is_face_sturdy(self, state, p, direction, SupportType::Full)
            .map_err(query)
    }
    fn face_sturdy(&mut self, p: [i32; 3], direction: Direction) -> Result<bool, PlacementQueryError> {
        let state = self.get(p);
        self.state_face_sturdy(state, p, direction)
    }
    fn mark_postprocessing(&mut self, p: [i32; 3]) -> Result<(), PlacementQueryError> {
        writeln!(self.out, "post {} {} {}", p[0], p[1], p[2]).map_err(query)
    }
    fn set_section_block(&mut self, p: [i32; 3], state: BlockStateId) -> Result<(), PlacementQueryError> {
        self.overlay.insert(p, state);
        Ok(())
    }
    fn store_beehive_occupants(&mut self, p: [i32; 3], ticks: &[i32]) -> Result<(), PlacementQueryError> {
        for t in ticks {
            writeln!(self.out, "bee {} {} {} {t}", p[0], p[1], p[2]).map_err(query)?;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn fixture<R: RandomSource>(
    f: &Value,
    mut random: R,
    catalog: &FeatureCatalog,
    features: &BTreeMap<String, String>,
    bindings: &FeatureBlockBindings<'_>,
    blocks: FeatureBlocks,
    world: &mut World<'_, '_, impl Write>,
) -> io::Result<()> {
    let name = f["feature"].as_str().unwrap();
    writeln!(world.out, "fixture {name}")?;
    let feature = match catalog.feature_with_blocks(&features[name], bindings) {
        Ok(feature) => feature,
        Err(error) => {
            eprintln!("decode {name}: {error}");
            return writeln!(world.out, "decode error");
        }
    };
    let origin = std::array::from_fn(|i| f["origin"][i].as_i64().unwrap() as i32);
    match feature.place(world, &mut random, origin, blocks) {
        Ok(result) => writeln!(world.out, "place {result}")?,
        Err(error) => writeln!(world.out, "place error {error}")?,
    }
    writeln!(world.out, "next {}", random.next_long())?;
    let changes: Vec<_> = world
        .overlay
        .iter()
        .filter(|(p, state)| world.terrain.at(**p) != **state)
        .map(|(p, s)| (*p, *s))
        .collect();
    for (p, state) in changes {
        writeln!(world.out, "final {} {} {} {}", p[0], p[1], p[2], state.0)?;
    }
    writeln!(world.out, "end")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let read = |name: &str| -> Result<Tag, Box<dyn std::error::Error>> {
        Ok(nbt::read_named(&fs::read(dir.join(name))?, Limits::default())?.0.tag)
    };
    let registry = BlockRegistry::from_nbt(&read("block-registry.nbt")?)?;
    let collisions = CachedCollisionRegistry::from_nbt(&read("cached-collision.nbt")?, &registry)?;
    let properties = BlockPropertiesRegistry::from_nbt(&read("block-properties.nbt")?, &collisions)?;
    let support = BlockSupportRegistry::from_nbt(&read("cached-support.nbt")?, &registry)?;
    let resources: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("tags.json"))?)?;
    let mut tags = TagDefinitions::default();
    for (name, text) in resources {
        tags.apply_json(&name, "vanilla", &text)?;
    }
    let tags = tags.build(|name, _| registry.default_state(name).map(|_| name.to_owned()))?;
    let fluid_resources: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("fluid-tags.json"))?)?;
    let mut fluid_tags = TagDefinitions::default();
    for (name, text) in fluid_resources {
        fluid_tags.apply_json(&name, "vanilla", &text)?;
    }
    let fluid_tags =
        fluid_tags.build(|name, _| FluidKind::from_name(name).map(|_| name.to_owned()))?;
    let facts: Value = serde_json::from_str(&fs::read_to_string(dir.join("facts.json"))?)?;
    let integers: BTreeSet<(String, String)> = facts["integers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v[0].as_str().unwrap().to_owned(), v[1].as_str().unwrap().to_owned()))
        .collect();
    let bundle: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("vanilla-features.json"))?)?;
    let providers: BTreeMap<String, String> = serde_json::from_value(bundle["providers"].clone())?;
    let features: BTreeMap<String, String> = serde_json::from_value(bundle["features"].clone())?;
    let definitions: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("placement-definitions.json"))?)?;
    let bindings = FeatureBlockBindings::with_tags(&registry, integers.clone(), &tags.tags)?
        .with_block_properties(&properties)?
        .with_fluid_tags(&fluid_tags.tags)?
        .with_state_providers(
            StateProviderBindings::new(&registry, integers, &tags.tags)?
                .with_block_properties(&properties)?
                .with_fluid_tags(&fluid_tags.tags)?,
            StateProviderCatalog::from_resources(&providers)?,
            BlockPlacementBindings::new(
                &registry,
                &properties,
                &serde_json::from_value(definitions["double_plants"].clone())?,
                &serde_json::from_value(definitions["mossy_carpets"].clone())?,
            )?,
        )?;
    let catalog = FeatureCatalog::from_resources(&features, &BTreeMap::new(), &TagDefinitions::default())?;
    let blocks = FeatureBlocks::from_registry(&registry)?;
    let fluids = FluidStateTable::new(&registry)?;
    let mut by_name = BTreeMap::<String, Vec<BlockStateId>>::new();
    for id in 0..registry.len() {
        let id = BlockStateId(id as u32);
        by_name
            .entry(registry.state(id).unwrap().name.clone())
            .or_default()
            .push(id);
    }
    let tag_states: BTreeMap<String, BTreeSet<BlockStateId>> = tags
        .tags
        .iter()
        .map(|(name, entries)| {
            (
                name.clone(),
                entries
                    .iter()
                    .flat_map(|e| by_name.get(e).into_iter().flatten().copied())
                    .collect(),
            )
        })
        .collect();
    let support = Arc::new(support);
    let air = registry.default_state("minecraft:air").unwrap().0;
    let void_air = registry.default_state("minecraft:void_air").unwrap();
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in io::BufReader::new(fs::File::open(dir.join("inputs.txt"))?).lines() {
        let f: Value = serde_json::from_str(&line?)?;
        let mut world = World {
            out: &mut out,
            terrain: Terrain::new(&f, air),
            overlay: BTreeMap::new(),
            void_air,
            height: f["height"].as_i64().unwrap() as i32,
            fluids: &fluids,
            tags: &tag_states,
            support: &support,
        };
        let seed = f["seed"].as_i64().unwrap();
        if f["legacy"].as_bool().unwrap() {
            fixture(&f, LegacyRandom::new(seed), &catalog, &features, &bindings, blocks, &mut world)?;
        } else {
            fixture(&f, XoroshiroRandom::new(seed), &catalog, &features, &bindings, blocks, &mut world)?;
        }
    }
    out.flush()?;
    Ok(())
}
