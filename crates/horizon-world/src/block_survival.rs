//! Initial block-survival rules over version-bound definitions and world queries.
//! Unsupported definitions remain explicit; survival is not inferred from shape.
use crate::{
    block_collision::{BlockCollisionRegistry, CollisionQueryError, DynamicCollisionWorld},
    block_properties::{BlockPropertiesRegistry, BlockProperty, BoundBlockProperty},
    block_support::{BlockSupportRegistry, SupportQueryError, SupportType},
    compression::invalid,
    fluid::{FluidKind, FluidState},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::geometry::Direction;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurvivalRule {
    Fire,
    Cactus,
    Path,
    Mushroom,
    Multiface,
    Vine,
    HangingMoss,
    SnowLayer,
    SeaPickle,
    Bell,
    GrowingPlant,
    MangrovePropagule,
    Speleothem,
    Scaffolding,
    PistonHead,
    ChorusPlant,
    ChorusFlower,
    PitcherCrop,
    SugarCane,
    LilyPad,
    Frogspawn,
    SporeBlossom,
    SmallDripleaf,
    TallSeagrass,
    BigDripleaf,
    BigDripleafStem,
    Farmland,
    Cocoa,
    Crop,
    DoublePlant,
    Door,
    TagBelow(String),
    CactusFlower,
    Seagrass,
    BubbleColumn,
    RedstoneWire,
    Always,
    SolidBelow,
    SolidBehind,
    NonAirBelow,
    MossCarpet,
    CenterBelow,
    Lantern,
    FullAbove,
    FullBehind,
    RigidOrCenterBelow,
    RigidBelow,
    FullBelow,
    CenterAbove,
    FaceAttached,
    SupportsBamboo,
}
/// Version-specific definition inputs, independent of evaluated survival results.
#[derive(Clone, Debug, Default)]
pub struct SurvivalDefinitions {
    pub rules: BTreeMap<String, SurvivalRule>,
    pub ignition_odds: BTreeMap<String, i32>,
    pub fence_gates: BTreeSet<String>,
}
#[derive(Clone, Copy)]
enum Rule {
    Fire,
    Cactus,
    Path,
    Mushroom,
    Multiface(u8),
    Vine(u8),
    HangingMoss,
    SnowLayer,
    SeaPickle,
    GrowingPlant {
        head: BlockStateId,
        body: BlockStateId,
        direction: Direction,
        kelp: bool,
    },
    MangrovePropagule(bool),
    Speleothem(Direction),
    Scaffolding,
    PistonHead {
        direction: Direction,
        sticky: bool,
    },
    ChorusPlant,
    ChorusFlower,
    PitcherCrop(bool),
    SugarCane,
    LilyPad,
    Frogspawn,
    SporeBlossom,
    SmallDripleaf(bool),
    TallSeagrass(bool),
    BigDripleaf,
    BigDripleafStem,
    Farmland,
    Cocoa(Direction),
    Crop,
    DoublePlant(bool),
    Door(bool),
    TagBelow(usize),
    CactusFlower,
    Seagrass,
    BubbleColumn,
    RedstoneWire,
    Always,
    Solid(Direction),
    NonAirBelow,
    MossCarpet(bool),
    Support {
        face: Direction,
        kind: SupportType,
        center_restriction: bool,
    },
    RigidOrCenterBelow,
    SupportsBamboo,
}
pub trait SurvivalWorld: DynamicCollisionWorld {
    fn fluid_state(&mut self, position: [i32; 3]) -> Result<FluidState, Self::Error>;
    fn raw_brightness(&mut self, position: [i32; 3], sky_darken: i32) -> Result<i32, Self::Error>;
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, Self::Error>;
}
#[derive(Debug)]
pub enum SurvivalError<E> {
    World(E),
    Support(SupportQueryError<E>),
    Collision(CollisionQueryError<E>),
    Registry(io::Error),
    Unsupported(BlockStateId),
}
impl<E: std::fmt::Display> std::fmt::Display for SurvivalError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Support(e) => e.fmt(f),
            Self::Collision(e) => e.fmt(f),
            Self::Registry(e) => e.fmt(f),
            Self::Unsupported(id) => write!(f, "unsupported block survival state {}", id.0),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for SurvivalError<E> {}
/// A content mod's block whose states can't take the rule its template gave
/// it (a growing-plant family vanilla doesn't have, a property it lacks) gets
/// no rule, named once in the log, as a condition vanilla doesn't know: one
/// block doesn't stop every world from starting (#12428). Vanilla's own rules
/// stay strict.
fn rule_or_skip(
    name: &str,
    built: io::Result<Rule>,
    skipped: &mut BTreeSet<String>,
) -> io::Result<Option<Rule>> {
    match built {
        Ok(rule) => Ok(Some(rule)),
        Err(e) if !name.starts_with("minecraft:") => {
            if skipped.insert(name.to_owned()) {
                log::warn!("{name} has no block survival rule: {e}");
            }
            Ok(None)
        }
        Err(e) => Err(e),
    }
}
pub struct BlockSurvivalRegistry<'r> {
    registry: &'r BlockRegistry,
    support: Arc<BlockSupportRegistry<'r>>,
    collision: Arc<BlockCollisionRegistry<'r>>,
    solid: BoundBlockProperty,
    liquid: BoundBlockProperty,
    solid_render: BoundBlockProperty,
    flammable: BTreeSet<BlockStateId>,
    fence_gates: BTreeSet<BlockStateId>,
    cactus_support: BTreeSet<BlockStateId>,
    mushroom_light_override: BTreeSet<BlockStateId>,
    rules: Vec<Option<Rule>>,
    /// The states whose own template asks its ground
    /// (`Behaviours::states_asking_their_ground`): the only ones whose rules may consult
    /// the ground hooks (#17943).
    asks_their_ground: BTreeSet<BlockStateId>,
    unstable_bottom_center: BTreeSet<BlockStateId>,
    supports_bamboo: BTreeSet<BlockStateId>,
    bound_tags: Vec<BTreeSet<BlockStateId>>,
    cactus_flower: BTreeSet<BlockStateId>,
    no_seagrass: BTreeSet<BlockStateId>,
    vegetation: BTreeSet<BlockStateId>,
    dripleaf: BTreeSet<BlockStateId>,
    maintains_farmland: BTreeSet<BlockStateId>,
    cocoa: BTreeSet<BlockStateId>,
    crops: BTreeSet<BlockStateId>,
    no_snow: BTreeSet<BlockStateId>,
    snow_override: BTreeSet<BlockStateId>,
    mangrove: BTreeSet<BlockStateId>,
    hanging_mangrove: BTreeSet<BlockStateId>,
    no_kelp: BTreeSet<BlockStateId>,
    chorus_plant: BTreeSet<BlockStateId>,
    chorus_flower: BTreeSet<BlockStateId>,
    lily_pad: BTreeSet<BlockStateId>,
    frogspawn: BTreeSet<BlockStateId>,
    small_dripleaf: BTreeSet<BlockStateId>,
    sugar_cane: BTreeSet<BlockStateId>,
    sugar_cane_adjacent: BTreeSet<BlockStateId>,
    fluid_tags: [u8; 5],
    bubble_up: BTreeSet<BlockStateId>,
    bubble_down: BTreeSet<BlockStateId>,
}
impl<'r> BlockSurvivalRegistry<'r> {
    pub fn new(
        registry: &'r BlockRegistry,
        properties: &BlockPropertiesRegistry<'r>,
        support: Arc<BlockSupportRegistry<'r>>,
        collision: Arc<BlockCollisionRegistry<'r>>,
        definitions: &SurvivalDefinitions,
        tags: &BTreeMap<String, Vec<String>>,
        fluid_tags: &BTreeMap<String, Vec<String>>,
        asks_their_ground: BTreeSet<BlockStateId>,
    ) -> io::Result<Self> {
        if !std::ptr::eq(registry, properties.registry())
            || !std::ptr::eq(registry, support.registry())
            || !std::ptr::eq(registry, collision.registry())
        {
            return Err(invalid("survival inputs use different registries"));
        }
        for name in definitions
            .rules
            .keys()
            .chain(definitions.ignition_odds.keys())
            .chain(definitions.fence_gates.iter())
        {
            if registry.default_state(name).is_none() {
                return Err(invalid("unknown block survival definition"));
            }
        }
        let mut rules = Vec::with_capacity(registry.len());
        let mut skipped = BTreeSet::new();
        let mut tag_names = Vec::<String>::new();
        let mut tag_indices = BTreeMap::<String, usize>::new();
        for id in 0..registry.len() {
            let state = registry.state(BlockStateId(id as u32)).unwrap();
            let Some(rule) = definitions.rules.get(&state.name) else {
                rules.push(None);
                continue;
            };
            let property = |name: &str| {
                state
                    .properties
                    .get(name)
                    .map(String::as_str)
                    .ok_or_else(|| invalid("missing survival property"))
            };
            let facing = || -> io::Result<Direction> {
                match property("facing")? {
                    "down" => Ok(Direction::Down),
                    "up" => Ok(Direction::Up),
                    "north" => Ok(Direction::North),
                    "south" => Ok(Direction::South),
                    "west" => Ok(Direction::West),
                    "east" => Ok(Direction::East),
                    _ => Err(invalid("invalid survival facing")),
                }
            };
            let boolean = |name: &str| -> io::Result<bool> {
                match property(name)? {
                    "true" => Ok(true),
                    "false" => Ok(false),
                    _ => Err(invalid("invalid survival boolean")),
                }
            };
            let upper_half = || -> io::Result<bool> {
                match property("half")? {
                    "upper" => Ok(true),
                    "lower" => Ok(false),
                    _ => Err(invalid("invalid survival half")),
                }
            };
            let full = |face| Rule::Support {
                face,
                kind: SupportType::Full,
                center_restriction: false,
            };
            let faces = || -> io::Result<u8> {
                let mut mask = 0;
                for direction in Direction::ALL {
                    if state.properties.contains_key(direction_name(direction))
                        && boolean(direction_name(direction))?
                    {
                        mask |= 1 << direction as u8;
                    }
                }
                Ok(mask)
            };
            let built = (|| -> io::Result<Rule> {
                Ok(match rule {
                    SurvivalRule::Fire => Rule::Fire,
                    SurvivalRule::Cactus => Rule::Cactus,
                    SurvivalRule::Path => Rule::Path,
                    SurvivalRule::Mushroom => Rule::Mushroom,
                    SurvivalRule::Multiface => Rule::Multiface(faces()?),
                    SurvivalRule::Vine => Rule::Vine(faces()?),
                    SurvivalRule::HangingMoss => Rule::HangingMoss,
                    SurvivalRule::SnowLayer => Rule::SnowLayer,
                    SurvivalRule::SeaPickle => Rule::SeaPickle,
                    SurvivalRule::Bell => {
                        let (face, kind, center_restriction) = match property("attachment")? {
                            "floor" => (Direction::Up, SupportType::Full, false),
                            "ceiling" => (Direction::Down, SupportType::Center, true),
                            "single_wall" | "double_wall" => {
                                (facing()?.opposite(), SupportType::Full, false)
                            }
                            _ => return Err(invalid("invalid bell attachment")),
                        };
                        Rule::Support {
                            face,
                            kind,
                            center_restriction,
                        }
                    }
                    SurvivalRule::GrowingPlant => {
                        let (head, body, direction, kelp) = match state.name.as_str() {
                            "minecraft:kelp" | "minecraft:kelp_plant" => (
                                "minecraft:kelp",
                                "minecraft:kelp_plant",
                                Direction::Up,
                                true,
                            ),
                            "minecraft:cave_vines" | "minecraft:cave_vines_plant" => (
                                "minecraft:cave_vines",
                                "minecraft:cave_vines_plant",
                                Direction::Down,
                                false,
                            ),
                            "minecraft:twisting_vines" | "minecraft:twisting_vines_plant" => (
                                "minecraft:twisting_vines",
                                "minecraft:twisting_vines_plant",
                                Direction::Up,
                                false,
                            ),
                            "minecraft:weeping_vines" | "minecraft:weeping_vines_plant" => (
                                "minecraft:weeping_vines",
                                "minecraft:weeping_vines_plant",
                                Direction::Down,
                                false,
                            ),
                            _ => return Err(invalid("unknown growing plant family")),
                        };
                        Rule::GrowingPlant {
                            head: registry
                                .default_state(head)
                                .ok_or_else(|| invalid("missing growing plant head"))?,
                            body: registry
                                .default_state(body)
                                .ok_or_else(|| invalid("missing growing plant body"))?,
                            direction,
                            kelp,
                        }
                    }
                    SurvivalRule::MangrovePropagule => Rule::MangrovePropagule(boolean("hanging")?),
                    SurvivalRule::Speleothem => {
                        Rule::Speleothem(match property("vertical_direction")? {
                            "up" => Direction::Up,
                            "down" => Direction::Down,
                            _ => return Err(invalid("invalid speleothem direction")),
                        })
                    }
                    SurvivalRule::Scaffolding => Rule::Scaffolding,
                    SurvivalRule::PistonHead => Rule::PistonHead {
                        direction: facing()?,
                        sticky: match property("type")? {
                            "normal" => false,
                            "sticky" => true,
                            _ => return Err(invalid("invalid piston head type")),
                        },
                    },
                    SurvivalRule::ChorusPlant => Rule::ChorusPlant,
                    SurvivalRule::ChorusFlower => Rule::ChorusFlower,
                    SurvivalRule::PitcherCrop => Rule::PitcherCrop(upper_half()?),
                    SurvivalRule::SugarCane => Rule::SugarCane,
                    SurvivalRule::LilyPad => Rule::LilyPad,
                    SurvivalRule::Frogspawn => Rule::Frogspawn,
                    SurvivalRule::SporeBlossom => Rule::SporeBlossom,
                    SurvivalRule::SmallDripleaf => Rule::SmallDripleaf(upper_half()?),
                    SurvivalRule::TallSeagrass => Rule::TallSeagrass(upper_half()?),
                    SurvivalRule::BigDripleaf => Rule::BigDripleaf,
                    SurvivalRule::BigDripleafStem => Rule::BigDripleafStem,
                    SurvivalRule::Farmland => Rule::Farmland,
                    SurvivalRule::Cocoa => Rule::Cocoa(facing()?),
                    SurvivalRule::Crop => Rule::Crop,
                    SurvivalRule::DoublePlant => Rule::DoublePlant(upper_half()?),
                    SurvivalRule::Door => Rule::Door(upper_half()?),
                    SurvivalRule::TagBelow(name) => {
                        Rule::TagBelow(*tag_indices.entry(name.clone()).or_insert_with(|| {
                            let index = tag_names.len();
                            tag_names.push(name.clone());
                            index
                        }))
                    }
                    SurvivalRule::CactusFlower => Rule::CactusFlower,
                    SurvivalRule::Seagrass => Rule::Seagrass,
                    SurvivalRule::BubbleColumn => Rule::BubbleColumn,
                    SurvivalRule::RedstoneWire => Rule::RedstoneWire,
                    SurvivalRule::Always => Rule::Always,
                    SurvivalRule::SolidBelow => Rule::Solid(Direction::Down),
                    SurvivalRule::SolidBehind => Rule::Solid(facing()?.opposite()),
                    SurvivalRule::NonAirBelow => Rule::NonAirBelow,
                    SurvivalRule::MossCarpet => Rule::MossCarpet(boolean("bottom")?),
                    SurvivalRule::CenterBelow => Rule::Support {
                        face: Direction::Up,
                        kind: SupportType::Center,
                        center_restriction: true,
                    },
                    SurvivalRule::Lantern => Rule::Support {
                        face: if boolean("hanging")? {
                            Direction::Down
                        } else {
                            Direction::Up
                        },
                        kind: SupportType::Center,
                        center_restriction: true,
                    },
                    SurvivalRule::FullAbove => full(Direction::Down),
                    SurvivalRule::FullBehind => full(facing()?),
                    SurvivalRule::RigidOrCenterBelow => Rule::RigidOrCenterBelow,
                    SurvivalRule::RigidBelow => Rule::Support {
                        face: Direction::Up,
                        kind: SupportType::Rigid,
                        center_restriction: false,
                    },
                    SurvivalRule::FullBelow => full(Direction::Up),
                    SurvivalRule::CenterAbove => Rule::Support {
                        face: Direction::Down,
                        kind: SupportType::Center,
                        center_restriction: false,
                    },
                    SurvivalRule::FaceAttached => full(match property("face")? {
                        "floor" => Direction::Up,
                        "ceiling" => Direction::Down,
                        "wall" => facing()?,
                        _ => return Err(invalid("invalid attachment face")),
                    }),
                    SurvivalRule::SupportsBamboo => Rule::SupportsBamboo,
                })
            })();
            rules.push(rule_or_skip(&state.name, built, &mut skipped)?);
        }
        let tag_states = |name: &str| -> io::Result<BTreeSet<BlockStateId>> {
            let names = tags
                .get(name)
                .into_iter()
                .flatten()
                .collect::<BTreeSet<_>>();
            for name in &names {
                if registry.default_state(name).is_none() {
                    return Err(invalid("unknown survival tag member"));
                }
            }
            Ok((0..registry.len())
                .map(|i| BlockStateId(i as u32))
                .filter(|id| names.contains(&registry.state(*id).unwrap().name))
                .collect())
        };
        let fluid_tag = |name: &str| -> io::Result<u8> {
            fluid_tags
                .get(name)
                .into_iter()
                .flatten()
                .try_fold(0, |mask, name| {
                    Ok(mask
                        | FluidKind::from_name(name)
                            .ok_or_else(|| invalid("unknown survival fluid tag member"))?
                            .mask())
                })
        };
        Ok(Self {
            registry,
            support,
            collision,
            solid: properties.bind(BlockProperty::Solid),
            liquid: properties.bind(BlockProperty::Liquid),
            solid_render: properties.bind(BlockProperty::SolidRender),
            flammable: (0..registry.len())
                .map(|i| BlockStateId(i as u32))
                .filter(|id| {
                    let state = registry.state(*id).unwrap();
                    definitions
                        .ignition_odds
                        .get(&state.name)
                        .is_some_and(|value| *value > 0)
                        && state
                            .properties
                            .get("waterlogged")
                            .is_none_or(|value| value != "true")
                })
                .collect(),
            fence_gates: (0..registry.len())
                .map(|i| BlockStateId(i as u32))
                .filter(|id| {
                    definitions
                        .fence_gates
                        .contains(&registry.state(*id).unwrap().name)
                })
                .collect(),
            cactus_support: tag_states("minecraft:supports_cactus")?,
            mushroom_light_override: tag_states("minecraft:overrides_mushroom_light_requirement")?,
            rules,
            asks_their_ground,
            unstable_bottom_center: tag_states("minecraft:unstable_bottom_center")?,
            supports_bamboo: tag_states("minecraft:supports_bamboo")?,
            bound_tags: tag_names
                .iter()
                .map(|name| tag_states(name))
                .collect::<io::Result<_>>()?,
            cactus_flower: tag_states("minecraft:support_override_cactus_flower")?,
            no_seagrass: tag_states("minecraft:cannot_support_seagrass")?,
            vegetation: tag_states("minecraft:supports_vegetation")?,
            dripleaf: tag_states("minecraft:supports_big_dripleaf")?,
            maintains_farmland: tag_states("minecraft:maintains_farmland")?,
            cocoa: tag_states("minecraft:supports_cocoa")?,
            crops: tag_states("minecraft:supports_crops")?,
            no_snow: tag_states("minecraft:cannot_support_snow_layer")?,
            snow_override: tag_states("minecraft:support_override_snow_layer")?,
            mangrove: tag_states("minecraft:supports_mangrove_propagule")?,
            hanging_mangrove: tag_states("minecraft:supports_hanging_mangrove_propagule")?,
            no_kelp: tag_states("minecraft:cannot_support_kelp")?,
            chorus_plant: tag_states("minecraft:supports_chorus_plant")?,
            chorus_flower: tag_states("minecraft:supports_chorus_flower")?,
            lily_pad: tag_states("minecraft:supports_lily_pad")?,
            frogspawn: tag_states("minecraft:supports_frogspawn")?,
            small_dripleaf: tag_states("minecraft:supports_small_dripleaf")?,
            sugar_cane: tag_states("minecraft:supports_sugar_cane")?,
            sugar_cane_adjacent: tag_states("minecraft:supports_sugar_cane_adjacently")?,
            fluid_tags: [
                fluid_tag("minecraft:water")?,
                fluid_tag("minecraft:supports_lily_pad")?,
                fluid_tag("minecraft:supports_frogspawn")?,
                fluid_tag("minecraft:supports_sugar_cane_adjacently")?,
                fluid_tag("minecraft:lava")?,
            ],
            bubble_up: tag_states("minecraft:enables_bubble_column_push_up")?,
            bubble_down: tag_states("minecraft:enables_bubble_column_drag_down")?,
        })
    }
    fn read<W: SurvivalWorld>(
        &self,
        world: &mut W,
        position: [i32; 3],
    ) -> Result<BlockStateId, SurvivalError<W::Error>> {
        let state = world.block_state(position).map_err(SurvivalError::World)?;
        if self.registry.state(state).is_none() {
            return Err(SurvivalError::Registry(invalid(
                "world survival state outside registry",
            )));
        }
        Ok(state)
    }
    fn support<W: SurvivalWorld>(
        &self,
        world: &mut W,
        position: [i32; 3],
        face: Direction,
        kind: SupportType,
        center_restriction: bool,
    ) -> Result<bool, SurvivalError<W::Error>> {
        let state = self.read(world, position)?;
        if center_restriction
            && face == Direction::Down
            && self.unstable_bottom_center.contains(&state)
        {
            return Ok(false);
        }
        self.support
            .is_face_sturdy(world, state, position, face, kind)
            .map_err(SurvivalError::Support)
    }
    fn is_air(&self, state: BlockStateId) -> bool {
        matches!(
            self.registry.state(state).unwrap().name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        )
    }
    fn face_property(&self, state: BlockStateId, direction: Direction) -> bool {
        self.registry
            .state(state)
            .unwrap()
            .properties
            .get(direction_name(direction))
            .is_some_and(|value| value == "true")
    }
    /// Native multiface attachment at a candidate position, testing support
    /// first and then empty-context collision on the neighbor's opposite face.
    pub fn can_attach_to<W: SurvivalWorld>(
        &self,
        world: &mut W,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, SurvivalError<W::Error>> {
        let step = direction.normal();
        let at = std::array::from_fn(|i| position[i].wrapping_add(step[i]));
        let neighbor = self.read(world, at)?;
        self.can_attach(world, neighbor, at, direction)
    }
    fn can_attach<W: SurvivalWorld>(
        &self,
        world: &mut W,
        neighbor: BlockStateId,
        at: [i32; 3],
        direction: Direction,
    ) -> Result<bool, SurvivalError<W::Error>> {
        let face = direction.opposite();
        if self
            .support
            .is_face_sturdy(world, neighbor, at, face, SupportType::Full)
            .map_err(SurvivalError::Support)?
        {
            return Ok(true);
        }
        let shape = self
            .collision
            .collision_shape(world, neighbor, at)
            .map_err(SurvivalError::Collision)?;
        SupportType::Full
            .is_supporting(&shape, face)
            .map_err(|error| SurvivalError::Registry(invalid(error.to_string())))
    }
    fn is_named(&self, state: BlockStateId, name: &str) -> bool {
        self.registry.state(state).unwrap().name == name
    }
    fn same_block(&self, first: BlockStateId, second: BlockStateId) -> bool {
        self.registry.state(first).unwrap().name == self.registry.state(second).unwrap().name
    }
    fn scaffold_distance(&self, state: BlockStateId) -> io::Result<Option<i32>> {
        if !self.is_named(state, "minecraft:scaffolding") {
            return Ok(None);
        }
        self.registry
            .state(state)
            .unwrap()
            .properties
            .get("distance")
            .and_then(|v| v.parse::<i32>().ok())
            .filter(|v| (0..=7).contains(v))
            .map(Some)
            .ok_or_else(|| invalid("invalid scaffolding distance"))
    }
    fn is_lower_half(&self, below: BlockStateId, candidate: BlockStateId) -> bool {
        let neighbor = self.registry.state(below).unwrap();
        neighbor.name == self.registry.state(candidate).unwrap().name
            && neighbor
                .properties
                .get("half")
                .is_some_and(|v| v == "lower")
    }
    fn fluid_matches(&self, tag: usize, fluid: FluidState) -> bool {
        self.fluid_tags[tag] & fluid.kind().mask() != 0
    }
    /// The plant's own ground rule for the plant `state` standing at `position`, as the live
    /// game's `VegetationBlock.mayPlaceOn` asks it: the parts of the soil under it first, the
    /// parts of the plant's own block second (`Survive::may_place_on`, `level/survive.rs`), and
    /// `None` when neither answered, leaving the caller's own rule to decide.
    fn plant_place_on<W: SurvivalWorld>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<Option<bool>, SurvivalError<W::Error>> {
        let below = relative(position, Direction::Down);
        let ground = self.read(world, below)?;
        if let Some(sustains) =
            crate::native_worldgen_hooks::can_sustain_plant(state, ground, below)
        {
            return Ok(Some(sustains));
        }
        Ok(crate::native_worldgen_hooks::may_place_on(
            state, ground, below,
        ))
    }
    pub fn can_survive<W: SurvivalWorld>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, SurvivalError<W::Error>> {
        // The block's own rule is a mod's to replace: the parts of `state`'s block answer
        // before the table does, as they do in the live game (#17943). Asked before the rule
        // lookup, since a mod block's states have no rule here (Unsupported).
        if let Some(survives) = crate::native_worldgen_hooks::can_survive(state, position) {
            return Ok(survives);
        }
        let rule = self.rules.get(state.0 as usize).ok_or_else(|| {
            SurvivalError::Registry(invalid("survival candidate outside registry"))
        })?;
        let Some(rule) = *rule else {
            // A block the table has no rule for: its template's own `canSurvive` answers,
            // which asks the ground only in the templates that ask it at all — a modded
            // torch's parts never decide where it stands — and an unanswered one is
            // unsupported (#17943).
            if !self.asks_their_ground.contains(&state) {
                return Err(SurvivalError::Unsupported(state));
            }
            return match self.plant_place_on(world, state, position)? {
                Some(survives) => Ok(survives),
                None => Err(SurvivalError::Unsupported(state)),
            };
        };
        match rule {
            Rule::Fire => {
                let below = relative(position, Direction::Down);
                let neighbor = self.read(world, below)?;
                if self
                    .support
                    .is_face_sturdy(world, neighbor, below, Direction::Up, SupportType::Full)
                    .map_err(SurvivalError::Support)?
                {
                    return Ok(true);
                }
                for direction in Direction::ALL {
                    let neighbor = self.read(world, relative(position, direction))?;
                    if self.flammable.contains(&neighbor) {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Rule::Cactus => {
                for direction in [
                    Direction::North,
                    Direction::East,
                    Direction::South,
                    Direction::West,
                ] {
                    let at = relative(position, direction);
                    let neighbor = self.read(world, at)?;
                    if self.solid.contains(neighbor)
                        || self
                            .fluid_matches(4, world.fluid_state(at).map_err(SurvivalError::World)?)
                    {
                        return Ok(false);
                    }
                }
                let neighbor = self.read(world, relative(position, Direction::Down))?;
                if !self.same_block(neighbor, state) && !self.cactus_support.contains(&neighbor) {
                    return Ok(false);
                }
                let above = self.read(world, relative(position, Direction::Up))?;
                Ok(!self.liquid.contains(above))
            }
            Rule::Path => {
                let above = self.read(world, relative(position, Direction::Up))?;
                Ok(!self.solid.contains(above) || self.fence_gates.contains(&above))
            }
            Rule::Mushroom => {
                let below = relative(position, Direction::Down);
                let neighbor = self.read(world, below)?;
                if self.mushroom_light_override.contains(&neighbor) {
                    return Ok(true);
                }
                if world
                    .raw_brightness(position, 0)
                    .map_err(SurvivalError::World)?
                    >= 13
                {
                    return Ok(false);
                }
                // `MushroomBlock.canSurvive` below 13 asks `mayPlaceOn`, which a mod may
                // replace: the soil's parts first, the plant's second (#17943).
                if let Some(may) = self.plant_place_on(world, state, position)? {
                    return Ok(may);
                }
                Ok(self.solid_render.contains(neighbor))
            }
            Rule::Multiface(faces) => {
                for direction in Direction::ALL {
                    if faces & (1 << direction as u8) != 0 {
                        let at = relative(position, direction);
                        let neighbor = self.read(world, at)?;
                        if !self.can_attach(world, neighbor, at, direction)? {
                            return Ok(false);
                        }
                    }
                }
                Ok(faces != 0)
            }
            Rule::HangingMoss => {
                let at = relative(position, Direction::Up);
                let neighbor = self.read(world, at)?;
                Ok(self.can_attach(world, neighbor, at, Direction::Up)?
                    || self.same_block(neighbor, state))
            }
            Rule::Vine(faces) => {
                let above = relative(position, Direction::Up);
                let mut any = false;
                let mut cached_above = None;
                if faces & (1 << Direction::Up as u8) != 0 {
                    let neighbor = self.read(world, above)?;
                    // getUpdatedState passes DOWN here, unlike canSupportAtFace.
                    any = self.can_attach(world, neighbor, above, Direction::Down)?;
                }
                for direction in [
                    Direction::North,
                    Direction::East,
                    Direction::South,
                    Direction::West,
                ] {
                    if faces & (1 << direction as u8) == 0 {
                        continue;
                    }
                    let at = relative(position, direction);
                    let neighbor = self.read(world, at)?;
                    let mut valid = self.can_attach(world, neighbor, at, direction)?;
                    if !valid {
                        let neighbor = self.read(world, above)?;
                        valid = self.same_block(neighbor, state)
                            && self.face_property(neighbor, direction);
                    }
                    if !valid {
                        let neighbor = match cached_above {
                            Some(neighbor) => neighbor,
                            None => {
                                let neighbor = self.read(world, above)?;
                                cached_above = Some(neighbor);
                                neighbor
                            }
                        };
                        valid = self.same_block(neighbor, state)
                            && self.face_property(neighbor, direction);
                    }
                    any |= valid;
                }
                Ok(any)
            }
            Rule::SnowLayer => {
                let at = relative(position, Direction::Down);
                let neighbor = self.read(world, at)?;
                if self.no_snow.contains(&neighbor) {
                    return Ok(false);
                }
                if self.snow_override.contains(&neighbor) {
                    return Ok(true);
                }
                let shape = self
                    .collision
                    .collision_shape(world, neighbor, at)
                    .map_err(SurvivalError::Collision)?;
                Ok(SupportType::Full
                    .is_supporting(&shape, Direction::Up)
                    .map_err(|error| SurvivalError::Registry(invalid(error.to_string())))?
                    || (self.same_block(neighbor, state)
                        && self
                            .registry
                            .state(neighbor)
                            .unwrap()
                            .properties
                            .get("layers")
                            .is_some_and(|value| value == "8")))
            }
            Rule::SeaPickle => {
                // `SeaPickleBlock.canSurvive` asks `mayPlaceOn`, which a mod may replace: the
                // soil's parts first, the plant's second (#17943).
                if let Some(may) = self.plant_place_on(world, state, position)? {
                    return Ok(may);
                }
                let at = relative(position, Direction::Down);
                let neighbor = self.read(world, at)?;
                let shape = self
                    .collision
                    .collision_shape(world, neighbor, at)
                    .map_err(SurvivalError::Collision)?;
                Ok(!shape
                    .face_shape(Direction::Up)
                    .map_err(|error| SurvivalError::Registry(invalid(error.to_string())))?
                    .is_empty()
                    || self
                        .support
                        .is_face_sturdy(world, neighbor, at, Direction::Up, SupportType::Full)
                        .map_err(SurvivalError::Support)?)
            }
            Rule::GrowingPlant {
                head,
                body,
                direction,
                kelp,
            } => {
                let at = relative(position, direction.opposite());
                let neighbor = self.read(world, at)?;
                if kelp && self.no_kelp.contains(&neighbor) {
                    return Ok(false);
                }
                Ok(self.same_block(neighbor, head)
                    || self.same_block(neighbor, body)
                    || self
                        .support
                        .is_face_sturdy(world, neighbor, at, direction, SupportType::Full)
                        .map_err(SurvivalError::Support)?)
            }
            Rule::MangrovePropagule(hanging) => {
                let neighbor = self.read(
                    world,
                    relative(
                        position,
                        if hanging {
                            Direction::Up
                        } else {
                            Direction::Down
                        },
                    ),
                )?;
                Ok(if hanging {
                    self.hanging_mangrove.contains(&neighbor)
                } else {
                    self.mangrove.contains(&neighbor)
                })
            }
            Rule::Speleothem(direction) => {
                let at = relative(position, direction.opposite());
                let neighbor = self.read(world, at)?;
                Ok(self
                    .support
                    .is_face_sturdy(world, neighbor, at, direction, SupportType::Full)
                    .map_err(SurvivalError::Support)?
                    || (self.same_block(neighbor, state)
                        && self
                            .registry
                            .state(neighbor)
                            .unwrap()
                            .properties
                            .get("vertical_direction")
                            == self
                                .registry
                                .state(state)
                                .unwrap()
                                .properties
                                .get("vertical_direction")))
            }
            Rule::Scaffolding => {
                let below = relative(position, Direction::Down);
                let neighbor = self.read(world, below)?;
                let mut distance = if let Some(distance) = self
                    .scaffold_distance(neighbor)
                    .map_err(SurvivalError::Registry)?
                {
                    distance
                } else {
                    if self
                        .support
                        .is_face_sturdy(world, neighbor, below, Direction::Up, SupportType::Full)
                        .map_err(SurvivalError::Support)?
                    {
                        return Ok(true);
                    }
                    7
                };
                for direction in [
                    Direction::North,
                    Direction::East,
                    Direction::South,
                    Direction::West,
                ] {
                    let neighbor = self.read(world, relative(position, direction))?;
                    if let Some(adjacent) = self
                        .scaffold_distance(neighbor)
                        .map_err(SurvivalError::Registry)?
                    {
                        distance = distance.min(adjacent + 1);
                        if distance == 1 {
                            break;
                        }
                    }
                }
                Ok(distance < 7)
            }
            Rule::PistonHead { direction, sticky } => {
                let neighbor = self.read(world, relative(position, direction.opposite()))?;
                let definition = self.registry.state(neighbor).unwrap();
                let base = if sticky {
                    "minecraft:sticky_piston"
                } else {
                    "minecraft:piston"
                };
                Ok(((definition.name == base
                    && definition
                        .properties
                        .get("extended")
                        .is_some_and(|v| v == "true"))
                    || definition.name == "minecraft:moving_piston")
                    && definition.properties.get("facing")
                        == self.registry.state(state).unwrap().properties.get("facing"))
            }
            Rule::ChorusPlant => {
                let below = self.read(world, relative(position, Direction::Down))?;
                let above = self.read(world, relative(position, Direction::Up))?;
                let crowded = !self.is_air(above) && !self.is_air(below);
                for direction in [
                    Direction::North,
                    Direction::East,
                    Direction::South,
                    Direction::West,
                ] {
                    let at = relative(position, direction);
                    let neighbor = self.read(world, at)?;
                    if self.same_block(neighbor, state) {
                        if crowded {
                            return Ok(false);
                        }
                        let support = self.read(world, relative(at, Direction::Down))?;
                        if self.same_block(support, state) || self.chorus_plant.contains(&support) {
                            return Ok(true);
                        }
                    }
                }
                Ok(self.same_block(below, state) || self.chorus_plant.contains(&below))
            }
            Rule::ChorusFlower => {
                let below = self.read(world, relative(position, Direction::Down))?;
                if self.is_named(below, "minecraft:chorus_plant")
                    || self.chorus_flower.contains(&below)
                {
                    return Ok(true);
                }
                if !self.is_air(below) {
                    return Ok(false);
                }
                let mut found = false;
                for direction in [
                    Direction::North,
                    Direction::East,
                    Direction::South,
                    Direction::West,
                ] {
                    let neighbor = self.read(world, relative(position, direction))?;
                    if self.is_named(neighbor, "minecraft:chorus_plant") {
                        if found {
                            return Ok(false);
                        }
                        found = true;
                    } else if !self.is_air(neighbor) {
                        return Ok(false);
                    }
                }
                Ok(found)
            }
            Rule::PitcherCrop(upper) => {
                if !upper
                    && world
                        .raw_brightness(position, 0)
                        .map_err(SurvivalError::World)?
                        < 8
                {
                    return Ok(false);
                }
                let below = self.read(world, relative(position, Direction::Down))?;
                Ok(if upper {
                    self.is_lower_half(below, state)
                } else {
                    self.crops.contains(&below)
                })
            }
            Rule::SugarCane => {
                let below = relative(position, Direction::Down);
                let neighbor = self.read(world, below)?;
                if self.registry.state(neighbor).unwrap().name
                    == self.registry.state(state).unwrap().name
                {
                    return Ok(true);
                }
                if !self.sugar_cane.contains(&neighbor) {
                    return Ok(false);
                }
                for direction in [
                    Direction::North,
                    Direction::East,
                    Direction::South,
                    Direction::West,
                ] {
                    let at = relative(below, direction);
                    let adjacent = self.read(world, at)?;
                    let fluid = world.fluid_state(at).map_err(SurvivalError::World)?;
                    if self.fluid_matches(3, fluid) || self.sugar_cane_adjacent.contains(&adjacent)
                    {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Rule::LilyPad => {
                let below = relative(position, Direction::Down);
                let neighbor = self.read(world, below)?;
                let fluid_below = world.fluid_state(below).map_err(SurvivalError::World)?;
                let fluid_here = world.fluid_state(position).map_err(SurvivalError::World)?;
                Ok(
                    (self.fluid_matches(1, fluid_below) || self.lily_pad.contains(&neighbor))
                        && fluid_here.kind() == FluidKind::Empty,
                )
            }
            Rule::Frogspawn => {
                let below = relative(position, Direction::Down);
                let fluid_below = world.fluid_state(below).map_err(SurvivalError::World)?;
                let fluid_here = world.fluid_state(position).map_err(SurvivalError::World)?;
                Ok((self.fluid_matches(2, fluid_below)
                    || self.frogspawn.contains(&self.read(world, below)?))
                    && fluid_here.kind() == FluidKind::Empty)
            }
            Rule::SporeBlossom => Ok(self.support(
                world,
                relative(position, Direction::Up),
                Direction::Down,
                SupportType::Center,
                true,
            )? && !self.fluid_matches(
                0,
                world.fluid_state(position).map_err(SurvivalError::World)?,
            )),
            Rule::SmallDripleaf(upper) => {
                let neighbor = self.read(world, relative(position, Direction::Down))?;
                if upper {
                    return Ok(self.is_lower_half(neighbor, state));
                }
                // The lower half's `canSurvive` asks the ground its template asks
                // (`SmallDripleafBlock` reaches `VegetationBlock.mayPlaceOn`), where a mod
                // may replace the plant's rule, before the table's tags decide (#17943).
                if let Some(may) = self.plant_place_on(world, state, position)? {
                    return Ok(may);
                }
                Ok(self.small_dripleaf.contains(&neighbor)
                    || (world
                        .fluid_state(position)
                        .map_err(SurvivalError::World)?
                        .kind()
                        == FluidKind::Water
                        && self.vegetation.contains(&neighbor)))
            }
            Rule::TallSeagrass(upper) => {
                let below = relative(position, Direction::Down);
                if upper {
                    return Ok(self.is_lower_half(self.read(world, below)?, state));
                }
                let fluid = world.fluid_state(position).map_err(SurvivalError::World)?;
                let neighbor = self.read(world, below)?;
                Ok(self
                    .support
                    .is_face_sturdy(world, neighbor, below, Direction::Up, SupportType::Full)
                    .map_err(SurvivalError::Support)?
                    && !self.no_seagrass.contains(&neighbor)
                    && self.fluid_matches(0, fluid)
                    && fluid.is_full())
            }
            Rule::BigDripleaf => {
                let below = self.read(world, relative(position, Direction::Down))?;
                Ok(matches!(
                    self.registry.state(below).unwrap().name.as_str(),
                    "minecraft:big_dripleaf" | "minecraft:big_dripleaf_stem"
                ) || self.dripleaf.contains(&below))
            }
            Rule::BigDripleafStem => {
                let below = self.read(world, relative(position, Direction::Down))?;
                let above = self.read(world, relative(position, Direction::Up))?;
                Ok(
                    (self.registry.state(below).unwrap().name == "minecraft:big_dripleaf_stem"
                        || self.dripleaf.contains(&below))
                        && matches!(
                            self.registry.state(above).unwrap().name.as_str(),
                            "minecraft:big_dripleaf" | "minecraft:big_dripleaf_stem"
                        ),
                )
            }
            Rule::Farmland => {
                let above = relative(position, Direction::Up);
                Ok(!self.solid.contains(self.read(world, above)?)
                    || self.maintains_farmland.contains(&self.read(world, above)?))
            }
            Rule::Cocoa(direction) => Ok(self
                .cocoa
                .contains(&self.read(world, relative(position, direction))?)),
            Rule::Crop => {
                if world
                    .raw_brightness(position, 0)
                    .map_err(SurvivalError::World)?
                    < 8
                {
                    return Ok(false);
                }
                // `CropBlock.mayPlaceOn`: the plant's own rule, which a mod may replace — the
                // soil's parts first, as the live game asks them, then the plant's.
                if let Some(may) = self.plant_place_on(world, state, position)? {
                    return Ok(may);
                }
                let ground = self.read(world, relative(position, Direction::Down))?;
                Ok(self.crops.contains(&ground))
            }
            Rule::DoublePlant(upper) => {
                let below = self.read(world, relative(position, Direction::Down))?;
                if !upper {
                    // The lower half's `canSurvive` goes on to the plant's ground rule
                    // (`VegetationBlock.mayPlaceOn`), where a mod may replace the tag the
                    // table holds, before that tag decides (#17943).
                    if let Some(may) = self.plant_place_on(world, state, position)? {
                        return Ok(may);
                    }
                    return Ok(self.vegetation.contains(&below));
                }
                let neighbor = self.registry.state(below).unwrap();
                Ok(neighbor.name == self.registry.state(state).unwrap().name
                    && neighbor
                        .properties
                        .get("half")
                        .is_some_and(|v| v == "lower"))
            }
            Rule::Door(upper) => {
                let below = relative(position, Direction::Down);
                let neighbor = self.read(world, below)?;
                if upper {
                    return Ok(self.registry.state(neighbor).unwrap().name
                        == self.registry.state(state).unwrap().name);
                }
                self.support
                    .is_face_sturdy(world, neighbor, below, Direction::Up, SupportType::Full)
                    .map_err(SurvivalError::Support)
            }
            Rule::TagBelow(index) => {
                // `VegetationBlock.mayPlaceOn`: the plant's ground rule, which a mod may
                // replace before the tag the table holds for it decides. Only a template
                // that asks its ground asks it here: soul fire shares this rule and keeps
                // its own base-block tag (#17943).
                if self.asks_their_ground.contains(&state)
                    && let Some(may) = self.plant_place_on(world, state, position)?
                {
                    return Ok(may);
                }
                let ground = self.read(world, relative(position, Direction::Down))?;
                Ok(self.bound_tags[index].contains(&ground))
            }
            Rule::CactusFlower => {
                let below = relative(position, Direction::Down);
                self.read(world, below)?;
                let neighbor = self.read(world, below)?;
                Ok(self.cactus_flower.contains(&neighbor)
                    || self
                        .support
                        .is_face_sturdy(world, neighbor, below, Direction::Up, SupportType::Center)
                        .map_err(SurvivalError::Support)?)
            }
            Rule::Seagrass => {
                let below = relative(position, Direction::Down);
                let neighbor = self.read(world, below)?;
                Ok(self
                    .support
                    .is_face_sturdy(world, neighbor, below, Direction::Up, SupportType::Full)
                    .map_err(SurvivalError::Support)?
                    && !self.no_seagrass.contains(&neighbor))
            }
            Rule::BubbleColumn => {
                let below = self.read(world, relative(position, Direction::Down))?;
                Ok(self.registry.state(below).unwrap().name
                    == self.registry.state(state).unwrap().name
                    || self.bubble_up.contains(&below)
                    || self.bubble_down.contains(&below))
            }
            Rule::RedstoneWire => {
                let below = relative(position, Direction::Down);
                let neighbor = self.read(world, below)?;
                Ok(self
                    .support
                    .is_face_sturdy(world, neighbor, below, Direction::Up, SupportType::Full)
                    .map_err(SurvivalError::Support)?
                    || self.registry.state(neighbor).unwrap().name == "minecraft:hopper")
            }
            Rule::Always => Ok(true),
            Rule::Solid(direction) => Ok(self
                .solid
                .contains(self.read(world, relative(position, direction))?)),
            Rule::NonAirBelow => {
                Ok(!self.is_air(self.read(world, relative(position, Direction::Down))?))
            }
            Rule::MossCarpet(base) => {
                let below = self.read(world, relative(position, Direction::Down))?;
                if base {
                    return Ok(!self.is_air(below));
                }
                let below = self.registry.state(below).unwrap();
                Ok(below.name == self.registry.state(state).unwrap().name
                    && below.properties.get("bottom").is_some_and(|v| v == "true"))
            }
            Rule::Support {
                face,
                kind,
                center_restriction,
            } => self.support(
                world,
                relative(position, face.opposite()),
                face,
                kind,
                center_restriction,
            ),
            Rule::RigidOrCenterBelow => {
                let below = relative(position, Direction::Down);
                Ok(
                    self.support(world, below, Direction::Up, SupportType::Rigid, false)?
                        || self.support(world, below, Direction::Up, SupportType::Center, true)?,
                )
            }
            Rule::SupportsBamboo => Ok(self
                .supports_bamboo
                .contains(&self.read(world, relative(position, Direction::Down))?)),
        }
    }
}
fn relative(position: [i32; 3], direction: Direction) -> [i32; 3] {
    let normal = direction.normal();
    std::array::from_fn(|i| position[i].wrapping_add(normal[i]))
}

fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Down => "down",
        Direction::Up => "up",
        Direction::North => "north",
        Direction::South => "south",
        Direction::West => "west",
        Direction::East => "east",
    }
}

#[cfg(test)]
mod rule_or_skip_tests {
    use super::{Rule, invalid, rule_or_skip};
    use std::collections::BTreeSet;

    /// A content block's unusable rule leaves it without one, counted once
    /// for all its states; a vanilla block's still fails the load.
    #[test]
    fn only_a_content_blocks_bad_rule_is_skipped() {
        let mut skipped = BTreeSet::new();
        let bad = || Err(invalid("unknown growing plant family"));
        for _ in 0..3 {
            let rule = rule_or_skip("biomesoplenty:spanish_moss", bad(), &mut skipped);
            assert!(rule.unwrap().is_none());
        }
        assert_eq!(skipped.len(), 1);
        assert!(rule_or_skip("minecraft:kelp", bad(), &mut skipped).is_err());
        let good = rule_or_skip("biomesoplenty:high_grass", Ok(Rule::Always), &mut skipped);
        assert!(matches!(good.unwrap(), Some(Rule::Always)));
    }
}

#[cfg(test)]
mod part_hook_tests {
    //! The survival table asks a block's parts before its own rule: `can_survive` of the
    //! candidate, and `mayPlaceOn` of the plant whose ground rule the table holds (#17943).
    use super::{
        BlockSurvivalRegistry, SurvivalDefinitions, SurvivalError, SurvivalRule, SurvivalWorld,
    };
    use crate::{
        block_collision::{BlockCollisionRegistry, DynamicCollisionWorld, ShulkerCollisionState},
        block_properties::BlockPropertiesRegistry,
        block_support::BlockSupportRegistry,
        collision::CachedCollisionRegistry,
        dynamic_support::DynamicSupportWorld,
        fluid::FluidState,
        native_worldgen_hooks::{self, NativeWorldgenHooks},
        registry::{BlockRegistry, BlockStateId},
    };
    use horizon_core::{geometry::VoxelShape, nbt::Tag};
    use std::{
        collections::{BTreeMap, BTreeSet},
        io,
        sync::{
            Mutex, OnceLock, PoisonError,
            atomic::{AtomicUsize, Ordering},
        },
    };

    /// The table's blocks in id order: air, the two grounds the vegetation tag holds and
    /// the crops tag does not, the crop and the plant the two rules belong to, a mod
    /// plant no rule in the table covers, the mushroom, its light-overriding ground, the
    /// pickle whose three rules ask a part about the ground, the soul fire that shares
    /// the vegetation rule without asking its ground, and a mod lantern the table has no
    /// rule for either and whose template asks no ground.
    const NAMES: [&str; 12] = [
        "minecraft:air",
        "minecraft:grass_block",
        "minecraft:sand",
        "minecraft:farmland",
        "minecraft:wheat",
        "minecraft:short_grass",
        "mymod:crystal",
        "minecraft:brown_mushroom",
        "minecraft:mycelium",
        "minecraft:sea_pickle",
        "minecraft:soul_fire",
        "mymod:lantern",
    ];
    const AIR: BlockStateId = BlockStateId(0);
    const GRASS: BlockStateId = BlockStateId(1);
    const SAND: BlockStateId = BlockStateId(2);
    const FARMLAND: BlockStateId = BlockStateId(3);
    const WHEAT: BlockStateId = BlockStateId(4);
    const SHORT_GRASS: BlockStateId = BlockStateId(5);
    const CRYSTAL: BlockStateId = BlockStateId(6);
    const MUSHROOM: BlockStateId = BlockStateId(7);
    const MYCELIUM: BlockStateId = BlockStateId(8);
    const SEA_PICKLE: BlockStateId = BlockStateId(9);
    const SOUL_FIRE: BlockStateId = BlockStateId(10);
    const LANTERN: BlockStateId = BlockStateId(11);
    /// Where a plant stands and where the ground under it is: somewhere no other test of
    /// this binary reads, so an answer set here reaches no other test's query.
    const PLANT: [i32; 3] = [7, 70, 7];
    const GROUND: [i32; 3] = [7, 69, 7];

    /// The blocks, defaulted and stateless, as the game's registry describes them.
    fn registry() -> &'static BlockRegistry {
        let states = NAMES
            .iter()
            .map(|name| Tag::String((*name).into()))
            .collect::<Vec<_>>();
        let defaults = NAMES
            .iter()
            .enumerate()
            .map(|(id, name)| (*name, Tag::Int(id as i32)));
        Box::leak(Box::new(
            BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                ("states", Tag::List(states)),
                ("defaults", Tag::compound(defaults)),
            ]))
            .unwrap(),
        ))
    }

    /// A collision table of one empty shape for every state, and — with `kind` — the
    /// support table that carries the same table under its own kind.
    fn shapes_tag(registry: &BlockRegistry, kind: Option<&str>) -> Tag {
        let states = (0..registry.len())
            .map(|id| {
                registry
                    .state(BlockStateId(id as u32))
                    .expect("registry index is in bounds")
                    .to_nbt()
            })
            .collect::<Vec<_>>();
        let empty = Tag::compound([
            ("x", Tag::LongArray(vec![0])),
            ("y", Tag::LongArray(vec![0])),
            ("z", Tag::LongArray(vec![0])),
            ("occupied", Tag::ByteArray(Vec::new())),
            ("cube", Tag::Byte(0)),
            ("cube_axes", Tag::Byte(0)),
            ("discrete_min", Tag::IntArray(vec![0; 3])),
            ("discrete_max", Tag::IntArray(vec![0; 3])),
        ]);
        let mut tag = Tag::compound([
            ("DataVersion", Tag::Int(crate::DATA_VERSION)),
            ("states", Tag::List(states)),
            ("shapes", Tag::List(vec![empty])),
            ("cached_shapes", Tag::IntArray(vec![0; registry.len()])),
        ]);
        if let Some(kind) = kind
            && let Tag::Compound(entries) = &mut tag
        {
            entries.insert("Kind".into(), Tag::String(kind.into()));
        }
        tag
    }

    /// The property table beside the collision shapes: no block is solid, liquid or
    /// replaceable in it, which these rules never ask about.
    fn properties_tag(occlusion: Tag, names: &[&str]) -> Tag {
        let blocks = names.iter().map(|name| (*name, Tag::Byte(0)));
        Tag::compound([
            ("DataVersion", Tag::Int(crate::DATA_VERSION)),
            (
                "states",
                occlusion
                    .get("states")
                    .expect("the shapes carry their state identities")
                    .clone(),
            ),
            ("blocks", Tag::compound(blocks)),
            ("occlusion", occlusion),
        ])
    }

    /// The tags the table's rules bind: the vegetation ground short grass stands on, the
    /// crops' farmland, the ground a mushroom needs no light on, and the base blocks soul
    /// fire keeps, which shares the vegetation rule without asking a ground.
    fn tags() -> BTreeMap<String, Vec<String>> {
        BTreeMap::from([
            (
                "minecraft:supports_vegetation".to_owned(),
                vec!["minecraft:grass_block".to_owned()],
            ),
            (
                "minecraft:supports_crops".to_owned(),
                vec!["minecraft:farmland".to_owned()],
            ),
            (
                "minecraft:overrides_mushroom_light_requirement".to_owned(),
                vec!["minecraft:mycelium".to_owned()],
            ),
            (
                "minecraft:soul_fire_base_blocks".to_owned(),
                vec!["minecraft:sand".to_owned()],
            ),
        ])
    }

    /// The rules the table holds: short grass stands on the vegetation tag, wheat on the
    /// crops' one, the mushroom on light and its own ground, the pickle on its support, the
    /// soul fire on its base blocks, and the two mod blocks have no rule at all.
    fn definitions() -> SurvivalDefinitions {
        SurvivalDefinitions {
            rules: BTreeMap::from([
                (
                    "minecraft:short_grass".to_owned(),
                    SurvivalRule::TagBelow("minecraft:supports_vegetation".to_owned()),
                ),
                ("minecraft:wheat".to_owned(), SurvivalRule::Crop),
                (
                    "minecraft:brown_mushroom".to_owned(),
                    SurvivalRule::Mushroom,
                ),
                ("minecraft:sea_pickle".to_owned(), SurvivalRule::SeaPickle),
                (
                    "minecraft:soul_fire".to_owned(),
                    SurvivalRule::TagBelow("minecraft:soul_fire_base_blocks".to_owned()),
                ),
            ]),
            ..Default::default()
        }
    }

    /// The states whose own templates ask the ground they stand on, as the behaviours'
    /// class chains name them: the plants of this table, the mod plant among them, and not
    /// soul fire's block nor the mod lantern, whose templates ask no ground.
    fn asks_their_ground() -> BTreeSet<BlockStateId> {
        BTreeSet::from([WHEAT, SHORT_GRASS, CRYSTAL, MUSHROOM, SEA_PICKLE])
    }

    /// The survival table of `registry` over the empty tables these rules never read
    /// further.
    fn table(
        registry: &'static BlockRegistry,
        names: &[&str],
        definitions: &SurvivalDefinitions,
        tags: &BTreeMap<String, Vec<String>>,
        asks_their_ground: BTreeSet<BlockStateId>,
    ) -> BlockSurvivalRegistry<'static> {
        let collisions = CachedCollisionRegistry::from_nbt(&shapes_tag(registry, None), registry)
            .expect("collision table");
        let properties = BlockPropertiesRegistry::from_nbt(
            &properties_tag(shapes_tag(registry, None), names),
            &collisions,
        )
        .expect("property table");
        let properties: &'static BlockPropertiesRegistry<'static> = Box::leak(Box::new(properties));
        let support =
            BlockSupportRegistry::from_nbt(&shapes_tag(registry, Some("block_support")), registry)
                .expect("support table");
        BlockSurvivalRegistry::new(
            registry,
            properties,
            std::sync::Arc::new(support),
            std::sync::Arc::new(
                BlockCollisionRegistry::new(std::sync::Arc::new(collisions))
                    .expect("collision registry"),
            ),
            definitions,
            tags,
            &BTreeMap::new(),
            asks_their_ground,
        )
        .expect("survival table")
    }

