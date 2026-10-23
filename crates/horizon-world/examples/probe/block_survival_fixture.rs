use horizon_core::geometry::VoxelShape;
use horizon_world::{
    block_collision::{BlockCollisionRegistry, DynamicCollisionWorld, ShulkerCollisionState},
    block_properties::BlockPropertiesRegistry,
    block_support::BlockSupportRegistry,
    block_survival::{BlockSurvivalRegistry, SurvivalDefinitions, SurvivalRule, SurvivalWorld},
    collision::CachedCollisionRegistry,
    dynamic_support::DynamicSupportWorld,
    fluid::{FluidKind, FluidState, FluidStateTable},
    nbt::{self, Limits, Tag},
    registry::{BlockRegistry, BlockStateId},
    worldgen::tags::TagDefinitions,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead, Write},
    path::Path,
    sync::Arc,
};
struct World<'a, W> {
    out: &'a mut W,
    neighbors: Vec<BlockStateId>,
    fluids: &'a FluidStateTable,
    fluid_states: Vec<BlockStateId>,
    fluid_calls: i32,
    fluid_fail: i32,
    entity: &'a str,
    shulker: ShulkerCollisionState,
    reads: i32,
    lights: i32,
    light_fail: i32,
    brightness: i32,
    entities: i32,
    fail: i32,
    entity_fail: i32,
}
impl<W: Write> World<'_, W> {
    fn entity(&mut self, p: [i32; 3]) -> io::Result<()> {
        writeln!(self.out, "entity {} {} {}", p[0], p[1], p[2])?;
        let old = self.entities;
        self.entities += 1;
        if old == self.entity_fail {
            return Err(io::Error::other("injected entity query failure"));
        }
        Ok(())
    }
}
impl<W: Write> DynamicSupportWorld for World<'_, W> {
    type Error = io::Error;
    fn shulker_is_closed(&mut self, p: [i32; 3]) -> io::Result<Option<bool>> {
        self.entity(p)?;
        Ok(match self.entity {
            "none" => None,
            "closed" => Some(true),
            "opened" => Some(false),
            _ => unreachable!(),
        })
    }
    fn moving_piston_shape(&mut self, p: [i32; 3]) -> io::Result<Option<VoxelShape>> {
        self.entity(p)?;
        Ok(None)
    }
}
impl<W: Write> DynamicCollisionWorld for World<'_, W> {
    fn shulker_collision_state(
        &mut self,
        p: [i32; 3],
    ) -> io::Result<Option<ShulkerCollisionState>> {
        self.entity(p)?;
        Ok((self.entity != "none").then_some(self.shulker))
    }
}
impl<W: Write> SurvivalWorld for World<'_, W> {
    fn fluid_state(&mut self, p: [i32; 3]) -> io::Result<FluidState> {
        writeln!(self.out, "fluid {} {} {}", p[0], p[1], p[2])?;
        let old = self.fluid_calls;
        self.fluid_calls += 1;
        if old == self.fluid_fail {
            return Err(io::Error::other("injected fluid query failure"));
        }
        self.fluids
            .get(self.fluid_states[old as usize % self.fluid_states.len()])
            .ok_or_else(|| io::Error::other("unknown fixture fluid state"))
    }
    fn raw_brightness(&mut self, p: [i32; 3], sky_darken: i32) -> io::Result<i32> {
        writeln!(self.out, "light {} {} {} {sky_darken}", p[0], p[1], p[2])?;
        let old = self.lights;
        self.lights += 1;
        if old == self.light_fail {
            return Err(io::Error::other("injected light query failure"));
        }
        Ok(self.brightness)
    }
    fn block_state(&mut self, p: [i32; 3]) -> io::Result<BlockStateId> {
        writeln!(self.out, "read {} {} {}", p[0], p[1], p[2])?;
        let old = self.reads;
        self.reads += 1;
        if old == self.fail {
            return Err(io::Error::other("injected block query failure"));
        }
        Ok(self.neighbors[old as usize % self.neighbors.len()])
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let read = |name: &str| -> Result<Tag, Box<dyn std::error::Error>> {
        Ok(
            nbt::read_named(&fs::read(dir.join(name))?, Limits::default())?
                .0
                .tag,
        )
    };
    let registry = BlockRegistry::from_nbt(&read("block-registry.nbt")?)?;
    let collisions = CachedCollisionRegistry::from_nbt(&read("cached-collision.nbt")?, &registry)?;
    let properties =
        BlockPropertiesRegistry::from_nbt(&read("block-properties.nbt")?, &collisions)?;
    let support = Arc::new(BlockSupportRegistry::from_nbt(
        &read("cached-support.nbt")?,
        &registry,
    )?);
    let resources: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("tags.json"))?)?;
    let mut tags = TagDefinitions::default();
    for (name, text) in resources {
        tags.apply_json(&name, "vanilla", &text)?;
    }
    let tags = tags.build(|name, _| registry.default_state(name).map(|_| name.to_owned()))?;
    assert!(tags.missing.is_empty());
    let definitions: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("survival-rules.json"))?)?;
    let definitions = definitions
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
                _ => panic!("unknown rule {rule}"),
            };
            (name, rule)
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
    let fluids = FluidStateTable::new(&registry)?;
    let collision = Arc::new(BlockCollisionRegistry::new(Arc::new(collisions))?);
    let extra: Value = serde_json::from_str(&fs::read_to_string(
        dir.join("survival-extra-definitions.json"),
    )?)?;
    let definitions = SurvivalDefinitions {
        rules: definitions,
        ignition_odds: serde_json::from_value(extra["ignition_odds"].clone())?,
        fence_gates: serde_json::from_value(extra["fence_gates"].clone())?,
    };
    let survival = BlockSurvivalRegistry::new(
        &registry,
        &properties,
        support,
        collision,
        &definitions,
        &tags.tags,
        &fluid_tags.tags,
    )?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in io::BufReader::new(fs::File::open(dir.join("inputs.txt"))?).lines() {
        let f: Value = serde_json::from_str(&line?)?;
        let mut world = World {
            out: &mut out,
            fluids: &fluids,
            fluid_states: f
                .get("fluid_states")
                .unwrap_or(&f["neighbors"])
                .as_array()
                .unwrap()
                .iter()
                .map(|v| BlockStateId(v.as_u64().unwrap() as u32))
                .collect(),
            fluid_calls: 0,
            fluid_fail: f["fluid_fail"].as_i64().unwrap_or(-1) as i32,
            neighbors: f["neighbors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| BlockStateId(v.as_u64().unwrap() as u32))
                .collect(),
            entity: f["entity"].as_str().unwrap(),
            shulker: ShulkerCollisionState {
                previous_progress: f.get("shulker_progress").map_or(0.0, |v| {
                    f32::from_bits(u32::from_str_radix(v[0].as_str().unwrap(), 16).unwrap())
                }),
                progress: f.get("shulker_progress").map_or(0.0, |v| {
                    f32::from_bits(u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
                }),
            },
            reads: 0,
            lights: 0,
            brightness: f["brightness"].as_i64().unwrap_or(15) as i32,
            light_fail: f["light_fail"].as_i64().unwrap_or(-1) as i32,
            entities: 0,
            fail: f["fail"].as_i64().unwrap() as i32,
            entity_fail: f["entity_fail"].as_i64().unwrap() as i32,
        };
        let p = std::array::from_fn(|i| f["position"][i].as_i64().unwrap() as i32);
        match survival.can_survive(
            &mut world,
            BlockStateId(f["state"].as_u64().unwrap() as u32),
            p,
        ) {
            Ok(value) => writeln!(world.out, "survive {value}")?,
            Err(_) => writeln!(world.out, "survive error")?,
        }
        writeln!(
            world.out,
            "counts {} {} {} {}",
            world.reads, world.entities, world.lights, world.fluid_calls
        )?;
    }
    out.flush()?;
    Ok(())
}
