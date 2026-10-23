use crate::support::common::tag;
use horizon_core::random::{LegacyRandom, RandomSource, WorldgenRandom, XoroshiroRandom};
use horizon_world::{
    DATA_VERSION,
    block_collision::{BlockCollisionRegistry, DynamicCollisionWorld, ShulkerCollisionState},
    block_support::{BlockSupportRegistry, SupportType},
    block_survival::{BlockSurvivalRegistry, SurvivalDefinitions, SurvivalRule, SurvivalWorld},
    fluid::{FluidKind, FluidStateTable},
    heightmap::HeightmapType,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        biome::BiomeId,
        biome_predicate::BiomePredicateBindings,
        feature::FeatureWorld,
        placement::{PlacementContext, PlacementQueryError},
        state_provider::{StateProviderBindings, StateProviderCatalog},
        tags::TagDefinitions,
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
struct World<'a, W> {
    support: &'a BlockSupportRegistry<'a>,
    survival: &'a BlockSurvivalRegistry<'a>,
    out: &'a mut W,
    fallback: BlockStateId,
    biome: BiomeId,
    unobstructed: bool,
    brightness: i32,
    tags: &'a BTreeMap<String, BTreeSet<BlockStateId>>,
    calls: i32,
    fail: i32,
    queries: i32,
    query_fail: i32,
    bounds: [i32; 3],
}
impl<W: Write> FeatureWorld for World<'_, W> {
    type Error = ();
    fn block(&mut self, p: [i32; 3]) -> Result<BlockStateId, ()> {
        writeln!(self.out, "read {} {} {}", p[0], p[1], p[2]).map_err(|_| ())?;
        let call = self.calls;
        self.calls += 1;
        if call == self.fail {
            Err(())
        } else {
            Ok(self.fallback)
        }
    }
    fn set_block(&mut self, _: [i32; 3], _: BlockStateId, _: i32) -> Result<bool, ()> {
        panic!("providers must not write blocks")
    }
}
impl<W: Write> World<'_, W> {
    fn query(&mut self, name: &str, value: i32) -> Result<i32, PlacementQueryError> {
        writeln!(self.out, "{name}").map_err(|_| PlacementQueryError::Unavailable)?;
        let call = self.queries;
        self.queries += 1;
        if call == self.query_fail {
            Err(PlacementQueryError::Unavailable)
        } else {
            Ok(value)
        }
    }
}
impl<W: Write> horizon_world::dynamic_support::DynamicSupportWorld for World<'_, W> {
    type Error = PlacementQueryError;
    fn shulker_is_closed(&mut self, p: [i32; 3]) -> Result<Option<bool>, Self::Error> {
        self.query(&format!("entity {} {} {}", p[0], p[1], p[2]), 0)?;
        Ok(None)
    }
    fn moving_piston_shape(
        &mut self,
        p: [i32; 3],
    ) -> Result<Option<horizon_core::geometry::VoxelShape>, Self::Error> {
        self.query(&format!("entity {} {} {}", p[0], p[1], p[2]), 0)?;
        Ok(None)
    }
}
impl<W: Write> DynamicCollisionWorld for World<'_, W> {
    fn shulker_collision_state(
        &mut self,
        p: [i32; 3],
    ) -> Result<Option<ShulkerCollisionState>, Self::Error> {
        self.query(&format!("entity {} {} {}", p[0], p[1], p[2]), 0)?;
        Ok(None)
    }
}
impl<W: Write> SurvivalWorld for World<'_, W> {
    fn fluid_state(
        &mut self,
        p: [i32; 3],
    ) -> Result<horizon_world::fluid::FluidState, Self::Error> {
        self.query(&format!("fluid {} {} {}", p[0], p[1], p[2]), 0)?;
        horizon_world::fluid::FluidState::from_block(
            self.support.registry().state(self.fallback).unwrap(),
        )
        .map_err(|_| PlacementQueryError::Unavailable)
    }
    fn raw_brightness(&mut self, p: [i32; 3], sky_darken: i32) -> Result<i32, Self::Error> {
        self.query(
            &format!("light {} {} {} {sky_darken}", p[0], p[1], p[2]),
            self.brightness,
        )
    }
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, Self::Error> {
        self.block(position)
            .map_err(|_| PlacementQueryError::Unavailable)
    }
}
impl<W: Write> PlacementContext for World<'_, W> {
    fn block_survives(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let survival = self.survival;
        survival
            .can_survive(self, state, position)
            .map_err(|_| PlacementQueryError::Unavailable)
    }
    fn face_sturdy(
        &mut self,
        p: [i32; 3],
        direction: horizon_core::geometry::Direction,
    ) -> Result<bool, PlacementQueryError> {
        let state = self
            .block(p)
            .map_err(|_| PlacementQueryError::Unavailable)?;
        let support = self.support;
        support
            .is_face_sturdy(self, state, p, direction, SupportType::Full)
            .map_err(|_| PlacementQueryError::Unavailable)
    }

    fn unobstructed_block(&mut self, p: [i32; 3]) -> Result<bool, PlacementQueryError> {
        let min = p.map(f64::from);
        let max = min.map(|v| v + 1.0);
        let label = format!(
            "clear {:016x} {:016x} {:016x} {:016x} {:016x} {:016x}",
            min[0].to_bits(),
            min[1].to_bits(),
            min[2].to_bits(),
            max[0].to_bits(),
            max[1].to_bits(),
            max[2].to_bits()
        );
        self.query(&label, i32::from(self.unobstructed))
            .map(|value| value != 0)
    }
    fn biome_identity(&mut self, p: [i32; 3]) -> Result<BiomeId, PlacementQueryError> {
        self.query(
            &format!("biome {} {} {}", p[0], p[1], p[2]),
            self.biome.0 as i32,
        )
        .map(|id| BiomeId(id as u32))
    }
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext {
            min_y: self.bounds[0],
            height: self.bounds[1],
            sea_level: self.bounds[2],
        }
    }
    fn predicate_generation(&mut self) -> Result<WorldGenerationContext, PlacementQueryError> {
        let min_y = self.min_y()?;
        let height = self.query("height", self.bounds[1])?;
        let sea_level = self.query("sea", self.bounds[2])?;
        Ok(WorldGenerationContext {
            min_y,
            height,
            sea_level,
        })
    }
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        self.block(position)
            .map_err(|_| PlacementQueryError::Unavailable)
    }
    fn block_matches_tag(
        &mut self,
        position: [i32; 3],
        tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        let state = self
            .block(position)
            .map_err(|_| PlacementQueryError::Unavailable)?;
        Ok(self
            .tags
            .get(tag)
            .is_some_and(|states| states.contains(&state)))
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.query("min", self.bounds[0])
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self
            .min_y()?
            .wrapping_add(self.query("height", self.bounds[1])?)
            .wrapping_sub(1))
    }
    fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
}
fn fixture(
    f: &Value,
    bindings: &StateProviderBindings<'_>,
    catalog: &StateProviderCatalog,
    tags: &BTreeMap<String, BTreeSet<BlockStateId>>,
    support: (&BlockSupportRegistry<'_>, &BlockSurvivalRegistry<'_>),
    mut random: impl RandomSource,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = f["definition"].as_str().unwrap();
    let decoded = if f["holder"].as_bool().unwrap_or(false) {
        catalog.provider(json, bindings)
    } else if f["typed"].as_bool().unwrap() {
        catalog.typed_provider(json, bindings)
    } else {
        catalog.direct_provider(json, bindings)
    };
    match decoded {
        Err(_) => writeln!(out, "decode error")?,
        Ok(provider) => {
            writeln!(out, "decode ok")?;
            let mut world = World {
                support: support.0,
                survival: support.1,
                out,
                tags,
                fallback: BlockStateId(0),
                biome: BiomeId(0),
                unobstructed: f["unobstructed"].as_bool().unwrap_or(true),
                brightness: f["brightness"].as_i64().unwrap_or(15) as i32,
                calls: 0,
                queries: 0,
                query_fail: f["query_fail"].as_i64().unwrap_or(-1) as i32,
                bounds: f
                    .get("bounds")
                    .map(|v| std::array::from_fn(|i| v[i].as_i64().unwrap() as i32))
                    .unwrap_or([-64, 384, 63]),
                fail: f["fail"].as_i64().unwrap() as i32,
            };
            let mut position_index = 0usize;
            let mut sample_index = 0usize;
            for repeat in 0..f["repeats"].as_i64().unwrap() as i32 {
                for state in f["states"].as_array().unwrap() {
                    world.fallback = BlockStateId(state.as_u64().unwrap() as u32);
                    world.biome = f
                        .get("biomes")
                        .map(|values| {
                            let values = values.as_array().unwrap();
                            BiomeId(values[sample_index % values.len()].as_u64().unwrap() as u32)
                        })
                        .unwrap_or(BiomeId(0));
                    sample_index += 1;
                    let p = if let Some(positions) = f.get("positions") {
                        let positions = positions.as_array().unwrap();
                        let p = &positions[position_index % positions.len()];
                        position_index += 1;
                        std::array::from_fn(|i| p[i].as_i64().unwrap() as i32)
                    } else {
                        [17, repeat - 64, -33]
                    };
                    for optional in [false, true] {
                        let result = if optional {
                            provider.get_optional_state(&mut world, &mut random, p)
                        } else {
                            provider.get_state(&mut world, &mut random, p).map(Some)
                        };
                        match result {
                            Ok(Some(state)) => writeln!(world.out, "state {}", state.0)?,
                            Ok(None) => writeln!(world.out, "state none")?,
                            Err(_) => writeln!(world.out, "state error")?,
                        }
                        writeln!(world.out, "next {}", random.next_long())?;
                    }
                }
            }
        }
    }
    writeln!(out, "tail {}", random.next_long())?;
    Ok(())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let facts: Value = serde_json::from_str(&fs::read_to_string(dir.join("facts-server.json"))?)?;
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("states", tag(&facts["states"])),
        ("defaults", tag(&facts["defaults"])),
    ]))?;
    let integers = facts["integers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v[0].as_str().unwrap().to_owned(),
                v[1].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let resources: Value = serde_json::from_str(&fs::read_to_string(dir.join("tags.json"))?)?;
    let mut definitions = TagDefinitions::default();
    for (name, text) in resources.as_object().unwrap() {
        definitions.apply_json(name, "vanilla", text.as_str().unwrap())?;
    }
    let built =
        definitions.build(|name, _| registry.default_state(name).map(|_| name.to_owned()))?;
    assert!(built.missing.is_empty());
    let tag_states: BTreeMap<String, BTreeSet<BlockStateId>> = built
        .tags
        .iter()
        .map(|(tag, names)| {
            let states = (0..registry.len())
                .map(|i| BlockStateId(i as u32))
                .filter(|id| names.contains(&registry.state(*id).unwrap().name))
                .collect();
            (tag.clone(), states)
        })
        .collect();
    let fluid_resources: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("fluid-tags.json"))?)?;
    let mut fluid_tags = TagDefinitions::default();
    for (name, text) in fluid_resources {
        fluid_tags.apply_json(&name, "vanilla", &text)?;
    }
    let fluid_tags =
        fluid_tags.build(|name, _| FluidKind::from_name(name).map(|_| name.to_owned()))?;
    assert!(fluid_tags.missing.is_empty());
    let biome_resources: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("biomes.json"))?)?;
    let biome_names: BTreeMap<String, BiomeId> = biome_resources
        .keys()
        .enumerate()
        .map(|(i, name)| (name.clone(), BiomeId(i as u32)))
        .collect();
    let biome_tags: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("biome_tags.json"))?)?;
    let mut definitions = TagDefinitions::default();
    for (name, text) in biome_tags {
        definitions.apply_json(&name, "vanilla", &text)?;
    }
    let biome_tags =
        definitions.build(|name, _| biome_names.contains_key(name).then(|| name.to_owned()))?;
    assert!(biome_tags.missing.is_empty());
    let biomes = Arc::new(BiomePredicateBindings::new(&biome_names, &biome_tags.tags)?);
    let collision_tag = horizon_world::nbt::read_named(
        &fs::read(dir.join("cached-collision.nbt"))?,
        horizon_world::nbt::Limits::default(),
    )?
    .0
    .tag;
    let collision =
        horizon_world::collision::CachedCollisionRegistry::from_nbt(&collision_tag, &registry)?;
    let properties_tag = horizon_world::nbt::read_named(
        &fs::read(dir.join("block-properties.nbt"))?,
        horizon_world::nbt::Limits::default(),
    )?
    .0
    .tag;
    let properties = horizon_world::block_properties::BlockPropertiesRegistry::from_nbt(
        &properties_tag,
        &collision,
    )?;
    let support_tag = horizon_world::nbt::read_named(
        &fs::read(dir.join("cached-support.nbt"))?,
        horizon_world::nbt::Limits::default(),
    )?
    .0
    .tag;
    let support = Arc::new(BlockSupportRegistry::from_nbt(&support_tag, &registry)?);
    let survival_definitions: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("survival-rules.json"))?)?;
    let survival_definitions = survival_definitions
        .into_iter()
        .map(|(name, rule)| {
            use SurvivalRule::*;
            let rule = match rule.as_str() {
                value if value.starts_with("TagBelow:") => TagBelow(value[9..].to_owned()),
                "CactusFlower" => CactusFlower,
                "Seagrass" => Seagrass,
                "BubbleColumn" => BubbleColumn,
                "RedstoneWire" => RedstoneWire,
                "DoublePlant" => DoublePlant,
                "Door" => Door,
                "BigDripleaf" => BigDripleaf,
                "BigDripleafStem" => BigDripleafStem,
                "Farmland" => Farmland,
                "Cocoa" => Cocoa,
                "Crop" => Crop,
                "GrowingPlant" => GrowingPlant,
                "MangrovePropagule" => MangrovePropagule,
                "Speleothem" => Speleothem,
                "Scaffolding" => Scaffolding,
                "PistonHead" => PistonHead,
                "ChorusPlant" => ChorusPlant,
                "ChorusFlower" => ChorusFlower,
                "PitcherCrop" => PitcherCrop,
                "Multiface" => Multiface,
                "Vine" => Vine,
                "HangingMoss" => HangingMoss,
                "SnowLayer" => SnowLayer,
                "SeaPickle" => SeaPickle,
                "Fire" => Fire,
                "Cactus" => Cactus,
                "Path" => Path,
                "Mushroom" => Mushroom,
                "Bell" => Bell,
                "SugarCane" => SugarCane,
                "LilyPad" => LilyPad,
                "Frogspawn" => Frogspawn,
                "SporeBlossom" => SporeBlossom,
                "SmallDripleaf" => SmallDripleaf,
                "TallSeagrass" => TallSeagrass,
                "Always" => Always,
                "SolidBelow" => SolidBelow,
                "SolidBehind" => SolidBehind,
                "NonAirBelow" => NonAirBelow,
                "MossCarpet" => MossCarpet,
                "CenterBelow" => CenterBelow,
                "Lantern" => Lantern,
                "FullAbove" => FullAbove,
                "FullBehind" => FullBehind,
                "RigidOrCenterBelow" => RigidOrCenterBelow,
                "RigidBelow" => RigidBelow,
                "FullBelow" => FullBelow,
                "CenterAbove" => CenterAbove,
                "FaceAttached" => FaceAttached,
                "SupportsBamboo" => SupportsBamboo,
                _ => panic!("unknown survival rule {rule}"),
            };
            (name, rule)
        })
        .collect();
    let collision = Arc::new(BlockCollisionRegistry::new(Arc::new(collision))?);
    let extra: Value = serde_json::from_str(&fs::read_to_string(
        dir.join("survival-extra-definitions.json"),
    )?)?;
    let survival_definitions = SurvivalDefinitions {
        rules: survival_definitions,
        ignition_odds: serde_json::from_value(extra["ignition_odds"].clone())?,
        fence_gates: serde_json::from_value(extra["fence_gates"].clone())?,
    };
    let survival = BlockSurvivalRegistry::new(
        &registry,
        &properties,
        Arc::clone(&support),
        collision,
        &survival_definitions,
        &built.tags,
        &fluid_tags.tags,
    )?;
    let bindings = StateProviderBindings::new(&registry, integers, &built.tags)?
        .with_block_properties(&properties)?
        .with_fluid_tags(&fluid_tags.tags)?
        .with_biomes(biomes);
    let resources: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("catalog.json"))?)?;
    let catalog = StateProviderCatalog::from_resources(&resources)?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    let fluids = FluidStateTable::new(&registry)?;
    for i in 0..registry.len() {
        let fluid = fluids.get(BlockStateId(i as u32)).unwrap();
        writeln!(
            out,
            "fluid {i} {} {} {} {} {}",
            fluid.kind().name(),
            fluid.amount(),
            fluid.falling(),
            fluid.is_source(),
            fluid.own_height().to_bits()
        )?;
    }
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let seed = f["seed"].as_i64().unwrap();
        match (
            f["legacy"].as_bool().unwrap(),
            f["worldgen"].as_bool().unwrap(),
        ) {
            (true, false) => fixture(
                &f,
                &bindings,
                &catalog,
                &tag_states,
                (&support, &survival),
                LegacyRandom::new(seed),
                &mut out,
            )?,
            (false, false) => fixture(
                &f,
                &bindings,
                &catalog,
                &tag_states,
                (&support, &survival),
                XoroshiroRandom::new(seed),
                &mut out,
            )?,
            (true, true) => fixture(
                &f,
                &bindings,
                &catalog,
                &tag_states,
                (&support, &survival),
                WorldgenRandom::new(LegacyRandom::new(seed)),
                &mut out,
            )?,
            (false, true) => fixture(
                &f,
                &bindings,
                &catalog,
                &tag_states,
                (&support, &survival),
                WorldgenRandom::new(XoroshiroRandom::new(seed)),
                &mut out,
            )?,
        }
    }
    Ok(())
}