    /// The table under test over [`NAMES`].
    fn survival(tags: &BTreeMap<String, Vec<String>>) -> BlockSurvivalRegistry<'static> {
        table(
            registry(),
            &NAMES,
            &definitions(),
            tags,
            asks_their_ground(),
        )
    }

    /// The half plants in id order: air, the grass both their tags hold, sand they do not,
    /// and the lower and upper halves of small dripleaf and of a double plant.
    const HALVES: [(&str, Option<&str>); 7] = [
        ("minecraft:air", None),
        ("minecraft:grass_block", None),
        ("minecraft:sand", None),
        ("minecraft:small_dripleaf", Some("lower")),
        ("minecraft:small_dripleaf", Some("upper")),
        ("minecraft:tall_grass", Some("lower")),
        ("minecraft:tall_grass", Some("upper")),
    ];
    /// The same blocks once each, for the table keyed by name.
    const HALF_NAMES: [&str; 5] = [
        "minecraft:air",
        "minecraft:grass_block",
        "minecraft:sand",
        "minecraft:small_dripleaf",
        "minecraft:tall_grass",
    ];
    const DRIPLEAF_LOWER: BlockStateId = BlockStateId(3);
    const DRIPLEAF_UPPER: BlockStateId = BlockStateId(4);
    const TALL_LOWER: BlockStateId = BlockStateId(5);
    const TALL_UPPER: BlockStateId = BlockStateId(6);

    /// The registry of [`HALVES`], each state carrying the `half` its template reads.
    fn halves_registry() -> &'static BlockRegistry {
        let states = HALVES
            .iter()
            .map(|(name, half)| {
                let properties = half.map(|half| {
                    (
                        "properties",
                        Tag::compound([("half", Tag::String(half.into()))]),
                    )
                });
                Tag::compound(
                    [("id", Tag::String((*name).into()))]
                        .into_iter()
                        .chain(properties),
                )
            })
            .collect::<Vec<_>>();
        let defaults = HALVES
            .iter()
            .enumerate()
            .map(|(id, (name, _))| (*name, Tag::Int(id as i32)));
        Box::leak(Box::new(
            BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                ("states", Tag::List(states)),
                ("defaults", Tag::compound(defaults)),
            ]))
            .expect("half plant registry"),
        ))
    }

    /// The table under test for the two half plants: both tags hold grass, and no state of
    /// this registry asks its ground through the behaviour tables, which these two rules do
    /// not read.
    fn halves() -> BlockSurvivalRegistry<'static> {
        let definitions = SurvivalDefinitions {
            rules: BTreeMap::from([
                (
                    "minecraft:small_dripleaf".to_owned(),
                    SurvivalRule::SmallDripleaf,
                ),
                ("minecraft:tall_grass".to_owned(), SurvivalRule::DoublePlant),
            ]),
            ..Default::default()
        };
        let tags = BTreeMap::from([
            (
                "minecraft:supports_small_dripleaf".to_owned(),
                vec!["minecraft:grass_block".to_owned()],
            ),
            (
                "minecraft:supports_vegetation".to_owned(),
                vec!["minecraft:grass_block".to_owned()],
            ),
        ]);
        table(
            halves_registry(),
            &HALF_NAMES,
            &definitions,
            &tags,
            BTreeSet::new(),
        )
    }

    /// A level of named blocks under daylight and no fluids: `Rule::Crop` reads the light
    /// at the plant, which daylight gives it.
    struct TestWorld {
        blocks: BTreeMap<[i32; 3], BlockStateId>,
        light: i32,
    }
    impl TestWorld {
        /// A world with the plant at [`PLANT`] and its ground at [`GROUND`].
        fn new(plant: BlockStateId, ground: BlockStateId) -> Self {
            Self {
                blocks: BTreeMap::from([(PLANT, plant), (GROUND, ground)]),
                light: 15,
            }
        }
        /// The same world under `light` everywhere, as a cave or a lit room has.
        fn with_light(mut self, light: i32) -> Self {
            self.light = light;
            self
        }
    }
    impl DynamicSupportWorld for TestWorld {
        type Error = io::Error;
        fn shulker_is_closed(&mut self, _: [i32; 3]) -> io::Result<Option<bool>> {
            Ok(None)
        }
        fn moving_piston_shape(&mut self, _: [i32; 3]) -> io::Result<Option<VoxelShape>> {
            Ok(None)
        }
    }
    impl DynamicCollisionWorld for TestWorld {
        fn shulker_collision_state(
            &mut self,
            _: [i32; 3],
        ) -> io::Result<Option<ShulkerCollisionState>> {
            Ok(None)
        }
    }
    impl SurvivalWorld for TestWorld {
        fn block_state(&mut self, position: [i32; 3]) -> io::Result<BlockStateId> {
            Ok(*self.blocks.get(&position).unwrap_or(&AIR))
        }
        fn fluid_state(&mut self, _: [i32; 3]) -> io::Result<FluidState> {
            Ok(FluidState::EMPTY)
        }
        fn raw_brightness(&mut self, _: [i32; 3], _: i32) -> io::Result<i32> {
            Ok(self.light)
        }
    }

    /// The table's own answer for the plant at [`PLANT`].
    fn survives(
        survival: &BlockSurvivalRegistry<'_>,
        world: &mut TestWorld,
        plant: BlockStateId,
    ) -> bool {
        survival
            .can_survive(world, plant, PLANT)
            .expect("the table answers for this plant")
    }

    /// What the installed hooks answer while one test asks: the state (and, for
    /// `may_place_on`, the ground state) and the position that answer belongs to. A query
    /// from any other test in this binary — they run on many threads at once — matches no
    /// switch and gets no answer at all.
    #[derive(Clone, Copy)]
    struct Asked {
        state: BlockStateId,
        ground: Option<BlockStateId>,
        at: [i32; 3],
        answer: Option<bool>,
    }
    static SURVIVES: Mutex<Option<Asked>> = Mutex::new(None);
    static PLACE_ON: Mutex<Option<Asked>> = Mutex::new(None);
    static SUSTAINS: Mutex<Option<Asked>> = Mutex::new(None);
    /// How many times a switch was consulted and matched.
    static ASKS: AtomicUsize = AtomicUsize::new(0);
    /// Held by a test that turns a switch on for its whole run, so no two ever do.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// The answer one switch holds for this query, if it is the query it names.
    fn asked(
        switch: &Mutex<Option<Asked>>,
        state: BlockStateId,
        ground: Option<BlockStateId>,
        at: [i32; 3],
    ) -> Option<Option<bool>> {
        let asked = (*switch.lock().unwrap_or_else(PoisonError::into_inner))?;
        if asked.state != state || asked.ground != ground || asked.at != at {
            return None;
        }
        ASKS.fetch_add(1, Ordering::SeqCst);
        Some(asked.answer)
    }
    fn hook_can_survive(state: BlockStateId, position: [i32; 3], survives: &mut Option<bool>) {
        if let Some(answer) = asked(&SURVIVES, state, None, position) {
            *survives = answer;
        }
    }
    fn hook_may_place_on(
        state: BlockStateId,
        ground: BlockStateId,
        ground_position: [i32; 3],
        may: &mut Option<bool>,
    ) {
        if let Some(answer) = asked(&PLACE_ON, state, Some(ground), ground_position) {
            *may = answer;
        }
    }
    fn hook_can_sustain_plant(
        state: BlockStateId,
        ground: BlockStateId,
        ground_position: [i32; 3],
        sustains: &mut Option<bool>,
    ) {
        if let Some(answer) = asked(&SUSTAINS, state, Some(ground), ground_position) {
            *sustains = answer;
        }
    }

    /// Installs the callbacks once for this test binary; every other test's query matches
    /// no switch, so it sees the answers it would see with no hooks installed.
    fn install_hooks() {
        static INSTALL: OnceLock<()> = OnceLock::new();
        INSTALL.get_or_init(|| {
            let installed = native_worldgen_hooks::install(NativeWorldgenHooks {
                alter_ground: |_, _| {},
                on_tree_grow: |_, _| None,
                rotate: |_, _, _, table| table,
                can_survive: hook_can_survive,
                may_place_on: hook_may_place_on,
                can_sustain_plant: hook_can_sustain_plant,
                feature: |place| place(),
            });
            assert!(
                installed.is_ok(),
                "this test installs the worldgen hooks first"
            );
        });
    }

    fn set(switch: &Mutex<Option<Asked>>, answer: Option<Asked>) {
        *switch.lock().unwrap_or_else(PoisonError::into_inner) = answer;
    }

    /// Runs `f` with one switch set, then clears it, so the rest of the binary keeps the
    /// answers it would get without these hooks.
    fn answers<R>(switch: &Mutex<Option<Asked>>, ask: Asked, f: impl FnOnce() -> R) -> R {
        set(switch, Some(ask));
        let result = f();
        set(switch, None);
        result
    }

    /// Runs `f` with the soil's switch and the plant's set together, then clears both: the
    /// pair the table asks in that order.
    fn answers_both<R>(soil: Option<Asked>, plant: Option<Asked>, f: impl FnOnce() -> R) -> R {
        set(&SUSTAINS, soil);
        set(&PLACE_ON, plant);
        let result = f();
        set(&SUSTAINS, None);
        set(&PLACE_ON, None);
        result
    }

    /// The table asks a part's `canSurvive` before its own rule, so the part's answer
    /// replaces it — for a vanilla plant whose rule the table holds, and for a mod block
    /// the table has no rule for at all, which the rule alone reports `Unsupported`.
    #[test]
    fn the_table_asks_a_blocks_can_survive_before_its_rule() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        install_hooks();
        let tags = tags();
        let survival = survival(&tags);

        // The rule alone: short grass stands on grass, not on sand.
        assert!(survives(
            &survival,
            &mut TestWorld::new(SHORT_GRASS, GRASS),
            SHORT_GRASS
        ));
        assert!(!survives(
            &survival,
            &mut TestWorld::new(SHORT_GRASS, SAND),
            SHORT_GRASS
        ));

        // A part's no replaces the rule's yes, and a part's yes its no.
        let refuse = Asked {
            state: SHORT_GRASS,
            ground: None,
            at: PLANT,
            answer: Some(false),
        };
        assert!(!answers(&SURVIVES, refuse, || survives(
            &survival,
            &mut TestWorld::new(SHORT_GRASS, GRASS),
            SHORT_GRASS
        )));
        let allow = Asked {
            state: SHORT_GRASS,
            ground: None,
            at: PLANT,
            answer: Some(true),
        };
        assert!(answers(&SURVIVES, allow, || survives(
            &survival,
            &mut TestWorld::new(SHORT_GRASS, SAND),
            SHORT_GRASS
        )));

        // A block the table holds no rule for is answered by its parts alone: the ask
        // comes before the rule the table does not have.
        let mut world = TestWorld::new(CRYSTAL, SAND);
        assert!(matches!(
            survival.can_survive(&mut world, CRYSTAL, PLANT),
            Err(SurvivalError::Unsupported(state)) if state == CRYSTAL
        ));
        let allow = Asked {
            state: CRYSTAL,
            ground: None,
            at: PLANT,
            answer: Some(true),
        };
        assert!(answers(&SURVIVES, allow, || survives(
            &survival,
            &mut TestWorld::new(CRYSTAL, SAND),
            CRYSTAL
        )));
        let refuse = Asked {
            state: CRYSTAL,
            ground: None,
            at: PLANT,
            answer: Some(false),
        };
        assert!(!answers(&SURVIVES, refuse, || survives(
            &survival,
            &mut TestWorld::new(CRYSTAL, SAND),
            CRYSTAL
        )));

        // A part asked whose answer stays empty leaves the rule's answer, and it was asked.
        let before = ASKS.load(Ordering::SeqCst);
        let silent = Asked {
            state: SHORT_GRASS,
            ground: None,
            at: PLANT,
            answer: None,
        };
        assert!(answers(&SURVIVES, silent, || survives(
            &survival,
            &mut TestWorld::new(SHORT_GRASS, GRASS),
            SHORT_GRASS
        )));
        assert_eq!(
            ASKS.load(Ordering::SeqCst),
            before + 1,
            "the table asked the parts"
        );
    }

    /// The plant's own ground rule asks the parts of the plant's block about the ground
    /// under it, with the ground's own position, and their answer replaces the table's
    /// tag — in the crop rule and in the vegetation one, which the vanilla states that
    /// own those branches read.
    #[test]
    fn the_plant_rule_asks_parts_about_the_ground() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        install_hooks();
        let tags = tags();
        let survival = survival(&tags);

        // The table alone: wheat and short grass stand on their tag's ground, not on sand.
        assert!(survives(
            &survival,
            &mut TestWorld::new(WHEAT, FARMLAND),
            WHEAT
        ));
        assert!(!survives(
            &survival,
            &mut TestWorld::new(WHEAT, SAND),
            WHEAT
        ));
        assert!(!survives(
            &survival,
            &mut TestWorld::new(SHORT_GRASS, SAND),
            SHORT_GRASS
        ));

        // A part's yes on ground outside the tag replaces the crop rule...
        let allow = Asked {
            state: WHEAT,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(true),
        };
        assert!(answers(&PLACE_ON, allow, || survives(
            &survival,
            &mut TestWorld::new(WHEAT, SAND),
            WHEAT
        )));
        // ... and the vegetation rule, which the answer names by the plant it stands for.
        let allow = Asked {
            state: SHORT_GRASS,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(true),
        };
        assert!(answers(&PLACE_ON, allow, || survives(
            &survival,
            &mut TestWorld::new(SHORT_GRASS, SAND),
            SHORT_GRASS
        )));
        // A part's no on ground inside the tag replaces it the other way.
        let refuse = Asked {
            state: WHEAT,
            ground: Some(FARMLAND),
            at: GROUND,
            answer: Some(false),
        };
        assert!(!answers(&PLACE_ON, refuse, || survives(
            &survival,
            &mut TestWorld::new(WHEAT, FARMLAND),
            WHEAT
        )));

        // A part asked whose answer stays empty leaves the tag's answer, and it was asked.
        let before = ASKS.load(Ordering::SeqCst);
        let silent = Asked {
            state: WHEAT,
            ground: Some(SAND),
            at: GROUND,
            answer: None,
        };
        assert!(!answers(&PLACE_ON, silent, || survives(
            &survival,
            &mut TestWorld::new(WHEAT, SAND),
            WHEAT
        )));
        assert_eq!(
            ASKS.load(Ordering::SeqCst),
            before + 1,
            "the table asked the parts"
        );
    }

    /// A block the table has no rule for is answered by the ground question its template's
    /// `canSurvive` would ask, the soil's parts and then the plant's: a mod plant whose ground
    /// rule alone answers no longer gets `Unsupported` (#17943).
    #[test]
    fn a_rule_less_block_asks_its_parts_about_the_ground() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        install_hooks();
        let tags = tags();
        let survival = survival(&tags);

        // Neither the soil nor the plant answers: the table still has no rule for the block.
        let mut world = TestWorld::new(CRYSTAL, SAND);
        assert!(matches!(
            survival.can_survive(&mut world, CRYSTAL, PLANT),
            Err(SurvivalError::Unsupported(state)) if state == CRYSTAL
        ));

        // The plant's own ground rule answers, on the ground under it.
        let allow = Asked {
            state: CRYSTAL,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(true),
        };
        assert!(answers_both(None, Some(allow), || survives(
            &survival,
            &mut TestWorld::new(CRYSTAL, SAND),
            CRYSTAL
        )));
        let refuse = Asked {
            state: CRYSTAL,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(false),
        };
        assert!(!answers_both(None, Some(refuse), || survives(
            &survival,
            &mut TestWorld::new(CRYSTAL, SAND),
            CRYSTAL
        )));

        // So does the soil's, asked first.
        let soil = Asked {
            state: CRYSTAL,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(true),
        };
        assert!(answers_both(Some(soil), None, || survives(
            &survival,
            &mut TestWorld::new(CRYSTAL, SAND),
            CRYSTAL
        )));
    }

    /// The soil's answer comes before the plant's, as the live game's `may_place_on` asks them
    /// (`level/survive.rs`): a soil that refuses wins over a plant that allows, and only a
    /// silent soil leaves the plant's answer.
    #[test]
    fn the_soils_answer_comes_before_the_plants() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        install_hooks();
        let tags = tags();
        let survival = survival(&tags);

        // The table alone: short grass stands on grass, not on sand.
        assert!(survives(
            &survival,
            &mut TestWorld::new(SHORT_GRASS, GRASS),
            SHORT_GRASS
        ));
        assert!(!survives(
            &survival,
            &mut TestWorld::new(SHORT_GRASS, SAND),
            SHORT_GRASS
        ));

        // A soil that refuses and a plant that allows: the soil is the answer.
        let soil_refuses = Asked {
            state: SHORT_GRASS,
            ground: Some(GRASS),
            at: GROUND,
            answer: Some(false),
        };
        let plant_allows = Asked {
            state: SHORT_GRASS,
            ground: Some(GRASS),
            at: GROUND,
            answer: Some(true),
        };
        assert!(!answers_both(
            Some(soil_refuses),
            Some(plant_allows),
            || survives(
                &survival,
                &mut TestWorld::new(SHORT_GRASS, GRASS),
                SHORT_GRASS
            )
        ));

        // A silent soil leaves the plant's yes on ground outside the tag.
        let plant_allows = Asked {
            state: SHORT_GRASS,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(true),
        };
        assert!(answers_both(None, Some(plant_allows), || survives(
            &survival,
            &mut TestWorld::new(SHORT_GRASS, SAND),
            SHORT_GRASS
        )));

        // And a soil that allows wins over a plant that refuses.
        let soil_allows = Asked {
            state: SHORT_GRASS,
            ground: Some(GRASS),
            at: GROUND,
            answer: Some(true),
        };
        let plant_refuses = Asked {
            state: SHORT_GRASS,
            ground: Some(GRASS),
            at: GROUND,
            answer: Some(false),
        };
        assert!(answers_both(Some(soil_allows), Some(plant_refuses), || {
            survives(
                &survival,
                &mut TestWorld::new(SHORT_GRASS, GRASS),
                SHORT_GRASS,
            )
        }));
    }

    /// A mushroom keeps its two shortcuts and asks the ground question only between them: a
    /// ground in the light-override tag holds it without a question, light of 13 and more
    /// refuses it without one, and below 13 the parts' answer replaces the solid-ground rule.
    #[test]
    fn a_mushroom_keeps_its_light_and_tag_shortcuts() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        install_hooks();
        let tags = tags();
        let survival = survival(&tags);

        // A ground in `overrides_mushroom_light_requirement` holds it in full light, and the
        // parts are not asked about a plant that would refuse.
        let refuse = Asked {
            state: MUSHROOM,
            ground: Some(MYCELIUM),
            at: GROUND,
            answer: Some(false),
        };
        let before = ASKS.load(Ordering::SeqCst);
        let mut world = TestWorld::new(MUSHROOM, MYCELIUM);
        assert!(answers_both(None, Some(refuse), || survives(
            &survival, &mut world, MUSHROOM
        )));
        assert_eq!(
            ASKS.load(Ordering::SeqCst),
            before,
            "a light-overriding ground asks no part"
        );

        // Light of 13 and more refuses it, and the parts are not asked about a plant that
        // would allow.
        let allow = Asked {
            state: MUSHROOM,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(true),
        };
        let before = ASKS.load(Ordering::SeqCst);
        assert!(!answers_both(None, Some(allow), || survives(
            &survival,
            &mut TestWorld::new(MUSHROOM, SAND),
            MUSHROOM
        )));
        assert_eq!(
            ASKS.load(Ordering::SeqCst),
            before,
            "full light asks no part"
        );

        // Below 13 the parts answer the ground question instead of the solid-ground rule, on a
        // ground that is not solid at all in this table.
        let before = ASKS.load(Ordering::SeqCst);
        assert!(!survives(
            &survival,
            &mut TestWorld::new(MUSHROOM, SAND).with_light(12),
            MUSHROOM
        ));
        assert_eq!(
            ASKS.load(Ordering::SeqCst),
            before,
            "no part answers, and the table's own rule decides"
        );
        let allow = Asked {
            state: MUSHROOM,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(true),
        };
        assert!(answers_both(None, Some(allow), || survives(
            &survival,
            &mut TestWorld::new(MUSHROOM, SAND).with_light(12),
            MUSHROOM
        )));
        let refuse = Asked {
            state: MUSHROOM,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(false),
        };
        assert!(!answers_both(None, Some(refuse), || survives(
            &survival,
            &mut TestWorld::new(MUSHROOM, SAND).with_light(12),
            MUSHROOM
        )));
    }

    /// A sea pickle asks the parts about its ground before its own support rule: a plant that
    /// refuses is refused on support the table would otherwise hold, and one that allows stands
    /// where the table's empty shapes would not.
    #[test]
    fn a_pickle_asks_the_parts_before_its_support() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        install_hooks();
        let tags = tags();
        let survival = survival(&tags);

        // The table alone: every shape in it is empty, so the pickle stands nowhere.
        assert!(!survives(
            &survival,
            &mut TestWorld::new(SEA_PICKLE, SAND),
            SEA_PICKLE
        ));

        let allow = Asked {
            state: SEA_PICKLE,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(true),
        };
        assert!(answers_both(None, Some(allow), || survives(
            &survival,
            &mut TestWorld::new(SEA_PICKLE, SAND),
            SEA_PICKLE
        )));
        let refuse = Asked {
            state: SEA_PICKLE,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(false),
        };
        assert!(!answers_both(None, Some(refuse), || survives(
            &survival,
            &mut TestWorld::new(SEA_PICKLE, SAND),
            SEA_PICKLE
        )));
        // The soil's answer is asked first here too.
        let soil = Asked {
            state: SEA_PICKLE,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(true),
        };
        assert!(answers_both(Some(soil), None, || survives(
            &survival,
            &mut TestWorld::new(SEA_PICKLE, SAND),
            SEA_PICKLE
        )));
    }

    /// A block whose template asks no ground reaches no ground part even with no rule in the
    /// table: the mod block the table does not cover is unsupported whatever stands under it,
    /// as a modded torch must be (#17943).
    #[test]
    fn a_rule_less_block_whose_template_asks_no_ground_asks_no_part() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        install_hooks();
        let tags = tags();
        let survival = survival(&tags);

        let allow = Asked {
            state: LANTERN,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(true),
        };
        let before = ASKS.load(Ordering::SeqCst);
        let mut world = TestWorld::new(LANTERN, SAND);
        assert!(matches!(
            answers_both(Some(allow), Some(allow), || survival.can_survive(
                &mut world,
                LANTERN,
                PLANT
            )),
            Err(SurvivalError::Unsupported(state)) if state == LANTERN
        ));
        assert_eq!(
            ASKS.load(Ordering::SeqCst),
            before,
            "a template that asks no ground asks no part"
        );
    }

    /// Soul fire shares the vegetation rule but asks no ground: the tag the table holds for
    /// it decides alone, while a plant of the same rule still asks its parts (#17943).
    #[test]
    fn soul_fire_keeps_its_tag_where_a_plant_of_its_rule_asks_the_ground() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        install_hooks();
        let tags = tags();
        let survival = survival(&tags);

        // The table alone: soul fire stands on its base blocks, not on grass.
        assert!(survives(
            &survival,
            &mut TestWorld::new(SOUL_FIRE, SAND),
            SOUL_FIRE
        ));
        assert!(!survives(
            &survival,
            &mut TestWorld::new(SOUL_FIRE, GRASS),
            SOUL_FIRE
        ));

        // The parts refuse, and neither is asked: the tag answers.
        let refuse = Asked {
            state: SOUL_FIRE,
            ground: Some(SAND),
            at: GROUND,
            answer: Some(false),
        };
        let before = ASKS.load(Ordering::SeqCst);
        assert!(answers_both(Some(refuse), Some(refuse), || survives(
            &survival,
            &mut TestWorld::new(SOUL_FIRE, SAND),
            SOUL_FIRE
        )));
        assert_eq!(
            ASKS.load(Ordering::SeqCst),
            before,
            "soul fire asks no ground part"
        );

        // Short grass, the same rule in a template that asks its ground, is refused by a
        // part that refuses ground the tag holds.
        let refuse = Asked {
            state: SHORT_GRASS,
            ground: Some(GRASS),
            at: GROUND,
            answer: Some(false),
        };
        assert!(!answers(&PLACE_ON, refuse, || survives(
            &survival,
            &mut TestWorld::new(SHORT_GRASS, GRASS),
            SHORT_GRASS
        )));
    }

    /// The lower halves of small dripleaf and of a double plant ask the ground their templates
    /// ask before the tags the table holds for them decide; the upper halves keep their check
    /// on the half below and ask nothing, as the live game has them (`level/survive.rs`).
    #[test]
    fn the_lower_halves_of_the_half_plants_ask_the_ground_before_their_tags() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        install_hooks();
        let survival = halves();
        let lower = [DRIPLEAF_LOWER, TALL_LOWER];

        // The table alone: the lower halves stand on the grass both tags hold, not on sand.
        for plant in lower {
            assert!(survives(
                &survival,
                &mut TestWorld::new(plant, GRASS),
                plant
            ));
            assert!(!survives(
                &survival,
                &mut TestWorld::new(plant, SAND),
                plant
            ));
        }

        // A ground part's yes stands either of them where no tag holds the ground...
        for plant in lower {
            let allow = Asked {
                state: plant,
                ground: Some(SAND),
                at: GROUND,
                answer: Some(true),
            };
            assert!(answers(&PLACE_ON, allow, || survives(
                &survival,
                &mut TestWorld::new(plant, SAND),
                plant
            )));
        }
        // ... its no refuses them ground the tags hold ...
        for plant in lower {
            let refuse = Asked {
                state: plant,
                ground: Some(GRASS),
                at: GROUND,
                answer: Some(false),
            };
            assert!(!answers(&PLACE_ON, refuse, || survives(
                &survival,
                &mut TestWorld::new(plant, GRASS),
                plant
            )));
        }
        // ... and the soil's answer is asked first here too.
        for plant in lower {
            let soil = Asked {
                state: plant,
                ground: Some(SAND),
                at: GROUND,
                answer: Some(true),
            };
            assert!(answers_both(Some(soil), None, || survives(
                &survival,
                &mut TestWorld::new(plant, SAND),
                plant
            )));
        }

        // The upper halves stand on their lower half and ask no part, which refuses here.
        let before = ASKS.load(Ordering::SeqCst);
        let refuse = Asked {
            state: TALL_UPPER,
            ground: Some(TALL_LOWER),
            at: GROUND,
            answer: Some(false),
        };
        assert!(answers_both(Some(refuse), Some(refuse), || survives(
            &survival,
            &mut TestWorld::new(TALL_UPPER, TALL_LOWER),
            TALL_UPPER
        )));
        let refuse = Asked {
            state: DRIPLEAF_UPPER,
            ground: Some(DRIPLEAF_LOWER),
            at: GROUND,
            answer: Some(false),
        };
        assert!(answers(&PLACE_ON, refuse, || survives(
            &survival,
            &mut TestWorld::new(DRIPLEAF_UPPER, DRIPLEAF_LOWER),
            DRIPLEAF_UPPER
        )));
        assert_eq!(
            ASKS.load(Ordering::SeqCst),
            before,
            "the upper halves ask no part"
        );
    }
}
